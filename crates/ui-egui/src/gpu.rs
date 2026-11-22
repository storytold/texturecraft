//! Bridge between the session and the wgpu renderer: per-frame scene sync (mesh, texture set
//! layout, dirty tiles, environment) and the egui paint callback that renders and blits a view.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use rayon::prelude::*;
use texturecraft_engine::Session;
use texturecraft_engine::compose::{DisplayKind, ImageRes, display_tile_bytes, kinds_for};
use texturecraft_engine::core::{BakedMap, Channel, LayerId};
use texturecraft_engine::geom::{Mesh, glam::Vec2};
use texturecraft_engine::session::ReleasedDab;
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_engine::{
    StackTarget, ViewMode, brush_device,
    compose::PaintTarget,
    paint::{Stamp, UvStamp},
    texel::TileImage,
};
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_render::gpu_composite::GRAPH_OUTPUT_PENDING;
use texturecraft_render::gpu_composite::{CompositeGraphs, CompositeProgram, CompositeRegion, CompositeTiming};
use texturecraft_render::{BakeVisualization, CageGeometry, Frame, Renderer, wgpu};
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_render::{
    gpu_brush::{BrushInput, GpuBrushStroke},
    gpu_composite::{LiveBrush, LiveBrushTarget},
};

use texturecraft_render::gpu_quick_mask::{QuickMaskAlpha, QuickMaskOverlay};

use crate::bake_scene::{BakeSceneLink, CageUpload};
use crate::i18n::n_;

/// Content shown through the aux texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AuxView {
    MeshMap(BakedMap),
    Mask(LayerId),
    /// The quick mask being edited, over the base color.
    QuickMask,
    /// The project image of the view picker's "Other..." (`Session::view_image`)
    Image,
}

/// A texture set's texels across and down, and its mesh material.
pub type SetLayout = ((u32, u32), usize);

/// A viewport composite belongs to one document state and display size.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CompositeKey {
    mesh: u64,
    revision: u64,
    size: (u32, u32),
    shift: u32,
}

/// The low poly's hard edges without a UV seam (vertex pairs) per material
pub type MissingSeamEdges = Vec<Vec<(u32, u32)>>;

/// What the renderer must catch up with before drawing.
#[derive(Default)]
pub struct SceneSync {
    pub mesh: Option<(Arc<Mesh>, u64)>,
    mesh_id: u64,
    /// (texels across and down, material) per texture set, the mesh material count and the
    /// viewport texture resolution (display shift).
    pub sets: Option<(Vec<SetLayout>, usize, u32)>,
    /// The environment name and, for `resource:<name>`, its decoded image.
    pub env: Option<(String, Option<Arc<ImageRes>>)>,
    /// Reduction applied when preparing each display tile.
    pub tile_shift: u32,
    pub overview: Option<(usize, texturecraft_engine::MaterialOverview)>,
    cleared_overviews: Vec<usize>,
    cpu_texture_sets: Vec<usize>,
    resized_sets: Vec<(usize, Arc<AtomicBool>)>,
    ready_cpu_sets: Vec<usize>,
    composites: Vec<(usize, CompositeProgram, Arc<AtomicBool>)>,
    composite_work: Option<CompositeWork>,
    pub(crate) compositor_changed: bool,
    pub tiles: Vec<(usize, DisplayKind, u32, u32, Vec<u8>)>,
    released_dabs: Vec<ReleasedDab>,
    pub aux: Vec<(usize, u32, Vec<u8>)>,
    quick_masks: Vec<(usize, QuickMaskOverlay)>,
    quick_mask_failed: Option<Arc<AtomicBool>>,
    release_quick_mask: bool,
    /// One image buffer per resolution, with all texture sets that need it.
    pub image_aux: BTreeMap<u32, (Vec<usize>, Vec<u8>)>,
    /// Bake mode's high poly (`None` removes it) and the id it is sent under
    pub bake_high: Option<(Option<Arc<Mesh>>, u64)>,
    /// Which vertices of the high poly (by its id) the bake's rays miss (1) or reach (0)
    pub bake_high_missed: Option<(u64, Vec<f32>)>,
    /// Changed cages: texture set, the cage or `None` to remove it, its topology
    pub bake_cages: Vec<(usize, Option<CageUpload>, u64)>,
    /// Texture sets left after some were removed: their cages go too
    pub bake_cage_count: Option<usize>,
    /// The low poly's hard edges without a UV seam per material, for mesh id
    pub missing_seams: Option<(u64, MissingSeamEdges)>,
}

/// Counts a frame's queued composite until submission completes or the frame is dropped.
struct CompositeWork {
    in_flight: Arc<AtomicUsize>,
    started: Option<f64>,
}

impl Drop for CompositeWork {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

impl SceneSync {
    pub(crate) fn import_preview(preview: &texturecraft_engine::import_cmds::ImportPreview) -> Self {
        let mesh_id = preview.mesh_id;
        let sets = preview
            .materials
            .iter()
            .enumerate()
            .map(|(set, &material)| {
                let size = if set == preview.texture_set { preview.overview.size } else { (1, 1) };
                (size, material)
            })
            .collect();
        Self {
            mesh: Some((preview.mesh.clone(), mesh_id)),
            mesh_id,
            sets: Some((sets, preview.mesh.materials.len().max(1), 0)),
            env: Some((texturecraft_engine::core::DisplaySettings::default().environment, None)),
            overview: Some((preview.texture_set, preview.overview.clone())),
            release_quick_mask: true,
            ..Self::default()
        }
    }

    fn changes_base_color(&self, ts: usize) -> bool {
        self.sets.is_some()
            || self.cleared_overviews.contains(&ts)
            || self.overview.as_ref().is_some_and(|(set, _)| *set == ts)
            || self.composites.iter().any(|(set, program, _)| *set == ts && !program.is_mask_preview())
            || self.tiles.iter().any(|(set, kind, ..)| *set == ts && *kind == DisplayKind::BaseColor)
    }

    #[cfg(test)]
    fn apply_display(&self, renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) -> (Vec<usize>, Vec<(usize, String)>) {
        self.apply_display_reporting(renderer, device, queue, None)
    }

    fn apply_display_reporting(
        &self,
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        reports: Option<&Mutex<GpuReports>>,
    ) -> (Vec<usize>, Vec<(usize, String)>) {
        let mut reset = Vec::new();
        if let Some((sets, material_count, shift)) = &self.sets {
            let resizing: Vec<_> = self.resized_sets.iter().filter_map(|(ts, staged)| (!staged.load(Ordering::Acquire)).then_some(*ts)).collect();
            renderer.set_display_shift(*shift);
            reset = renderer.set_texture_sets_preserving(device, queue, sets, *material_count, &resizing);
            for (_, staged) in &self.resized_sets {
                staged.store(true, Ordering::Release);
            }
        }
        for &ts in &self.cleared_overviews {
            renderer.seed_display(device, queue, ts, (1, 1), &[]);
            reset.push(ts);
        }
        for &ts in &self.cpu_texture_sets {
            renderer.release_composite_inputs(ts);
        }
        let mut errors = Vec::new();
        for (ts, program, failed) in &self.composites {
            let mut timing = (reports.is_some() && !program.is_neutral() && !program.is_mask_preview()).then(CompositeTiming::default);
            if let Err(error) = renderer.composite_on_gpu_timed(device, queue, *ts, program, timing.as_mut()) {
                failed.store(true, Ordering::Release);
                errors.push((*ts, error));
            } else if let (Some(timing), Some(reports)) = (timing, reports) {
                reports
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .performance
                    .push((self.mesh_id, format!("GPU material preparation for texture set {}: {timing}", ts + 1)));
            }
        }
        if let Some((ts, overview)) = &self.overview {
            renderer.seed_display(device, queue, *ts, overview.size, &overview.maps);
        }
        for (texture_set, kind, tile_x, tile_y, bytes) in &self.tiles {
            renderer.upload_display_tile(device, queue, *texture_set, *kind, *tile_x, *tile_y, bytes, self.tile_shift);
        }
        for &ts in &self.ready_cpu_sets {
            renderer.finish_texture_set_resize(ts);
        }
        if self.release_quick_mask {
            renderer.release_quick_mask_inputs();
        }
        for (ts, overlay) in &self.quick_masks {
            if let Err(err) = renderer.draw_quick_mask(device, queue, *ts, overlay) {
                if let Some(failed) = &self.quick_mask_failed {
                    failed.store(true, Ordering::Release);
                }
                errors.push((*ts, err));
                break;
            }
        }
        for dab in &self.released_dabs {
            dab.acknowledge_upload();
        }
        (reset, errors)
    }
}

/// Lives in egui's callback resources.
pub struct GpuState {
    pub renderer: Renderer,
    /// Recreated sets and the display kinds uploaded since recreation, for [`GpuLink`].
    pub recreated: Arc<Mutex<BTreeMap<usize, BTreeSet<DisplayKind>>>>,
    pub reports: Arc<Mutex<GpuReports>>,
    /// Wakes the UI when the GPU side has something to report.
    pub ctx: egui::Context,
}

#[cfg(not(target_arch = "wasm32"))]
struct BrushDevice(egui_wgpu::RenderState);

#[cfg(not(target_arch = "wasm32"))]
impl brush_device::BrushDevice for BrushDevice {
    fn start(&self, surface: brush_device::BrushSurface) -> Result<Box<dyn brush_device::DeviceStroke>, String> {
        let mut renderer = self.0.renderer.write();
        let gpu = renderer.callback_resources.get_mut::<GpuState>().ok_or("GPU renderer unavailable")?;
        DeviceStroke::start(&mut gpu.renderer, &self.0.device, &self.0.queue, surface)
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct DeviceStroke(GpuBrushStroke);

#[cfg(not(target_arch = "wasm32"))]
impl DeviceStroke {
    fn start(
        renderer: &mut Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: brush_device::BrushSurface,
    ) -> Result<Box<dyn brush_device::DeviceStroke>, String> {
        let brush_device::BrushSurface { mesh, bvh, uv, material, depth, settings, alpha_image, stencil, limit } = surface;
        let stroke = renderer.start_gpu_brush(device, queue, BrushInput { mesh, bvh, uv, material, depth, settings, alpha_image, stencil, limit })?;
        Ok(Box::new(Self(stroke)))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl brush_device::DeviceStroke for DeviceStroke {
    fn append_world(&mut self, stamps: &[Stamp]) -> Result<brush_device::BrushTiles, String> {
        self.0.append_world(stamps)
    }

    fn append_uv(&mut self, stamps: &[UvStamp]) -> Result<brush_device::BrushTiles, String> {
        self.0.append_uv(stamps)
    }

    fn read_alpha(&self, tiles: &brush_device::BrushTiles) -> Result<TileImage, String> {
        self.0.read_alpha(tiles)
    }

    fn read_projection(&self, tiles: &brush_device::BrushTiles) -> Result<brush_device::ProjectedTiles, String> {
        self.0.read_projection(tiles)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        &self.0
    }
}

/// Problems the GPU side hands to the UI, which shows them as toasts.
#[derive(Default)]
pub struct GpuReports {
    /// Texture sets whose display textures could not be allocated (out of GPU memory).
    pub memory: Vec<usize>,
    /// wgpu errors no error scope caught (logged instead of wgpu's default panic).
    pub errors: Vec<String>,
    pub performance: Vec<(u64, String)>,
}

/// Most distinct uncaptured GPU errors kept until the UI shows them.
const MAX_GPU_ERRORS: usize = 8;
const GPU_COMPOSITE_ERROR: &str = n_("GPU compositing reported a device error. See the app log for details.");

#[cfg(not(target_arch = "wasm32"))]
struct GraphPreparation {
    key: CompositeKey,
    cancel: texturecraft_engine::compose::graphs::Cancel,
    worker: std::thread::JoinHandle<texturecraft_engine::compose::GraphStore>,
}

#[cfg(not(target_arch = "wasm32"))]
impl GraphPreparation {
    fn start(session: &Session, ts: usize, key: CompositeKey) -> Result<Self, &'static str> {
        use texturecraft_engine::compose::{Inputs, graphs::Cancel};

        let cancel = Cancel::new();
        let graphs = session.doc.graphs.for_resize(cancel.clone(), ts);
        let project = session.doc.project.clone();
        let gbuf = Arc::clone(&session.caches[ts].gbuf);
        let mesh = session.mesh.clone();
        let pixels = session.doc.pixels.clone();
        let baked = session.doc.baked.get(&ts).cloned().unwrap_or_default();
        let images = session.images.clone();
        let warm_saved_uses = !session.modified;
        let worker = std::thread::Builder::new()
            .name("viewport graph preparation".to_owned())
            .spawn(move || {
                if let Some(set) = project.texture_sets.get(ts) {
                    let inputs = Inputs {
                        project: &project,
                        ts,
                        mesh: &mesh,
                        gbuf: &gbuf,
                        pixels: &pixels,
                        baked: &baked,
                        block_scales: set.block_scales(),
                        preview: None,
                        images: &images,
                        graphs: &graphs,
                        bounds: mesh.bounds(),
                    };
                    if warm_saved_uses {
                        graphs.warm_saved_uses(ts);
                    }
                    if !warm_saved_uses || CompositeProgram::try_compile_saved_inputs(&inputs, None).is_err() {
                        texturecraft_engine::compose::eval::prepare_texture_set_graphs(&inputs);
                    }
                }
                graphs
            })
            .map_err(|_| n_("Could not start graph preparation."))?;
        Ok(Self { key, cancel, worker })
    }
}

#[derive(Default)]
struct SetCompositor {
    current: Option<CompositeKey>,
    stroke: Option<u64>,
    rejected: Option<(CompositeKey, &'static str)>,
    failed_key: Option<CompositeKey>,
    failed: Arc<AtomicBool>,
    reason: &'static str,
    neutral: bool,
    logged_status: Option<(&'static str, &'static str)>,
    graph_key: Option<CompositeKey>,
    #[cfg(not(target_arch = "wasm32"))]
    graph_job: Option<GraphPreparation>,
}

impl SetCompositor {
    fn uses_gpu(&self) -> bool {
        self.current.is_some() && !self.failed.load(Ordering::Acquire)
    }

    fn reset_view(&mut self) {
        let graph_key = self.graph_key.take();
        #[cfg(not(target_arch = "wasm32"))]
        let graph_job = self.graph_job.take();
        *self = Self::default();
        self.graph_key = graph_key;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.graph_job = graph_job;
        }
    }

    fn graph_pending(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.graph_job.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_graphs(&mut self, session: &Session, ts: usize, key: CompositeKey) -> Result<bool, &'static str> {
        let Some(job) = &self.graph_job else { return Ok(false) };
        if job.key != key {
            job.cancel.cancel();
        }
        if !job.worker.is_finished() {
            return Ok(true);
        }
        let Some(job) = self.graph_job.take() else { return Ok(false) };
        let result = job.worker.join().map_err(|_| n_("Graph preparation stopped unexpectedly."))?;
        if job.key == key {
            session.doc.graphs.accept_resize(&result, ts);
            self.graph_key = Some(key);
            // A recomputed graph may change even pixels far outside a brush dab.
            self.stroke = None;
        }
        Ok(false)
    }

    fn plan(
        &mut self,
        session: &Session,
        ts: usize,
        shift: u32,
        global_reason: &'static str,
        mask: Option<LayerId>,
    ) -> Result<Option<CompositeProgram>, &'static str> {
        let cache = session.caches.get(ts).ok_or(n_("The texture set is missing."))?;
        let key = CompositeKey { mesh: session.mesh_id, revision: cache.material_revision, size: cache.size, shift };
        let graph_key = CompositeKey { shift: 0, ..key };
        if self.failed.load(Ordering::Acquire) {
            self.failed_key = self.current;
            self.failed = Arc::default();
            self.stroke = None;
        }
        if !global_reason.is_empty() {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(job) = self.graph_job.take() {
                job.cancel.cancel();
            }
            return Err(global_reason);
        }
        if self.failed_key == Some(key) {
            return Err(GPU_COMPOSITE_ERROR);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.poll_graphs(session, ts, graph_key)? {
            return Ok(None);
        }
        let edits_view = !session.quick_mask.editing && mask.is_none_or(|id| session.stroke_mask_owner() == Some(session.doc.project.mask_owner(id)));
        let stroke = session.pending_display_stroke(ts).filter(|_| edits_view);
        if edits_view && session.stroke_texture_set() == Some(ts) && stroke.is_none() {
            return Err(n_("This painting tool uses CPU compositing."));
        }

        if mask.is_none() && cache.covered_tiles.is_empty() {
            return Err(n_("The texture set has no covered UV texels."));
        }
        let stroke_id = stroke.map(|(id, _)| id);
        let current = self.current == Some(key);
        let unchanged_stroke = self.stroke == stroke_id;
        if current && unchanged_stroke && stroke.is_none_or(|(_, tiles)| tiles.is_empty()) {
            return Ok(None);
        }
        if stroke.is_none()
            && let Some((_, reason)) = self.rejected.filter(|(rejected, _)| *rejected == key)
        {
            return Err(reason);
        }
        let changed_tiles = if current
            && (unchanged_stroke || self.stroke.is_none())
            && let Some((_, tiles)) = stroke
        {
            Some(tiles)
        } else if stroke.is_none()
            && let Some(previous_id) = self.stroke
            && let Some(previous) = self.current.filter(|old| old.mesh == key.mesh && old.size == key.size && old.shift == key.shift)
        {
            session.finished_display_stroke(ts, previous_id, previous.revision)
        } else {
            None
        };
        let pad = mask.is_none() && !cache.dilation.fully_covered();
        let region = changed_tiles
            .and_then(|tiles| CompositeRegion::from_tiles(tiles.iter().copied()))
            .map(|r| if pad { r.expanded(texturecraft_engine::texel::TILE) } else { r });
        if changed_tiles.is_some() && region.is_none() {
            self.current = Some(key);
            self.stroke = stroke_id;
            return Ok(None);
        }
        let preview = stroke.and_then(|_| {
            // A GPU neighbourhood stage can read earlier, still uncommitted CPU dabs well
            // outside the changed output. Keep all sparse stroke tiles available in that case.
            let project = &session.doc.project;
            let anchors = project
                .texture_sets
                .get(ts)
                .is_some_and(|set| project.render_tree(&set.stack).iter().any(|layer| project.layers.get(layer).is_some_and(|layer| layer.reads_anchor())));
            let bounds = if session.device_stroke().is_none() && (anchors || texturecraft_engine::compose::stack_reads_neighbours(project, ts)) {
                None
            } else {
                region.map(|r| r.expanded(texturecraft_engine::texel::TILE).bounds())
            };
            session.display_stroke_pixels(ts, bounds)
        });
        let pixels = preview.as_ref().unwrap_or(&session.doc.pixels);
        #[cfg(not(target_arch = "wasm32"))]
        let brush = if let Some(brush) = session.device_stroke().filter(|brush| brush.ts == ts && stroke.is_some()) {
            let coverage = brush.stroke.as_any().downcast_ref::<GpuBrushStroke>().ok_or(n_("This painting tool uses CPU compositing."))?;
            let target = match brush.target {
                PaintTarget::Layer(layer) => LiveBrushTarget::Layer { layer, values: brush.values.clone() },
                PaintTarget::Mask(effect) => LiveBrushTarget::Mask { effect, value: brush.mask_value },
            };
            Some(LiveBrush { target, opacity: brush.opacity, alpha: coverage.view(), stencil: coverage.stencil() })
        } else {
            None
        };
        #[cfg(target_arch = "wasm32")]
        let brush = None;
        let empty = Default::default();
        let baked = session.doc.baked.get(&ts).unwrap_or(&empty);
        let mut graphs = CompositeGraphs::new(&session.doc.graphs, baked, &session.images, cache.size).resident_only();
        if self.graph_key.is_some_and(|previous| previous != graph_key && previous.mesh == graph_key.mesh && previous.size == graph_key.size) {
            graphs = graphs.without_stack_results();
        }
        if !session.modified && self.graph_key.is_none_or(|previous| previous == graph_key) {
            graphs = graphs.saved_snapshot();
        }
        let graphs = Some(graphs);
        let program = match mask {
            Some(owner) => CompositeProgram::try_compile_mask_preview_with_graphs(&session.doc.project, pixels, ts, owner, region, brush, graphs),
            None => CompositeProgram::try_compile_with_graphs(&session.doc.project, pixels, ts, region, brush, graphs),
        };
        let mut program = match program {
            Ok(program) => program,
            #[cfg(not(target_arch = "wasm32"))]
            Err(GRAPH_OUTPUT_PENDING) if self.graph_key != Some(graph_key) => {
                self.graph_job = Some(GraphPreparation::start(session, ts, graph_key)?);
                return Ok(None);
            }
            Err(reason) => {
                if stroke_id.is_none() {
                    self.rejected = Some((key, reason));
                }
                return Err(reason);
            }
        };
        if pad && let Some(padding) = cache.dilation.mixed_coverage() {
            program.set_coverage(padding, texturecraft_engine::session::DISPLAY_PADDING);
        }
        if let Err(reason) = program.set_surface_inputs(&session.mesh, &cache.gbuf, session.doc.baked.get(&ts)) {
            if stroke_id.is_none() {
                self.rejected = Some((key, reason));
            }
            return Err(reason);
        }
        program.set_tile_images(&session.images);
        self.neutral = program.is_neutral();
        if !current {
            self.failed = Arc::default();
        }
        self.current = Some(key);
        self.graph_key = Some(graph_key);
        self.stroke = stroke_id;
        Ok(Some(program))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for SetCompositor {
    fn drop(&mut self) {
        if let Some(job) = &self.graph_job {
            job.cancel.cancel();
        }
    }
}

struct MaskPreview {
    ts: usize,
    layer: LayerId,
    compositor: SetCompositor,
}

/// UI-side bookkeeping of what the GPU already has.
pub struct GpuLink {
    pub reset: Arc<AtomicBool>,
    /// [`GpuState::recreated`].
    recreated: Arc<Mutex<BTreeMap<usize, BTreeSet<DisplayKind>>>>,
    pub reports: Arc<Mutex<GpuReports>>,
    /// Raised when a view callback was dropped before the GPU applied its scene sync (the
    /// frame was never painted, e.g. a WebGPU surface not ready yet): send everything again.
    pub lost: Arc<AtomicBool>,
    mesh_id: u64,
    sets: Vec<SetLayout>,
    /// The display shift the GPU has (`Prefs::viewport_texture`).
    shift: u32,
    /// The environment the GPU has: name and image identity (a re-imported resource differs).
    env: (String, Option<Weak<ImageRes>>),
    /// What each set's aux texture holds: (view key, revision).
    aux: BTreeMap<usize, (String, u64)>,
    /// Image identity without holding its decoded pixels alive.
    aux_image: Option<Weak<ImageRes>>,
    /// The display kinds sent to the GPU per set: kinds the set has no channels for are not
    /// converted, unless an earlier upload gave them content that must be overwritten. Only the
    /// renderer recreating a set's textures ([`GpuState::recreated`]) replaces its entry with
    /// kinds uploaded since recreation. A set that keeps its size retains its textures.
    sent: HashMap<usize, BTreeSet<DisplayKind>>,
    pub adapter: String,
    /// Most tiles waiting to be composited since compositing last caught up (progress).
    composite_total: usize,
    /// Next set to serve when the last frame exhausted the shared upload budget.
    upload_cursor: Option<usize>,
    pending_overviews: BTreeSet<usize>,
    active_overviews: BTreeSet<usize>,
    resized_sets: BTreeMap<usize, (u64, Arc<AtomicBool>)>,
    gpu_composite_enabled: bool,
    #[cfg(not(target_arch = "wasm32"))]
    brush_device: Option<Arc<dyn brush_device::BrushDevice>>,
    pub(crate) prefer_gpu_layers: bool,
    pub(crate) material_start_preview: bool,
    compositors: Vec<SetCompositor>,
    mask_preview: Option<MaskPreview>,
    quick_mask_failed: Arc<AtomicBool>,
    gpu_composite_in_flight: Arc<AtomicUsize>,
    cpu_composite_started: Option<f64>,
    cpu_padding_pending: bool,
    bake_scene: BakeSceneLink,
}

/// Compositing time per frame: a full recomposite of a large texture set spreads over frames.
const COMPOSITE_BUDGET_MS: f64 = 30.0;
/// The most a frame spends on the scene (with the work already done this frame, such as a
/// stroke's commit): compositing and display conversion leave the rest for the next frames, so
/// a large (8K) texture set never stalls one frame.
const FRAME_BUDGET_MS: f64 = 35.0;
/// Each frame still makes this much progress.
const MIN_SLICE_MS: f64 = 4.0;
/// Display tiles converted between budget checks.
const UPLOAD_CHUNK: usize = 64;
/// Estimated GPU upload cost of one display tile (write_texture of one 64×64 tile of one
/// display kind, measured at ~5.5 µs on a desktop GPU), counted against the frame budget.
const UPLOAD_MS_PER_TILE: f64 = 0.006;

pub fn init(rs: &egui_wgpu::RenderState, ctx: &egui::Context) -> GpuLink {
    let reset = Arc::new(AtomicBool::new(false));
    let recreated = Arc::new(Mutex::new(BTreeMap::new()));
    let reports = Arc::new(Mutex::new(GpuReports::default()));

    // Report uncaptured wgpu errors to preserve the session when texture allocation or the driver fails
    let (sink, wake) = (reports.clone(), ctx.clone());
    rs.device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        log::error!("GPU error: {e}");
        let mut r = sink.lock().unwrap_or_else(PoisonError::into_inner);
        let text = e.to_string();
        if r.errors.len() < MAX_GPU_ERRORS && !r.errors.contains(&text) {
            r.errors.push(text);
        }

        // The UI may be idle: draw a frame to show it
        wake.request_repaint();
    }));
    let renderer = Renderer::new(&rs.device, &rs.queue, rs.target_format);
    rs.renderer.write().callback_resources.insert(GpuState { renderer, recreated: recreated.clone(), reports: reports.clone(), ctx: ctx.clone() });
    let info = rs.adapter.get_info();
    GpuLink {
        reset,
        recreated,
        reports,
        lost: Arc::new(AtomicBool::new(false)),
        mesh_id: 0,
        sets: Vec::new(),
        shift: 0,
        env: Default::default(),
        aux: BTreeMap::new(),
        aux_image: None,
        sent: HashMap::new(),
        adapter: format!("{} ({:?})", info.name, info.backend),
        composite_total: 0,
        upload_cursor: None,
        pending_overviews: BTreeSet::new(),
        active_overviews: BTreeSet::new(),
        resized_sets: BTreeMap::new(),
        gpu_composite_enabled: !cfg!(target_arch = "wasm32")
            && info.device_type != wgpu::DeviceType::Cpu
            && info.backend != wgpu::Backend::Gl
            && std::env::var("TEXTURECRAFT_GPU_COMPOSITE").as_deref() != Ok("0"),
        #[cfg(not(target_arch = "wasm32"))]
        brush_device: Some(Arc::new(BrushDevice(rs.clone()))),
        prefer_gpu_layers: true,
        material_start_preview: true,
        compositors: Vec::new(),
        mask_preview: None,
        quick_mask_failed: Arc::default(),
        gpu_composite_in_flight: Arc::default(),
        cpu_composite_started: None,
        cpu_padding_pending: false,
        bake_scene: BakeSceneLink::default(),
    }
}

impl GpuLink {
    /// Keep rendering the last complete view until this resize's GPU textures are ready.
    pub(crate) fn replace_resized_view(&mut self, project: u64, ts: usize) {
        self.resized_sets.insert(ts, (project, Arc::default()));
    }
    pub(crate) fn supports_gpu_layers(&self) -> bool {
        self.gpu_composite_enabled
    }

    /// Leave imported sets with a valid GPU plan for the viewport's first material update.
    pub(crate) fn plan_import_set(session: &Session, ts: usize) -> Result<(), &'static str> {
        let project = &session.doc.project;
        let cache = session.caches.get(ts).filter(|cache| !cache.covered_tiles.is_empty()).ok_or("The texture set has no covered tiles.")?;
        let empty = Default::default();
        let baked = session.doc.baked.get(&ts).unwrap_or(&empty);
        let graphs = CompositeGraphs::new(&session.doc.graphs, baked, &session.images, cache.size);
        CompositeProgram::try_compile_with_graphs(project, &session.doc.pixels, ts, None, None, Some(graphs))
            .and_then(|mut program| program.set_baked_inputs(Some(baked)))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn brush_device(&self, session: &Session) -> Option<brush_device::BrushBackend> {
        let ts = session.ts();
        if !self.gpu_composite_enabled || !self.prefer_gpu_layers {
            return None;
        }
        let cache = session.caches.get(ts)?;
        let backend = || Some(brush_device::BrushBackend { device: self.brush_device.clone()?, surface: Arc::downgrade(&cache.gbuf) });
        if session.quick_mask.editing {
            return (!self.quick_mask_failed.load(Ordering::Acquire)).then(backend).flatten();
        }
        let state = match session.view {
            ViewMode::Mask(layer) => {
                let preview = self.mask_preview.as_ref().filter(|preview| preview.ts == ts && preview.layer == layer)?;
                let project = &session.doc.project;
                let owner = project.mask_owner(layer);
                let mask_selected = match session.target {
                    StackTarget::Mask => session.selected_layer().is_some_and(|selected| project.mask_owner(selected) == owner),
                    StackTarget::Effect(effect) => {
                        project.layers.get(&owner).and_then(|layer| layer.mask.as_ref()).is_some_and(|mask| mask.effects.iter().any(|entry| entry.id == effect))
                    }
                    StackTarget::Content => false,
                };
                if !mask_selected {
                    return None;
                }
                &preview.compositor
            }
            _ => self.compositors.get(ts)?,
        };
        let key = CompositeKey { mesh: session.mesh_id, revision: cache.material_revision, size: cache.size, shift: self.shift };
        if !state.uses_gpu() || state.current != Some(key) {
            return None;
        }
        backend()
    }

    /// What the GPU side reported since the last call (see [`GpuReports`]).
    pub fn take_reports(&self) -> GpuReports {
        std::mem::take(&mut *self.reports.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Whether bake mode's visualization is still being worked out off the UI thread
    pub fn bake_busy(&self) -> bool {
        self.bake_scene.busy()
    }

    fn cpu_sets<'a>(&'a self, session: &'a Session) -> impl Iterator<Item = &'a texturecraft_engine::session::SetCache> {
        let gpu_mask = self.mask_preview.as_ref().is_some_and(|preview| preview.compositor.uses_gpu() || preview.compositor.graph_pending());
        session.caches.iter().enumerate().filter_map(move |(ts, cache)| {
            let gpu_current = self.prefer_gpu_layers
                && self.compositors.get(ts).is_some_and(|state| {
                    state.graph_pending()
                        || state.uses_gpu() && state.current.is_some_and(|key| key.mesh == session.mesh_id && key.revision == cache.material_revision)
                });
            (!gpu_mask && !gpu_current).then_some(cache)
        })
    }

    /// CPU display work can stay deferred while the current detail textures are GPU-resident.
    pub(crate) fn display_pending(&self, session: &Session) -> bool {
        self.cpu_sets(session).any(|cache| !cache.dirty.is_empty() || !cache.settle.is_empty() || !cache.upload.is_empty())
    }

    pub(crate) fn gpu_composite_pending(&self) -> bool {
        self.gpu_composite_in_flight.load(Ordering::Acquire) > 0
            || self.compositors.iter().any(SetCompositor::graph_pending)
            || self.mask_preview.as_ref().is_some_and(|preview| preview.compositor.graph_pending())
    }

    pub(crate) fn cpu_padding_pending(&self) -> bool {
        self.cpu_padding_pending
    }

    pub(crate) fn compositor_status(&self) -> (&'static str, &'static str) {
        if let Some(preview) = &self.mask_preview {
            let compositor = &preview.compositor;
            if compositor.graph_pending() {
                return (n_("Preparing mask"), n_("Computing the imported graph inputs."));
            }
            if compositor.uses_gpu() {
                return (n_("Mask: GPU"), n_("The graphics card composes the displayed mask."));
            }
            let reason = if compositor.failed.load(Ordering::Acquire) { GPU_COMPOSITE_ERROR } else { compositor.reason };
            return (n_("Mask: CPU"), reason);
        }
        if self.compositors.iter().any(SetCompositor::graph_pending) {
            return (n_("Preparing layers"), n_("Computing the imported graph inputs."));
        }
        let cpu = self.compositors.iter().find(|set| !set.uses_gpu());
        let uses_gpu = self.compositors.iter().any(|set| set.uses_gpu() && !set.neutral);
        let reason =
            cpu.map_or(n_("Waiting for the first material update"), |set| if set.failed.load(Ordering::Acquire) { GPU_COMPOSITE_ERROR } else { set.reason });
        match (uses_gpu, cpu) {
            (true, None) => (n_("Layers: GPU"), n_("The graphics card composes the material.")),
            (true, Some(_)) => (n_("Layers: GPU + CPU"), reason),
            (false, None) if !self.compositors.is_empty() => (n_("Layers: idle"), n_("No layers need compositing.")),
            (false, _) => (n_("Layers: CPU"), reason),
        }
    }

    fn gpu_composite_reason(&self, interactive: bool) -> &'static str {
        if !self.prefer_gpu_layers {
            n_("CPU is selected for layer compositing in Settings.")
        } else if !self.gpu_composite_enabled {
            n_("GPU compositing is unavailable on this device.")
        } else if interactive {
            n_("The active view uses CPU compositing.")
        } else {
            ""
        }
    }

    fn plan_gpu_composites(&mut self, session: &mut Session, shift: u32, interactive: bool, gpu_quick_mask: bool, out: &mut SceneSync, started: f64) {
        let global_reason = self.gpu_composite_reason(interactive || session.solo.is_some());
        let mut submitted_stroke = false;
        let mut cpu_changed = false;
        for (ts, state) in self.compositors.iter_mut().enumerate() {
            let reason = match state.plan(session, ts, shift, global_reason, None) {
                Ok(program) => {
                    if let Some(program) = program {
                        out.composites.push((ts, program, state.failed.clone()));
                        submitted_stroke |= session.pending_display_stroke(ts).is_some();
                        self.sent.entry(ts).or_default().extend(DisplayKind::ALL);
                    }
                    self.pending_overviews.remove(&ts);
                    self.active_overviews.remove(&ts);
                    ""
                }
                Err(reason) => {
                    if session.stroke_texture_set() == Some(ts) && !(gpu_quick_mask && session.quick_mask_stroke(ts).is_some()) {
                        session.materialize_device_stroke(Some(reason));
                    }
                    state.stroke = None;
                    if state.current.take().is_some() {
                        session.caches[ts].queue_ready_uploads();
                    }
                    reason
                }
            };
            state.reason = reason;
            let (backend, detail) = if state.graph_pending() {
                ("preparing", "imported graph inputs")
            } else if !reason.is_empty() {
                ("CPU", reason)
            } else if state.neutral {
                ("idle", "no layers need compositing")
            } else {
                ("GPU", "layer compositor")
            };
            if state.logged_status != Some((backend, detail)) {
                state.logged_status = Some((backend, detail));
                out.compositor_changed = true;
                let (w, h) = session.caches[ts].size;
                session.log.performance(format!("Layers: {backend} for texture set {} at {w}×{h}: {detail}", ts + 1));
                cpu_changed |= !reason.is_empty();
            }
        }
        if submitted_stroke {
            session.clear_display_stroke_tiles();
        }
        if cpu_changed && !cfg!(target_arch = "wasm32") {
            self.cpu_composite_started = Some(started);
        }
    }

    fn plan_gpu_mask(&mut self, session: &mut Session, aux_map: Option<AuxView>, shift: u32, baking: bool, out: &mut SceneSync) -> bool {
        let Some(AuxView::Mask(layer)) = aux_map else {
            out.compositor_changed |= self.mask_preview.take().is_some();
            return false;
        };
        let Some((ts, _, _)) = session.doc.project.locate(layer) else {
            self.mask_preview = None;
            return false;
        };
        if self.mask_preview.as_ref().is_none_or(|preview| preview.ts != ts || preview.layer != layer) {
            self.mask_preview = Some(MaskPreview { ts, layer, compositor: SetCompositor::default() });
            out.compositor_changed = true;
        }
        let global_reason = self.gpu_composite_reason(baking);
        let Some(preview) = self.mask_preview.as_mut() else { return false };
        let state = &mut preview.compositor;
        let reason = match state.plan(session, ts, shift, global_reason, Some(layer)) {
            Ok(program) => {
                if let Some(program) = program {
                    out.composites.push((ts, program, state.failed.clone()));
                    if session.stroke_mask_owner() == Some(session.doc.project.mask_owner(layer)) {
                        session.clear_display_stroke_tiles();
                    }
                    // Aux now holds GPU pixels instead of the prior CPU upload.
                    self.aux.remove(&ts);
                }
                // A material view that is re-opened mid-stroke must rebuild from the
                // complete coverage, since this view acknowledged the recent tiles.
                for (ts, material) in self.compositors.iter_mut().enumerate() {
                    if material.stroke.is_some() || session.stroke_texture_set() == Some(ts) {
                        if material.current.take().is_some() {
                            session.caches[ts].queue_ready_uploads();
                        }
                        material.stroke = None;
                    }
                }
                ""
            }
            Err(reason) => {
                state.current = None;
                state.stroke = None;
                reason
            }
        };
        state.reason = reason;
        let backend = if state.graph_pending() {
            ("preparing", "imported graph inputs")
        } else if reason.is_empty() {
            ("GPU", "mask compositor")
        } else {
            ("CPU", reason)
        };
        if state.logged_status != Some(backend) {
            state.logged_status = Some(backend);
            out.compositor_changed = true;
            let (backend, detail) = backend;
            if let Some(cache) = session.caches.get(ts) {
                let (w, h) = cache.size;
                session.log.performance(format!("Mask: {backend} for texture set {} at {w}×{h}: {detail}", ts + 1));
            }
        }
        reason.is_empty()
    }

    pub(crate) fn pending_cpu_tiles(&self, session: &Session) -> usize {
        self.cpu_sets(session).map(|cache| cache.dirty.len() + cache.settle.len()).sum()
    }

    /// Compositing progress (0..1) while CPU tiles wait to be composited.
    pub fn composite_progress(&self, session: &Session) -> Option<f32> {
        let pending = self.pending_cpu_tiles(session);
        (pending > 0 && self.composite_total > 0).then(|| (1.0 - pending as f32 / self.composite_total as f32).clamp(0.0, 1.0))
    }

    /// Collect everything the GPU needs this frame, running the compositor first: `bake` is what bake mode's 3D view draws besides the low poly, `spent_ms` the time this frame already took
    pub fn sync(&mut self, session: &mut Session, aux_map: Option<AuxView>, bake: Option<BakeVisualization>, shift: u32, spent_ms: f64) -> SceneSync {
        self.sync_view(session, aux_map, bake, shift, spent_ms, None)
    }

    pub(crate) fn sync_view(
        &mut self,
        session: &mut Session,
        aux_map: Option<AuxView>,
        bake: Option<BakeVisualization>,
        shift: u32,
        spent_ms: f64,
        visible_uv: Option<[Vec2; 2]>,
    ) -> SceneSync {
        let start_ms = crate::now_ms();
        let mut queued = 0usize;
        let time_left_ms = |queued: usize| FRAME_BUDGET_MS - spent_ms - (crate::now_ms() - start_ms) - queued as f64 * UPLOAD_MS_PER_TILE;
        let demand = session.tile_demand(visible_uv);

        let mut out = SceneSync { mesh_id: session.mesh_id, tile_shift: shift.min(texturecraft_engine::texel::TILE.ilog2()), ..Default::default() };
        if self.lost.swap(false, Ordering::AcqRel) {
            self.mesh_id = 0;
            self.sets.clear();
            self.env = Default::default();
            self.bake_scene.resend();
            self.reset.store(true, Ordering::Release);
        }

        if self.mesh_id != session.mesh_id {
            out.mesh = Some((session.mesh.clone(), session.mesh_id));
            self.mesh_id = session.mesh_id;
            self.aux.clear();
        }

        let sets: Vec<SetLayout> = session.doc.project.texture_sets.iter().map(|texture_set| (texture_set.size(), texture_set.material)).collect();
        let resend = self.reset.swap(false, Ordering::AcqRel);
        self.resized_sets.retain(|_, (project, staged)| *project == session.project_id && !staged.load(Ordering::Acquire));
        out.resized_sets.extend(self.resized_sets.iter().map(|(&ts, (_, staged))| (ts, staged.clone())));
        if resend || sets != self.sets || shift != self.shift || out.mesh.is_some() || !out.resized_sets.is_empty() {
            self.compositors.resize_with(sets.len(), SetCompositor::default);
            self.pending_overviews.retain(|&ts| ts < sets.len());
            for (ts, layout) in sets.iter().enumerate() {
                if resend || out.mesh.is_some() || self.shift != shift || self.sets.get(ts) != Some(layout) || self.resized_sets.contains_key(&ts) {
                    self.compositors[ts].reset_view();
                    let Some(cache) = session.caches.get_mut(ts) else { continue };
                    // The renderer may recreate these textures. Dirty tiles will enter the
                    // upload queue after composition; sending them now sends blank pixels.
                    cache.queue_ready_uploads();
                    self.pending_overviews.remove(&ts);
                    if self.material_start_preview && cache.dirty.len() >= UPLOAD_CHUNK {
                        self.pending_overviews.insert(ts);
                    }
                }
            }
            // Retained GPU sets need their prior kinds to overwrite stale textures
            self.sent.retain(|&texture_set, _| texture_set < sets.len());

            // Recreated sets start with blank aux textures: resend the shown view
            self.aux.clear();
            self.mask_preview = None;
            self.quick_mask_failed = Arc::default();

            out.sets = Some((sets.clone(), session.mesh.materials.len().max(1), shift));
            self.sets = sets;
            self.shift = shift;
        }

        let display = &session.doc.project.display;
        let image = display.environment_resource().and_then(|name| session.images.get(name));
        if self.env.0 != display.environment || self.env.1.as_ref().map(Weak::as_ptr) != image.map(Arc::as_ptr) {
            self.env = (display.environment.clone(), image.map(Arc::downgrade));
            out.env = Some((display.environment.clone(), image.cloned()));
        }

        for (texture_set, kinds) in std::mem::take(&mut *self.recreated.lock().unwrap_or_else(PoisonError::into_inner)) {
            if texture_set < self.sets.len() {
                self.sent.insert(texture_set, kinds);
            }
        }

        if !self.material_start_preview {
            self.pending_overviews.clear();
            for ts in std::mem::take(&mut self.active_overviews) {
                if let Some(cache) = session.caches.get_mut(ts) {
                    out.cleared_overviews.push(ts);
                    cache.queue_ready_uploads();
                }
            }
        }

        self.cpu_padding_pending = false;
        if aux_map != Some(AuxView::QuickMask) && self.quick_mask_failed.load(Ordering::Acquire) {
            self.quick_mask_failed = Arc::default();
        }
        let gpu_quick_mask = aux_map == Some(AuxView::QuickMask)
            && bake.is_none()
            && !cfg!(target_arch = "wasm32")
            && self.prefer_gpu_layers
            && self.gpu_composite_enabled
            && !self.quick_mask_failed.load(Ordering::Acquire);
        out.release_quick_mask = !gpu_quick_mask;
        let interactive = (aux_map.is_some() && !gpu_quick_mask) || bake.is_some();
        let gpu_mask = self.plan_gpu_mask(session, aux_map, shift, bake.is_some(), &mut out);
        if !gpu_mask {
            self.plan_gpu_composites(session, shift, interactive, gpu_quick_mask, &mut out, start_ms);
        }
        if out.composites.iter().any(|(_, program, _)| !program.is_neutral()) {
            self.gpu_composite_in_flight.fetch_add(1, Ordering::AcqRel);
            out.composite_work = Some(CompositeWork { in_flight: self.gpu_composite_in_flight.clone(), started: out.compositor_changed.then_some(start_ms) });
        }
        let pending = self.cpu_sets(session).map(|cache| cache.dirty.len() + cache.settle.len()).sum::<usize>();
        self.composite_total = if pending == 0 { 0 } else { self.composite_total.max(pending) };
        let uses_cpu = |ts: usize| !gpu_mask && !self.compositors.get(ts).is_some_and(|state| state.uses_gpu() || state.graph_pending());

        let count = session.caches.len();
        if !interactive || !self.prefer_gpu_layers {
            out.cpu_texture_sets.extend((0..count).filter(|&ts| uses_cpu(ts)));
        }
        let active = session.ts();
        let padding_pending = BTreeSet::<usize>::new();
        #[cfg(not(target_arch = "wasm32"))]
        let padding_pending = {
            let mut not_ready = padding_pending;
            // Start at most one dense map at a time, with the active material first.
            let first = active.min(count);
            let (before, from_active) = session.caches.split_at_mut(first);
            let candidates = (first..count)
                .zip(from_active)
                .chain((0..first).zip(before))
                .filter(|(ts, cache)| uses_cpu(*ts) && cache.size.0.max(cache.size.1) > 1024 && (!cache.dirty.is_empty() || !cache.settle.is_empty()));
            for (ts, cache) in candidates {
                if !cache.dilation.is_ready() && (!not_ready.is_empty() || !cache.dilation.prepare_on_worker()) {
                    not_ready.insert(ts);
                }
            }
            self.cpu_padding_pending = !not_ready.is_empty();
            not_ready
        };
        // At most one small full-atlas evaluation per frame, active material first.
        let next_overview = if gpu_mask {
            None
        } else if self.pending_overviews.remove(&active) {
            Some(active)
        } else {
            self.pending_overviews.pop_first()
        };
        if let Some(ts) = next_overview
            && !(ts == active && demand.has_brush())
            && !session.caches[ts].dirty.is_empty()
            && let Some(overview) = session.material_overview(ts, 128)
        {
            self.sent.entry(ts).or_default().extend(overview.maps.iter().map(|(kind, _)| *kind));
            self.active_overviews.insert(ts);
            out.overview = Some((ts, overview));
        }
        if pending > 0 && (out.overview.is_none() || demand.has_brush()) {
            session.update_for_view_where(COMPOSITE_BUDGET_MS.min(time_left_ms(0)).max(MIN_SLICE_MS), crate::now_ms, demand, |ts| {
                uses_cpu(ts) && !padding_pending.contains(&ts)
            });
        }
        let first = self.upload_cursor.take().filter(|&set| set < count && out.mesh.is_none()).unwrap_or(active);
        let rotation = (first..count).chain(0..first.min(count));
        // While painting, share the first chunk with the next waiting set. An overloaded
        // frame still submits at most one chunk in total, independent of material count.
        let boost = uses_cpu(active)
            && (session.stroke_active() || demand.focused())
            && active != first
            && session.caches.get(active).is_some_and(|c| !c.upload.is_empty());
        let mut chunk_size = if boost { UPLOAD_CHUNK / 2 } else { UPLOAD_CHUNK };
        let order = boost.then_some((active, true)).into_iter().chain(rotation.map(|set| (set, false)));
        'uploads: for (texture_set, boosted) in order.filter(|(ts, _)| uses_cpu(*ts)) {
            let cache = &mut session.caches[texture_set];
            if cache.upload.is_empty() || self.pending_overviews.contains(&texture_set) {
                continue;
            }
            let empty = Default::default();
            let baked = session.doc.baked.get(&texture_set).unwrap_or(&empty);
            let channels: Vec<Channel> = cache.composite.channels.keys().copied().collect();
            let sent = self.sent.entry(texture_set).or_default();
            let kinds: Vec<DisplayKind> =
                DisplayKind::ALL.into_iter().filter(|kind| sent.contains(kind) || kinds_for(&channels).any(|needed| needed == *kind)).collect();
            sent.extend(kinds.iter().copied());

            while !cache.upload.is_empty() {
                let chunk = cache.take_upload_batch(chunk_size, if texture_set == active { demand } else { Default::default() });
                if !kinds.is_empty() {
                    out.released_dabs.extend(cache.released_dab_in(&chunk));
                }
                let composite = &cache.composite;
                let bytes: Vec<_> = chunk
                    .par_iter()
                    .flat_map_iter(|&(x, y)| kinds.iter().map(move |&kind| (kind, x, y)))
                    .map(|(kind, x, y)| (texture_set, kind, x, y, display_tile_bytes(composite, baked, kind, x, y, out.tile_shift)))
                    .collect();
                queued += bytes.len();
                out.tiles.extend(bytes);
                if boosted {
                    break;
                }
                chunk_size = UPLOAD_CHUNK;
                if time_left_ms(queued) < MIN_SLICE_MS {
                    self.upload_cursor = Some((texture_set + 1) % count);
                    break 'uploads;
                }
            }
        }

        self.active_overviews.retain(|&ts| session.caches.get(ts).is_some_and(|c| !c.dirty.is_empty() || !c.upload.is_empty()));
        out.ready_cpu_sets.extend(
            session
                .caches
                .iter()
                .enumerate()
                .filter(|(ts, cache)| uses_cpu(*ts) && cache.dirty.is_empty() && cache.settle.is_empty() && cache.upload.is_empty())
                .map(|(ts, _)| ts),
        );
        if !gpu_mask
            && !self.display_pending(session)
            && let Some(started) = self.cpu_composite_started.take()
        {
            session.log.performance(format!("CPU layer composition and display tile preparation completed in {:.0} ms", crate::now_ms() - started));
        }

        if self.aux_image.as_ref().is_some_and(|image| image.strong_count() == 0) {
            self.aux_image = None;
            self.aux.retain(|_, (view, _)| view != "image");
        }

        match aux_map {
            Some(AuxView::MeshMap(map)) => {
                for texture_set in 0..session.caches.len() {
                    // Streamed bake maps replace document maps while the bake is visible
                    let preview = session.bake_preview_baked(texture_set);
                    let baked = preview.or_else(|| session.doc.baked.get(&texture_set));
                    let have = baked.and_then(|baked| baked.maps.get(&map));
                    let revision = have.map(|data| Arc::as_ptr(data) as u64).unwrap_or(0);
                    let key = (format!("{map:?}{}", if preview.is_some() { "~" } else { "" }), revision);
                    if self.aux.get(&texture_set) == Some(&key) {
                        continue;
                    }

                    let Some(cache) = session.caches.get(texture_set) else { continue };

                    let resolution = texturecraft_render::aux_size(cache.size);
                    let rgba = if preview.is_some() && have.is_none() {
                        texturecraft_render::mesh_maps::neutral_rgba(map, resolution)
                    } else {
                        texturecraft_render::mesh_maps::mesh_map_rgba(baked, map, resolution, cache.size, &cache.composite.block_scales)
                    };
                    out.aux.push((texture_set, resolution, rgba));
                    self.aux.insert(texture_set, key);
                }
            }
            Some(AuxView::Mask(id)) => {
                if !gpu_mask && let Some((texture_set, _, _)) = session.doc.project.locate(id) {
                    let key = (format!("mask{}", id.0), session.revision);
                    if self.aux.get(&texture_set) != Some(&key) && !session.stroke_active() {
                        let resolution =
                            session.caches.get(texture_set).map_or(MASK_VIEW_FALLBACK_RESOLUTION, |cache| texturecraft_render::aux_size(cache.size));
                        let rgba = session.mask_view_rgba(id, resolution).unwrap_or_else(|| MASK_VIEW_BLANK.repeat((resolution * resolution) as usize));
                        out.aux.push((texture_set, resolution, rgba));
                        self.aux.insert(texture_set, key);
                    }
                }

                // The other texture sets show nothing of this mask: a flat dark gray
                let shown = session.doc.project.locate(id).map(|(texture_set, _, _)| texture_set);
                for texture_set in (0..session.caches.len()).filter(|set| Some(*set) != shown) {
                    let Some(cache) = session.caches.get(texture_set) else { continue };
                    let resolution = texturecraft_render::aux_size(cache.size);
                    let key = ("maskblank".to_owned(), u64::from(resolution));
                    if self.aux.get(&texture_set) != Some(&key) {
                        out.aux.push((texture_set, resolution, MASK_VIEW_BLANK.repeat((resolution * resolution) as usize)));
                        self.aux.insert(texture_set, key);
                    }
                }
            }
            Some(AuxView::Image) => {
                let shown = session.view_image.as_deref().and_then(|name| session.images.get(name));
                let same_image = match (&self.aux_image, shown) {
                    (Some(previous), Some(image)) => std::ptr::eq(previous.as_ptr(), Arc::as_ptr(image)),
                    (None, None) => true,
                    _ => false,
                };
                if !same_image {
                    self.aux.clear();
                    self.aux_image = shown.map(Arc::downgrade);
                }

                for (texture_set, cache) in session.caches.iter().enumerate() {
                    if self.aux.get(&texture_set).is_some_and(|(view, _)| view == "image") {
                        continue;
                    }

                    let resolution = texturecraft_render::aux_size(cache.size);
                    let (sets, _) = out.image_aux.entry(resolution).or_insert_with(|| {
                        let rgba = session.image_view_rgba(resolution).unwrap_or_else(|| MASK_VIEW_BLANK.repeat((resolution * resolution) as usize));
                        (Vec::new(), rgba)
                    });
                    sets.push(texture_set);
                    self.aux.insert(texture_set, ("image".into(), 0));
                }
            }
            Some(AuxView::QuickMask) => {
                for texture_set in 0..session.caches.len() {
                    let Some(cache) = session.caches.get(texture_set) else { continue };

                    let key = (format!("quickmask{gpu_quick_mask}:{}:{}", session.quick_mask.revision, cache.material_revision), cache.revision);
                    let base_changed = gpu_quick_mask && out.changes_base_color(texture_set);
                    if !base_changed && self.aux.get(&texture_set) == Some(&key) {
                        continue;
                    }

                    if gpu_quick_mask {
                        let stroke = session.quick_mask_stroke(texture_set).map(|stroke| {
                            #[cfg(not(target_arch = "wasm32"))]
                            if let Some(device) = stroke.device.and_then(|stroke| stroke.as_any().downcast_ref::<GpuBrushStroke>()) {
                                return (QuickMaskAlpha::Gpu(device.view()), stroke.opacity, stroke.erase);
                            }
                            (QuickMaskAlpha::Cpu(stroke.alpha.clone()), stroke.opacity, stroke.erase)
                        });
                        out.quick_masks.push((
                            texture_set,
                            QuickMaskOverlay { mask: session.quick_mask_image(texture_set, cache.size), inverted: session.quick_mask.inverted, stroke },
                        ));
                        out.quick_mask_failed = Some(self.quick_mask_failed.clone());
                    } else {
                        let resolution = texturecraft_render::aux_size(cache.size);
                        if let Some(rgba) = session.quick_mask_rgba(texture_set, resolution) {
                            out.aux.push((texture_set, resolution, rgba));
                        }
                    }
                    self.aux.insert(texture_set, key);
                }
            }
            None => {}
        }

        self.bake_scene.sync(session, bake, &mut out);
        out
    }
}

fn record_recreated_uploads(recreated: &mut BTreeMap<usize, BTreeSet<DisplayKind>>, reset: &[usize], sync: &SceneSync) {
    for &texture_set in reset {
        recreated.insert(texture_set, BTreeSet::new());
    }

    // Skip the tile scan on frames without a pending recreation acknowledgement
    if recreated.is_empty() {
        return;
    }

    for (ts, program, failed) in &sync.composites {
        if !program.is_mask_preview()
            && !failed.load(Ordering::Acquire)
            && let Some(kinds) = recreated.get_mut(ts)
        {
            kinds.extend(DisplayKind::ALL);
        }
    }

    if let Some((ts, overview)) = &sync.overview
        && let Some(kinds) = recreated.get_mut(ts)
    {
        kinds.extend(overview.maps.iter().map(|(kind, _)| *kind));
    }
    for (texture_set, kind, _, _, _) in &sync.tiles {
        if let Some(kinds) = recreated.get_mut(texture_set) {
            kinds.insert(*kind);
        }
    }
}

/// Mask view resolution for a texture set without a composite cache
const MASK_VIEW_FALLBACK_RESOLUTION: u32 = 1024;

/// RGBA of the texel the other texture sets show in the mask view
const MASK_VIEW_BLANK: [u8; 4] = [38, 38, 38, 255];

/// Paint callback for one view.
pub struct ViewCallback {
    pub frame: Frame,
    /// Scene changes, applied by the first view drawn this frame.
    pub sync: Mutex<Option<SceneSync>>,
    pub rect: egui::Rect,
    /// [`GpuLink::lost`].
    pub lost: Arc<AtomicBool>,
}

impl Drop for ViewCallback {
    fn drop(&mut self) {
        if self.sync.get_mut().unwrap_or_else(PoisonError::into_inner).is_some() {
            self.lost.store(true, Ordering::Release);
        }
    }
}

impl CallbackTrait for ViewCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<GpuState>() else { return Vec::new() };
        let sync = self.sync.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(mut sync) = sync {
            if let Some((env, image)) = &sync.env {
                gpu.renderer.set_environment(device, queue, env, image.as_deref());
            }
            if let Some((mesh, id)) = &sync.mesh {
                gpu.renderer.set_mesh(device, queue, mesh, *id);
            }
            let timing = sync.composite_work.as_ref().and_then(|work| work.started).map(|_| gpu.reports.as_ref());
            let (reset, errors) = sync.apply_display_reporting(&mut gpu.renderer, device, queue, timing);
            let submitted = sync.composites.iter().any(|(_, program, failed)| !program.is_neutral() && !failed.load(Ordering::Acquire));
            if !errors.is_empty() {
                let mut reports = gpu.reports.lock().unwrap_or_else(PoisonError::into_inner);
                reports
                    .performance
                    .extend(errors.into_iter().map(|(ts, error)| (sync.mesh_id, format!("GPU layer submission failed for texture set {}: {error}", ts + 1))));
                drop(reports);
                gpu.ctx.request_repaint();
            }
            if submitted && let Some(work) = sync.composite_work.take() {
                let trace = work.started.map(|started| (started, crate::now_ms() - started, gpu.reports.clone()));
                let (mesh, ctx) = (sync.mesh_id, gpu.ctx.clone());
                queue.on_submitted_work_done(move || {
                    if let Some((started, cpu_ms, reports)) = trace {
                        let elapsed = crate::now_ms() - started;
                        reports.lock().unwrap_or_else(PoisonError::into_inner).performance.push((
                            mesh,
                            format!(
                                "GPU layer work completed in {elapsed:.0} ms since viewport update started (CPU preparation and submission {cpu_ms:.0} ms)"
                            ),
                        ));
                    }
                    drop(work);
                    ctx.request_repaint();
                });
            }
            record_recreated_uploads(&mut gpu.recreated.lock().unwrap_or_else(PoisonError::into_inner), &reset, &sync);
            for (texture_set, size, bytes) in &sync.aux {
                gpu.renderer.upload_aux(device, queue, *texture_set, *size, bytes);
            }
            for (size, (sets, bytes)) in &sync.image_aux {
                for &texture_set in sets {
                    gpu.renderer.upload_aux(device, queue, texture_set, *size, bytes);
                }
            }
            if let Some((high, id)) = &sync.bake_high
                && let Err(error) = gpu.renderer.set_bake_high(device, queue, high.as_deref(), *id)
            {
                let mut reports = gpu.reports.lock().unwrap_or_else(PoisonError::into_inner);
                if reports.errors.len() < MAX_GPU_ERRORS && !reports.errors.contains(&error) {
                    reports.errors.push(error);
                }
                gpu.ctx.request_repaint();
            }
            if let Some((id, missed)) = &sync.bake_high_missed {
                gpu.renderer.set_bake_high_missed(queue, *id, missed);
            }
            if let Some(count) = sync.bake_cage_count {
                gpu.renderer.truncate_bake_cages(count);
            }
            for (texture_set, cage, topology) in &sync.bake_cages {
                let cage = cage.as_ref().map(|upload| CageGeometry {
                    corners: &upload.corners,
                    edges: &upload.shell.edges,
                    missing_seams: &upload.shell.missing_seams,
                });
                gpu.renderer.set_bake_cage(device, queue, *texture_set, cage, *topology);
            }
            if let Some((mesh_id, seams)) = &sync.missing_seams {
                gpu.renderer.set_missing_seams(device, queue, *mesh_id, seams);
            }
        }

        // Also installs (or rejects) full-size textures WebGPU answered for since last frame
        gpu.renderer.prepare(device, queue, encoder, &self.frame);
        let failed = gpu.renderer.take_memory_failures();
        if !failed.is_empty() {
            let mut reports = gpu.reports.lock().unwrap_or_else(PoisonError::into_inner);
            for texture_set in failed {
                if !reports.memory.contains(&texture_set) {
                    reports.memory.push(texture_set);
                }
            }

            // This frame's report drain already ran: the next frame shows the toast
            gpu.ctx.request_repaint();
        }
        if gpu.renderer.awaiting_gpu() {
            gpu.ctx.request_repaint();
        }

        let pixels_per_point = screen.pixels_per_point;
        let rect = self.rect;
        let blit_rect = [rect.min.x * pixels_per_point, rect.min.y * pixels_per_point, rect.width() * pixels_per_point, rect.height() * pixels_per_point];
        gpu.renderer.set_blit_rect(queue, self.frame.view_id, blit_rect, screen.size_in_pixels);
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(gpu) = resources.get::<GpuState>() else { return };
        let [sw, sh] = info.screen_size_px;
        let clip = info.clip_rect_in_pixels();
        if clip.width_px <= 0 || clip.height_px <= 0 || sw == 0 || sh == 0 {
            return;
        }
        pass.set_viewport(0.0, 0.0, sw as f32, sh as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.left_px.max(0) as u32, clip.top_px.max(0) as u32, clip.width_px as u32, clip.height_px as u32);
        gpu.renderer.blit_prepared(pass, self.frame.view_id);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use texturecraft_engine::execute;
    use texturecraft_engine::geom::primitives;
    #[cfg(not(target_arch = "wasm32"))]
    use texturecraft_render::headless::Headless;

    #[cfg(not(target_arch = "wasm32"))]
    struct HeadlessBrush(Mutex<Headless>);

    #[cfg(not(target_arch = "wasm32"))]
    impl brush_device::BrushDevice for HeadlessBrush {
        fn start(&self, surface: brush_device::BrushSurface) -> Result<Box<dyn brush_device::DeviceStroke>, String> {
            let Headless { renderer, device, queue, .. } = &mut *self.0.lock().unwrap();
            DeviceStroke::start(renderer, device, queue, surface)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn hiding_a_texture_set_lets_the_brush_reach_the_surface_behind_it() {
        use texturecraft_engine::geom::glam::Vec3;

        let check = |device: Option<Arc<dyn brush_device::BrushDevice>>| {
            let mut floor = primitives::plane(2.0, 1);
            floor.materials[0] = "Floor".into();
            let mut mesh = primitives::plane(1.0, 1);
            mesh.materials[0] = "Shelf".into();
            for point in &mut mesh.positions {
                *point = Vec3::new(point.x - 0.5, 0.15, point.z * 2.0);
            }
            mesh.append(&floor);

            let mut session = Session::new_project("Overlap", mesh, "two planes", None, 128).unwrap();
            session.viewport = Vec2::splat(400.0);
            session.camera.yaw = 0.0;
            session.camera.pitch = 85.0_f32.to_radians();
            execute(&mut session, "textureSet.select", &json!({"ts": 1})).unwrap();
            if let Some(device) = &device {
                session.brush_device = Some(brush_device::BrushBackend { device: device.clone(), surface: Arc::downgrade(&session.caches[1].gbuf) });
            }
            let layer = session.selected_layer().unwrap();
            execute(&mut session, "tool.brush", &json!({"size": 20, "hardness": 100, "flow": 100})).unwrap();
            execute(&mut session, "tool.material", &json!({"channel": "baseColor", "solo": true, "color": [1.0, 0.0, 0.0]})).unwrap();

            let screen = |session: &Session, x, z| {
                let p = session.camera.project(Vec3::new(x, 0.0, z), 1.0).unwrap();
                Vec2::new(p.x + 1.0, 1.0 - p.y) * (session.viewport * 0.5)
            };
            let alpha = |session: &Session, x, z| {
                let texel = ((Vec2::new(x, z) * 0.5 + Vec2::splat(0.5)) * 128.0).as_uvec2();
                session.doc.pixels.layer(layer, Channel::BaseColor).map_or(0, |image| image.get(texel.x, texel.y)[3])
            };

            for (shelf_visible, z) in [(true, -0.55), (false, 0.0), (true, 0.55)] {
                execute(&mut session, "textureSet.visible", &json!({"ts": 0, "visible": shelf_visible})).unwrap();
                let under_shelf = screen(&session, -0.65, z);
                assert_eq!(session.texture_set_at(under_shelf, session.viewport), Some(usize::from(!shelf_visible)));
                assert!(session.begin_stroke(under_shelf, session.viewport, 1.0).unwrap());
                assert_eq!(session.device_stroke().is_some(), device.is_some());
                session.end_stroke().unwrap();
                assert_eq!(alpha(&session, -0.65, z) > 0, !shelf_visible, "the stroke starts on the floor when the shelf is hidden");

                // The stamp starts on the open floor, but its footprint also reaches under the shelf.
                let center = screen(&session, 0.03, z);
                assert!(session.begin_stroke(center, session.viewport, 1.0).unwrap());
                assert_eq!(session.device_stroke().is_some(), device.is_some());
                session.end_stroke().unwrap();
                assert!(alpha(&session, 0.08, z) > 0, "the uncovered part always takes paint");
                assert_eq!(alpha(&session, -0.08, z) > 0, !shelf_visible, "the shelf only occludes the brush when visible");
            }

            assert!(session.log.iter().all(|entry| !entry.message.contains("GPU stroke readback failed")));
        };

        check(None);
        if let Ok(gpu) = Headless::new() {
            check(Some(Arc::new(HeadlessBrush(Mutex::new(gpu)))));
        }
    }

    pub(crate) fn link() -> GpuLink {
        GpuLink {
            reset: Arc::default(),
            recreated: Arc::default(),
            reports: Arc::default(),
            lost: Arc::default(),
            mesh_id: 0,
            sets: Vec::new(),
            shift: 0,
            env: Default::default(),
            aux: BTreeMap::new(),
            aux_image: None,
            sent: HashMap::new(),
            adapter: String::new(),
            composite_total: 0,
            upload_cursor: None,
            pending_overviews: BTreeSet::new(),
            active_overviews: BTreeSet::new(),
            resized_sets: BTreeMap::new(),
            gpu_composite_enabled: false,
            #[cfg(not(target_arch = "wasm32"))]
            brush_device: None,
            prefer_gpu_layers: true,
            material_start_preview: true,
            compositors: Vec::new(),
            mask_preview: None,
            quick_mask_failed: Arc::default(),
            gpu_composite_in_flight: Arc::default(),
            cpu_composite_started: None,
            cpu_padding_pending: false,
            bake_scene: BakeSceneLink::default(),
        }
    }

    fn session(emissive: bool) -> Session {
        let mut s = Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
        if emissive {
            execute(&mut s, "textureSet.channel.add", &json!({"channel": "emissive"})).unwrap();
            execute(&mut s, "layer.add.fill", &json!({})).unwrap();
            execute(&mut s, "layer.fill.set", &json!({"channel": "emissive", "color": [1.0, 0.5, 0.0]})).unwrap();
        }
        s
    }

    #[test]
    fn the_mask_view_uploads_the_mask_for_its_set_and_a_blank_for_the_others() {
        let mut gpu = link();
        let mut s = texturecraft_engine::sample_session(128, 64).unwrap();
        assert!(s.doc.project.texture_sets.len() >= 2, "needs another set to blank");
        let layer = texturecraft_engine::core::LayerId(execute(&mut s, "layer.add.fill", &json!({})).unwrap()["layer"].as_u64().unwrap());
        execute(&mut s, "mask.add.white", &json!({})).unwrap();
        let owner = s.doc.project.locate(layer).unwrap().0;
        let sets = s.doc.project.texture_sets.len();

        let out = gpu.sync(&mut s, Some(AuxView::Mask(layer)), None, 0, 0.0);
        assert_eq!(out.aux.len(), sets, "every set gets an aux texture");
        for (set, _, rgba) in &out.aux {
            let blank = rgba.chunks(4).all(|px| px == MASK_VIEW_BLANK);
            assert_eq!(blank, *set != owner, "set {set}: only the other sets are blank");
        }
        drop(out);
        let out = gpu.sync(&mut s, Some(AuxView::Mask(layer)), None, 0, 0.0);
        assert!(out.aux.is_empty(), "nothing is sent again while the view stays");
        drop(out);

        execute(&mut s, "mask.toggle", &json!({"layer": layer.0, "enabled": false})).unwrap();
        let out = gpu.sync(&mut s, Some(AuxView::Mask(layer)), None, 0, 0.0);
        assert_eq!(out.aux.len(), 1, "disabling the displayed mask replaces its old upload");
        let (set, resolution, rgba) = &out.aux[0];
        assert_eq!(*set, owner);
        assert_eq!(*rgba, MASK_VIEW_BLANK.repeat((resolution * resolution) as usize));
        assert_eq!(gpu.aux.len(), sets, "only view keys remain in the link");
        drop(out);
        let out = gpu.sync(&mut s, Some(AuxView::Mask(layer)), None, 0, 0.0);
        assert!(out.aux.is_empty(), "the disabled view needs no repeat upload");
        drop(out);

        execute(&mut s, "mask.toggle", &json!({"layer": layer.0, "enabled": true})).unwrap();
        let out = gpu.sync(&mut s, Some(AuxView::Mask(layer)), None, 0, 0.0);
        assert_eq!(out.aux.len(), 1, "re-enabling the mask uploads its values again");
        assert!(out.aux[0].2.chunks(4).any(|pixel| pixel != MASK_VIEW_BLANK));
    }

    #[test]
    fn a_resized_set_gets_the_shown_image_view_again() {
        let mut gpu = link();
        let mut s = texturecraft_engine::sample_session(128, 64).unwrap();
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]));
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let name = s.import_resource_bytes("stone.png", png).unwrap();
        execute(&mut s, "viewport.view", &json!({"mode": "image", "image": name})).unwrap();

        let out = gpu.sync(&mut s, Some(AuxView::Image), None, 0, 0.0);
        assert_eq!(out.image_aux.len(), 1, "equal resolutions share a single pixel buffer");
        let (targets, bytes) = out.image_aux.values().next().unwrap();
        assert!(targets.len() > 1);
        assert_eq!(targets.len(), s.caches.len());
        assert!(bytes.as_chunks::<4>().0.iter().all(|pixel| *pixel == [9, 9, 9, 255]));
        drop(out);
        let out = gpu.sync(&mut s, Some(AuxView::Image), None, 0, 0.0);
        assert!(out.image_aux.is_empty(), "nothing is sent again while the view stays");
        drop(out);

        // The renderer recreates a resized set's textures blank, in the same frame as this sync
        let shown = s.doc.project.texture_set(0).unwrap().size().0;
        let resolution = if shown == 256 { 512 } else { 256 };
        execute(&mut s, "textureSet.resolution", &json!({"resolution": resolution})).unwrap();
        let out = gpu.sync(&mut s, Some(AuxView::Image), None, 0, 0.0);
        assert!(out.sets.is_some());
        assert_eq!(out.image_aux.get(&resolution).unwrap().0, [0], "the resized set gets pixels at its new resolution");

        // Switching images leaves the previous decoded image owned only by the session
        let original = Arc::downgrade(s.images.get(&name).unwrap());
        execute(&mut s, "resource.remove", &json!({"name": name})).unwrap();
        assert!(original.upgrade().is_none(), "even a pending upload does not hold the decoded source");
        assert!(gpu.sync(&mut s, None, None, 0, 0.0).image_aux.is_empty());
        assert!(gpu.aux_image.is_none());

        let replacement = Arc::new(ImageRes { width: 1, height: 1, rgba: vec![[1.0, 0.0, 0.0, 1.0]], srgb: true });
        s.images.insert(name.clone(), replacement);
        execute(&mut s, "viewport.view", &json!({"mode": "image", "image": name})).unwrap();
        let updated = gpu.sync(&mut s, Some(AuxView::Image), None, 0, 0.0);
        assert_eq!(updated.image_aux.len(), 2);
        assert!(updated.image_aux.values().all(|(_, pixels)| pixels.as_chunks::<4>().0.iter().all(|pixel| *pixel == [255, 0, 0, 255])));
    }

    fn kinds(out: &SceneSync) -> BTreeSet<DisplayKind> {
        out.tiles.iter().filter(|t| t.0 == 0).map(|t| t.1).collect()
    }

    #[test]
    fn a_kind_sent_before_a_project_change_is_overwritten_while_its_textures_survive() {
        let mut gpu = link();
        let out = gpu.sync(&mut session(true), None, None, 0, 0.0);
        assert!(kinds(&out).contains(&DisplayKind::Emissive));

        // A same-size project retains the old emissive texture, so it must be overwritten
        let out = gpu.sync(&mut session(false), None, None, 0, 0.0);
        assert!(out.mesh.is_some() && out.sets.is_some());
        assert!(kinds(&out).contains(&DisplayKind::Emissive), "the stale emissive texture is overwritten");
    }

    #[test]
    fn a_kind_uploaded_after_recreation_is_overwritten_by_the_next_project() {
        let mut gpu = link();
        let out = gpu.sync(&mut session(true), None, None, 0, 0.0);
        assert!(kinds(&out).contains(&DisplayKind::Emissive));

        // The renderer recreated set zero and uploaded this sync's emissive tiles
        record_recreated_uploads(&mut gpu.recreated.lock().unwrap(), &[0], &out);
        gpu.reset.store(true, Ordering::Release);

        let out = gpu.sync(&mut session(false), None, None, 0, 0.0);
        assert!(kinds(&out).contains(&DisplayKind::Emissive), "the uploaded emissive texture is overwritten");
    }

    #[test]
    fn a_recreated_set_converts_only_the_kinds_it_has_channels_for() {
        let mut gpu = link();
        gpu.sync(&mut session(true), None, None, 0, 0.0);
        let mut plain = session(false);
        gpu.sync(&mut plain, None, None, 0, 0.0);
        // The recreated set starts neutral, so old display kinds need no upload
        gpu.recreated.lock().unwrap().insert(0, BTreeSet::new());
        gpu.reset.store(true, Ordering::Release);
        let out = gpu.sync(&mut plain, None, None, 0, 0.0);
        assert!(!out.tiles.is_empty());
        assert!(!kinds(&out).contains(&DisplayKind::Emissive), "nothing to overwrite in a neutral texture");
    }

    #[test]
    fn an_over_budget_frame_services_one_set_and_the_next_frame_services_the_other() {
        let mut gpu = link();
        let mut s = Session::new_project("uploads", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        s.update();
        assert_eq!(s.caches.len(), 2);
        s.active_set = 1;
        for expected in [1, 0] {
            let scene = gpu.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
            assert!(!scene.tiles.is_empty());
            assert!(scene.tiles.iter().all(|tile| tile.0 == expected));
            assert!(s.caches[expected].upload.is_empty());
            if expected == 1 {
                s.caches[1].upload.insert((0, 0));
            }
        }
        assert!(!s.caches[1].upload.is_empty());
        let scene = gpu.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
        assert!(scene.tiles.iter().all(|tile| tile.0 == 1));
        assert!(s.caches.iter().all(|cache| cache.upload.is_empty()));
    }

    #[test]
    fn zoomed_view_and_brush_reach_the_first_upload_chunk() {
        let mut s = Session::new_project("view", primitives::plane(1.0, 1), "builtin:plane", None, 1024).unwrap();
        s.update();
        let all = texturecraft_engine::compose::all_tiles(1024, 1024);
        let mut gpu = link();
        // The zoomed square spans four by four tiles at the lower right of the atlas.
        let bounds = crate::viewport::visible_uv_bounds([512, 512], (1024, 1024), 4.0 / 0.92, [-3.0, 3.0], 0.0);
        let out = gpu.sync_view(&mut s, None, None, 0, FRAME_BUDGET_MS, Some(bounds));
        let coords: BTreeSet<_> = out.tiles.iter().map(|t| (t.2, t.3)).collect();
        for x in 12..16 {
            for y in 12..16 {
                assert!(coords.contains(&(x, y)), "missing visible tile ({x}, {y})");
            }
        }
        assert_eq!(coords.len(), UPLOAD_CHUNK);
        assert_eq!(coords.len() + s.caches[0].upload.len(), all.len());
        s.caches[0].upload.extend(all);
        let brush = Vec2::new(0.96875, 0.34375);
        s.begin_stroke_uv(brush, 1.0).unwrap();
        let out = gpu.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
        assert_eq!(out.tiles.first().map(|t| (t.2, t.3)), Some((15, 5)));
        s.end_stroke().unwrap();
        s.update();
        s.caches[0].upload.extend(texturecraft_engine::compose::all_tiles(1024, 1024));
        let tap = Vec2::splat(0.96875);
        s.begin_stroke_uv(tap, 1.0).unwrap();
        s.end_stroke().unwrap();
        let out = gpu.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
        assert_eq!(out.tiles.first().map(|t| (t.2, t.3)), Some((15, 15)), "a tap released in one frame must still reach the first upload");
        let frame = Frame {
            view_id: 0,
            size: [64, 64],
            kind: texturecraft_render::ViewKind::Scene,
            camera: s.camera,
            shade: Default::default(),
            visible: vec![true],
            overlay: Vec::new(),
        };
        drop(ViewCallback { frame, sync: Mutex::new(Some(out)), rect: egui::Rect::ZERO, lost: gpu.lost.clone() });
        let replay = gpu.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
        assert_eq!(replay.tiles.first().map(|t| (t.2, t.3)), Some((15, 15)), "a dropped callback must preserve the dab's priority");
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(mut gpu) = texturecraft_render::headless::Headless::new() {
            let (_, errors) = replay.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            assert!(errors.is_empty());
            assert!(!s.tile_demand(None).focused(), "a consumed callback clears the released dab's priority");
        }
    }

    #[test]
    fn smaller_viewport_packs_only_the_samples_the_renderer_will_use() {
        use texturecraft_engine::{
            compose::{Composite, tile_bytes},
            texel::TILE,
        };
        let mut gpu = link();
        let mut s = session(false);
        s.update();
        let cache = &mut s.caches[0];
        cache.composite = Composite::new(&Channel::ALL, 128);
        for img in cache.composite.channels.values_mut() {
            let comps = img.comps();
            for y in 0..TILE {
                for x in 0..TILE {
                    let v = ((x + y * TILE) * 13) as u16;
                    img.set(TILE + x, y, &[v, v / 2, u16::MAX - v][..comps]);
                }
            }
        }
        for shift in 0..=TILE.ilog2() {
            s.caches[0].upload = [(1, 0)].into();
            let scene = gpu.sync(&mut s, None, None, shift, 0.0);
            // Changing the display scale recreates the set, including the other ready tiles.
            assert_eq!(scene.tiles.iter().filter(|(_, _, x, y, _)| (*x, *y) == (1, 0)).count(), DisplayKind::ALL.len());
            for (_, kind, x, y, bytes) in &scene.tiles {
                let full = tile_bytes(&s.caches[0].composite, &Default::default(), *kind, *x, *y);
                let step = 1usize << shift;
                let stride = kind.bytes_per_texel();
                let rows = full.chunks_exact(TILE as usize * stride).step_by(step);
                let expected: Vec<_> = rows.flat_map(|row| row.chunks_exact(stride).step_by(step).flatten().copied()).collect();
                assert!(bytes == &expected, "{kind:?} at shift {shift}");
            }
        }
    }

    #[test]
    fn disabling_the_material_overview_keeps_ordinary_tile_uploads() {
        let mut session = Session::new_project("preview switch", primitives::plane(1.0, 1), "builtin:plane", None, 1024).unwrap();
        let mut link = link();
        link.material_start_preview = false;
        let update = link.sync(&mut session, None, None, 0, FRAME_BUDGET_MS);
        assert!(update.overview.is_none());
        assert!(!update.tiles.is_empty());
        // The CPU may already have computed every tile, but the upload budget still
        // leaves part of the material for a later frame.
        assert!(link.display_pending(&session));

        link.material_start_preview = true;
        execute(&mut session, "textureSet.resolution", &json!({"resolution":2048})).unwrap();
        let update = link.sync(&mut session, None, None, 0, FRAME_BUDGET_MS);
        assert!(update.overview.is_some());
        link.material_start_preview = false;
        let update = link.sync(&mut session, None, None, 0, FRAME_BUDGET_MS);
        assert_eq!(update.cleared_overviews, [0]);
        assert!(update.overview.is_none());
        assert!(!update.tiles.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn opening_and_resizing_show_the_whole_material_before_detail_arrives() {
        use texturecraft_render::{Shade, ViewKind, headless::Headless};
        let mut source = Session::new_project("overview", primitives::plane(1.0, 1), "builtin:plane", None, 1024).unwrap();
        execute(&mut source, "layer.add.fill", &json!({"color":[1.0, 0.0, 0.0]})).unwrap();
        let mut s = Session::from_project_bytes(&source.to_project_bytes().unwrap()).unwrap();
        let mut gpu = link();
        let Ok(mut h) = Headless::new() else { return };
        let frame = Frame {
            view_id: 2,
            size: [128, 128],
            kind: ViewKind::Uv { ts: 0, zoom: 1.0 / 0.92, pan: [0.0; 2], rotation: 0.0 },
            camera: s.camera,
            shade: Shade { view_mode: 1, channel: 0, ..Default::default() },
            visible: vec![true],
            overlay: vec![],
        };
        for size in [1024, 2048] {
            if size != 1024 {
                execute(&mut s, "textureSet.resolution", &json!({"resolution":size})).unwrap();
            }
            let mut frames = 0;
            loop {
                let scene = gpu.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
                if let Some((mesh, id)) = &scene.mesh {
                    h.renderer.set_mesh(&h.device, &h.queue, mesh, *id);
                }
                let (reset, _) = scene.apply_display(&mut h.renderer, &h.device, &h.queue);
                record_recreated_uploads(&mut gpu.recreated.lock().unwrap(), &reset, &scene);
                frames += 1;
                let ready = s.pending_tiles() == 0 && s.caches[0].upload.is_empty();
                if frames <= 2 || ready {
                    let (w, height, rgba) = h.render(&frame).unwrap();
                    for fy in [0.1, 0.5, 0.9] {
                        for fx in [0.1, 0.5, 0.9] {
                            let i = ((((height as f32 * fy) as u32) * w + (w as f32 * fx) as u32) * 4) as usize;
                            assert!(rgba[i] > rgba[i + 1].saturating_add(64), "missing material on frame {frames} at {fx}, {fy}");
                        }
                    }
                    if frames == 1 {
                        assert!(!ready, "read back before the detailed tiles are ready");
                    }
                }
                if ready {
                    break;
                }
                assert!(frames < 2048);
            }
            println!("{size}: whole material visible on first rendered frame, detail finished after {frames} syncs");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn procedural_stack(size: u32) -> Session {
        let mut session = Session::new_project("GPU stacks", primitives::plane(1.0, 1), "builtin:plane", None, size).unwrap();
        for (color, pattern, channel, low, high) in [
            ([0.7, 0.5, 0.4], "clouds", "baseColor", [-0.1, 0.15, 0.3], [0.8, 0.75, 0.5]),
            ([0.35, 0.6, 0.75], "grunge", "roughness", [0.25; 3], [0.8; 3]),
            ([0.9, 0.7, 0.5], "cells", "metallic", [0.05; 3], [0.85; 3]),
            ([0.3, 0.3, 0.3], "perlin", "height", [-0.1; 3], [0.15; 3]),
        ] {
            let layer = execute(&mut session, "layer.add.fill", &json!({"color":color,"opacity":0.3})).unwrap();
            execute(
                &mut session,
                "layer.fill.set",
                &json!({"layer":layer["layer"], "channel":channel, "procedural":pattern, "scale":7.5, "seed":73, "low":low,"high":high}),
            )
            .unwrap();
            execute(&mut session, "layer.blend", &json!({"layer":layer["layer"],"channel":"all","opacity":0.3})).unwrap();
        }
        execute(&mut session, "layer.add.paint", &json!({})).unwrap();
        session
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_layers_cover_uv_islands_and_their_padding() {
        use texturecraft_render::{headless::Headless, readback::Readback};
        let Ok(mut headless) = Headless::new() else { return };
        let mut mesh = primitives::plane(1.0, 1);
        for uv in &mut mesh.uvs {
            *uv = *uv * 0.5 + Vec2::splat(0.25);
        }
        let mut session = Session::new_project("inset UVs", mesh, "builtin:plane", None, 128).unwrap();
        execute(&mut session, "layer.add.fill", &json!({"color":[1.0,0.0,0.0]})).unwrap();
        execute(&mut session, "layer.add.paint", &json!({})).unwrap();
        let layer = session.selected_layer().unwrap();
        let paint = session.doc.pixels.layer_mut(layer, Channel::BaseColor, (128, 128));
        paint.set(32, 32, &[0, 0, 65535, 65535]);
        paint.set(33, 32, &[0, 65535, 0, 65535]);
        let geometry_bytes = session.memory().geometry;
        let mut link = link();
        link.gpu_composite_enabled = true;
        let scene = link.sync(&mut session, None, None, 0, 0.0);
        assert!(!scene.composites.is_empty());
        scene.apply_display(&mut headless.renderer, &headless.device, &headless.queue);
        assert!(!link.compositors[0].failed.load(Ordering::Acquire));
        assert_eq!(link.compositor_status().0, "Layers: GPU");
        let (width, _, image) = Readback::blocking(&headless.device, &headless.queue, headless.renderer.display_textures()[0][0]).unwrap();
        let samples = |x: u32, y: u32| {
            let i = (y * width + x) as usize * 4;
            &image[i..i + 4]
        };
        assert_eq!(samples(32, 64), &[255, 0, 0, 255]);
        assert_eq!(samples(31, 64), &[255, 0, 0, 255]);
        assert_ne!(samples(16, 64), &[255, 0, 0, 255]);
        assert_eq!(samples(31, 31), samples(32, 32), "padding across a packed word boundary");
        assert_eq!(samples(33, 31), samples(32, 32), "the leftmost equidistant texel wins");
        assert_eq!(samples(34, 31), samples(33, 32), "the closer texel wins");
        assert_ne!(samples(32, 32), samples(33, 32));
        assert_eq!(session.memory().geometry, geometry_bytes);
        assert!(session.log.iter().any(|entry| entry.source == "Performance" && entry.message.contains("Layers: GPU")));

        execute(&mut session, "effect.add.filter", &json!({"target":"content", "filter":"blur", "radius":32})).unwrap();
        let scene = link.sync(&mut session, None, None, 0, 0.0);
        assert_eq!(scene.composites.len(), 1);
        let (_, errors) = scene.apply_display(&mut headless.renderer, &headless.device, &headless.queue);
        assert!(errors.is_empty(), "{errors:?}");
        let (_, _, blurred) = Readback::blocking(&headless.device, &headless.queue, headless.renderer.display_textures()[0][0]).unwrap();
        let edge = (32 * width as usize + 32) * 4;
        assert_ne!(blurred[edge..edge + 4], image[edge..edge + 4], "the GPU applied the blur to the padded island");
        assert_eq!(link.compositor_status().0, "Layers: GPU");
        assert_eq!(session.memory().geometry, geometry_bytes);

        // A device error still enables the dense padding only on the CPU path.
        scene.composites[0].2.store(true, Ordering::Release);
        let scene = link.sync(&mut session, None, None, 0, 0.0);
        assert!(scene.composites.is_empty());
        assert_eq!(link.compositor_status().0, "Layers: CPU");
        assert!(session.log.iter().any(|entry| entry.message.contains(GPU_COMPOSITE_ERROR)));
        assert!(session.memory().geometry > geometry_bytes, "only the CPU path allocates the dense padding map");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn project_open_and_4k_resize_compose_multiple_sets_on_gpu() {
        use texturecraft_render::{headless::Headless, readback::Readback};

        let Ok(mut headless) = Headless::new() else { return };
        let mut original = Session::new_project("multi-material", primitives::material_ball(), "builtin:material_ball", None, 2048).unwrap();
        assert_eq!(original.caches.len(), 2);
        for (ts, color) in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]].into_iter().enumerate() {
            execute(&mut original, "textureSet.select", &json!({"ts":ts})).unwrap();
            execute(&mut original, "layer.add.fill", &json!({"color":color})).unwrap();
        }
        let mut session = Session::from_project_bytes(&original.to_project_bytes().unwrap()).unwrap();
        drop(original);
        let mut link = link();
        link.gpu_composite_enabled = true;
        link.material_start_preview = false;

        for size in [2048, 4096] {
            if size == 4096 {
                execute(&mut session, "textureSet.resolution", &json!({"ts":0,"resolution":size})).unwrap();
            }
            let scene = link.sync(&mut session, None, None, 0, 0.0);
            let expected: &[usize] = if size == 2048 { &[0, 1] } else { &[0] };
            assert_eq!(scene.composites.iter().map(|(ts, _, _)| *ts).collect::<Vec<_>>(), expected);
            assert!(scene.tiles.is_empty());
            assert!(scene.overview.is_none());
            let (recreated, errors) = scene.apply_display(&mut headless.renderer, &headless.device, &headless.queue);
            assert!(errors.is_empty(), "{errors:?}");
            record_recreated_uploads(&mut link.recreated.lock().unwrap(), &recreated, &scene);
            headless.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            let textures = headless.renderer.display_textures();
            assert_eq!(textures[0][0].width(), size);
            assert_eq!(textures[1][0].width(), 2048);
            assert_eq!(link.compositor_status().0, "Layers: GPU");
            assert!(!link.display_pending(&session));
        }

        for (ts, color) in [[255, 0, 0, 255], [0, 255, 0, 255]].into_iter().enumerate() {
            let (width, _, rgba) = Readback::blocking(&headless.device, &headless.queue, headless.renderer.display_textures()[ts][0]).unwrap();
            let cache = &session.caches[ts];
            assert_eq!(width, cache.gbuf.width);
            let covered = cache.gbuf.tri.iter().position(|&tri| tri != texturecraft_engine::geom::NO_TRI).unwrap();
            assert_eq!(&rgba[covered * 4..covered * 4 + 4], &color);
        }
        execute(&mut session, "textureSet.select", &json!({"ts":1})).unwrap();
        assert!(link.sync(&mut session, None, None, 0, 0.0).composites.is_empty(), "selection retains all uploaded materials");
        execute(&mut session, "effect.add.filter", &json!({"target":"content","filter":"blur","radius":8})).unwrap();
        let scene = link.sync(&mut session, None, None, 0, 0.0);
        assert_eq!(scene.composites.len(), 1);
        assert_eq!(scene.composites[0].0, 1, "only the blurred set needs to be recomposited");
        assert_eq!(link.compositor_status().0, "Layers: GPU");

        // A failed GPU submission for the second set leaves the first on the GPU.
        scene.composites[0].2.store(true, Ordering::Release);
        let _ = session.caches[1].dilation.source(0, 0);
        let before = session.caches[0].revision;
        let scene = link.sync(&mut session, None, None, 0, 0.0);
        assert_eq!(link.compositor_status().0, "Layers: GPU + CPU");
        assert!(scene.composites.is_empty());
        assert!(!scene.tiles.is_empty());
        assert!(scene.tiles.iter().all(|(ts, ..)| *ts == 1));
        assert_eq!(session.caches[0].revision, before, "GPU sets skip desktop CPU composition");
        assert!(session.log.iter().any(|entry| entry.message.contains("Layers: CPU for texture set 2")));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn every_builtin_material_uses_gpu_layers() {
        let mut gpu = texturecraft_render::headless::Headless::new().ok();
        let mut rejected = Vec::new();
        for material in texturecraft_engine::materials::MATERIALS {
            let mut session = Session::new_project("Material shelf", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
            if material.id == "moss" {
                // Adding sheen must keep an existing stack on the GPU.
                for _ in 0..32 {
                    execute(&mut session, "layer.add.fill", &json!({"color": [0.12, 0.19, 0.06]})).unwrap();
                }
            }
            execute(&mut session, "material.apply", &json!({"material": material.id})).unwrap();
            let mut link = link();
            link.gpu_composite_enabled = true;
            let update = link.sync(&mut session, None, None, 0, 0.0);
            let (status, reason) = link.compositor_status();
            if status != "Layers: GPU" || update.composites.len() != 1 || !update.tiles.is_empty() {
                rejected.push(format!("{}: {status} ({reason})", material.id));
            } else if let Some(gpu) = &mut gpu {
                let (_, errors) = update.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
                rejected.extend(errors.into_iter().map(|(_, error)| format!("{}: {error}", material.id)));
                gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            }
        }
        if let Some(gpu) = &gpu {
            println!("Checked {} built-in materials on {}", texturecraft_engine::materials::MATERIALS.len(), gpu.adapter_name);
        }
        assert!(rejected.is_empty(), "built-in materials fell back to CPU: {rejected:#?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn submit_library_preview(link: &mut GpuLink, session: &mut Session, gpu: &mut Headless, aux: Option<AuxView>) -> Result<(), String> {
        let ts = session.ts();
        let update = link.sync(session, aux, None, 0, 0.0);
        let mask = matches!(aux, Some(AuxView::Mask(_)));
        let expected_status = if mask { "Mask: GPU" } else { "Layers: GPU" };
        let (status, reason) = link.compositor_status();
        let uses_expected_route = status == expected_status
            && update.composites.iter().any(|(set, program, _)| *set == ts && program.is_mask_preview() == mask && !program.is_neutral())
            && update.tiles.is_empty()
            && update.aux.iter().all(|(set, ..)| *set != ts);
        if let Some((mesh, id)) = &update.mesh {
            gpu.renderer.set_mesh(&gpu.device, &gpu.queue, mesh, *id);
        }
        let (_, errors) = update.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        for (ts, size, pixels) in update.aux {
            gpu.renderer.upload_aux(&gpu.device, &gpu.queue, ts, size, &pixels);
        }
        if !errors.is_empty() {
            Err(format!("{errors:?}"))
        } else if !uses_expected_route {
            Err(format!("{status} ({reason}); expected a non-neutral {expected_status} update"))
        } else {
            Ok(())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn builtin_library_and_default_generators_render_on_gpu_on_the_baked_sample() {
        use texturecraft_engine::{core::GeneratorKind, materials::MATERIALS, smart_masks::SMART_MASKS};
        use texturecraft_render::readback::Readback;

        let Ok(mut gpu) = Headless::new() else { return };
        const SIZE: u32 = 256;
        let mut session = texturecraft_engine::sample_session(SIZE, SIZE).unwrap();
        execute(&mut session, "layer.add.fill", &json!({"color": [1.0, 0.0, 1.0], "roughness": 0.123})).unwrap();
        let mut link = link();
        link.gpu_composite_enabled = true;
        submit_library_preview(&mut link, &mut session, &mut gpu, None).unwrap();

        let material_pixels = |gpu: &Headless| {
            [DisplayKind::BaseColor, DisplayKind::Orm].map(|kind| {
                let (w, h, pixels) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][kind as usize]).unwrap();
                assert_eq!((w, h), (SIZE, SIZE));
                pixels
            })
        };
        let baseline = material_pixels(&gpu);
        let mut failures = Vec::new();
        for material in MATERIALS {
            execute(&mut session, "material.apply", &json!({"material": material.id})).unwrap();
            match submit_library_preview(&mut link, &mut session, &mut gpu, None) {
                Ok(()) if material_pixels(&gpu) != baseline => {}
                Ok(()) => failures.push(format!("{} did not change the displayed material", material.id)),
                Err(reason) => failures.push(format!("{}: {reason}", material.id)),
            }
            session.undo().unwrap();
        }

        // The pedestal has edges, cavities and differently oriented faces.
        session.active_set = 1;
        execute(&mut session, "material.apply", &json!({"material": "gold_polished"})).unwrap();
        let layer = session.selected_layer().unwrap();
        let mut check_mask = |session: &mut Session, name: &str| {
            session.view = ViewMode::Material;
            if let Err(reason) = submit_library_preview(&mut link, session, &mut gpu, None) {
                failures.push(format!("{name} on the material: {reason}"));
            }

            session.view = ViewMode::Mask(layer);
            if let Err(reason) = submit_library_preview(&mut link, session, &mut gpu, Some(AuxView::Mask(layer))) {
                failures.push(format!("{name} in the mask view: {reason}"));
                return;
            }
            let texture = gpu.renderer.aux_texture(session.ts()).unwrap();
            let (w, h, rgba) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap();
            assert_eq!((w, h), (SIZE, SIZE), "{name}");
            let actual = rgba.as_chunks::<4>().0;
            if actual.iter().any(|[r, g, b, a]| r != g || r != b || *a != 255) {
                failures.push(format!("{name}: mask texture is not opaque grayscale"));
            }
            let expected = session.mask_view_rgba(layer, SIZE).unwrap();
            if expected.as_chunks::<4>().0.iter().any(|p| p[0] > 8)
                && !actual.iter().zip(expected.as_chunks::<4>().0).any(|(gpu, cpu)| gpu[0] > 0 && cpu[0] > 8)
            {
                failures.push(format!("{name}: mask has no visible GPU coverage at the CPU witness pixels"));
            }
        };
        for mask in SMART_MASKS {
            execute(&mut session, "mask.smart.apply", &json!({"name": mask.id, "layer": layer.0})).unwrap();
            check_mask(&mut session, mask.id);
        }
        for generator in GeneratorKind::ALL {
            execute(&mut session, "mask.remove", &json!({"layer": layer.0})).unwrap();
            execute(&mut session, "mask.add.black", &json!({"layer": layer.0})).unwrap();
            let name = format!("{generator:?}");
            execute(&mut session, "effect.add.generator", &json!({"target": "mask", "layer": layer.0, "generator": name})).unwrap();
            check_mask(&mut session, &name);
        }
        println!(
            "Checked {} materials, {} smart masks and {} default generators on {}",
            MATERIALS.len(),
            SMART_MASKS.len(),
            GeneratorKind::ALL.len(),
            gpu.adapter_name
        );
        assert!(failures.is_empty(), "GPU library sweep errors: {failures:#?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn moss_and_leather_display_procedurals_and_sheen_on_gpu() {
        use texturecraft_render::{headless::Headless, readback::Readback};

        let Ok(mut gpu) = Headless::new() else { return };
        for (material, kind) in [("moss", DisplayKind::Sheen), ("leather", DisplayKind::BaseColor)] {
            let mut session = Session::new_project("Material shelf", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
            execute(&mut session, "material.apply", &json!({"material": material})).unwrap();
            let mut link = link();
            link.gpu_composite_enabled = true;
            let update = link.sync(&mut session, None, None, 0, 0.0);
            assert_eq!(link.compositor_status().0, "Layers: GPU", "{material}: {}", link.compositor_status().1);
            let (_, errors) = update.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            assert!(errors.is_empty(), "{material}: {errors:?}");
            session.update();

            let (width, height, bytes) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][kind as usize]).unwrap();
            assert_eq!((width, height), (128, 128), "{material}: full-resolution material texture");
            let cpu = display_tile_bytes(&session.caches[0].composite, &Default::default(), kind, 0, 0, 0);
            for (x, y) in [(9, 7), (12, 21), (33, 55), (47, 15)] {
                let actual = &bytes[(y * width as usize + x) * 4..][..4];
                let expected = &cpu[(y * 64 + x) * 4..][..4];
                assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 2), "{material}: {actual:?} vs {expected:?}");
            }
            if material == "moss" {
                assert!(bytes.as_chunks::<4>().0.iter().any(|p| p[3] > 0), "Moss has visible sheen");
            } else {
                assert!(bytes.as_chunks::<4>().0.iter().any(|p| p != &bytes[..4]), "Leather has visible procedural grain");
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_layers_pack_the_extended_material_channels() {
        use texturecraft_render::{headless::Headless, readback::Readback};

        let mut session = Session::new_project("Material channels", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        execute(&mut session, "layer.add.fill", &json!({})).unwrap();
        for (channel, color) in [
            (Channel::Specular, [0.05, 0.20, 0.65]),
            (Channel::Glossiness, [0.24; 3]),
            (Channel::Emissive, [0.12, 0.28, 0.67]),
            (Channel::ScatterWeight, [0.47; 3]),
            (Channel::CoatWeight, [0.72; 3]),
            (Channel::CoatRoughness, [0.36; 3]),
            (Channel::AnisotropyLevel, [0.64; 3]),
            (Channel::AnisotropyAngle, [0.81; 3]),
            (Channel::SheenColor, [0.11, 0.25, 0.39]),
            (Channel::SheenWeight, [0.44; 3]),
            (Channel::SheenRoughness, [0.17; 3]),
            (Channel::CoatNormal, [0.25, 0.75, 0.85]),
            (Channel::Transmission, [0.57; 3]),
        ] {
            execute(&mut session, "textureSet.channel.add", &json!({"channel": channel.id()})).unwrap();
            execute(&mut session, "layer.fill.set", &json!({"channel": channel.id(), "color": color})).unwrap();
        }

        let mut link = link();
        link.gpu_composite_enabled = true;
        let update = link.sync(&mut session, None, None, 0, 0.0);
        assert_eq!(link.compositor_status().0, "Layers: GPU", "{}", link.compositor_status().1);
        let Ok(mut gpu) = Headless::new() else { return };
        let (_, errors) = update.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        assert!(errors.is_empty(), "{errors:?}");
        session.update();
        let textures = gpu.renderer.display_textures();
        for kind in DisplayKind::ALL {
            if kind == DisplayKind::BaseColor {
                continue;
            }
            let (width, height, actual) = Readback::blocking(&gpu.device, &gpu.queue, textures[0][kind as usize]).unwrap();
            assert_eq!((width, height), (128, 128), "{kind:?}");
            let expected = display_tile_bytes(&session.caches[0].composite, &Default::default(), kind, 0, 0, 0);
            let stride = kind.bytes_per_texel();
            let (x, y) = (9, 15);
            let actual = &actual[(y * width as usize + x) * stride..][..stride];
            let expected = &expected[(y * 64 + x) * stride..][..stride];
            assert_eq!(actual, expected, "{kind:?}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn built_in_sample_composes_both_smart_materials_on_gpu_after_open_and_resize() {
        use texturecraft_render::{headless::Headless, readback::Readback};

        let mut session = Session::closed().unwrap();
        execute(&mut session, "project.sample", &json!({})).unwrap();
        assert_eq!(session.caches.len(), 2);
        let mut link = link();
        link.gpu_composite_enabled = true;
        link.material_start_preview = false;
        let mut headless = Headless::new().ok();
        let opened = std::time::Instant::now();
        for (size, sets) in [(2048, &[0, 1][..]), (4096, &[0][..])] {
            let started = std::time::Instant::now();
            if size == 4096 {
                execute(&mut session, "textureSet.resolution", &json!({"ts":0,"resolution":size})).unwrap();
            }
            let update = link.sync(&mut session, None, None, 0, 0.0);
            let planned: Vec<_> = update.composites.iter().map(|(ts, _, _)| *ts).collect();
            assert_eq!(planned, sets, "{}", link.compositor_status().1);
            assert!(update.tiles.is_empty());
            assert_eq!(link.compositor_status().0, "Layers: GPU");
            if let Some(gpu) = &mut headless {
                let (reset, errors) = update.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
                assert!(errors.is_empty(), "{errors:?}");
                record_recreated_uploads(&mut link.recreated.lock().unwrap(), &reset, &update);
                gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                let textures = gpu.renderer.display_textures();
                assert_eq!(textures[0][0].width(), size);
                assert_eq!(textures[0][2].width(), size, "baked normals reach the GPU display");
                assert_eq!(textures[1][0].width(), 2048);
                println!("{size}: {:?} on {}", started.elapsed(), gpu.adapter_name);
            }
            assert!(!link.display_pending(&session));
        }
        if let Some(gpu) = &mut headless {
            for ts in 0..2 {
                let (width, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[ts][0]).unwrap();
                let cache = &session.caches[ts];
                assert_eq!(width, cache.gbuf.width);
                let mut samples =
                    cache.gbuf.tri.iter().enumerate().step_by(101).filter(|(_, tri)| **tri != texturecraft_engine::geom::NO_TRI).map(|(i, _)| rgba[i * 4]);
                let first = samples.next().unwrap();
                assert!(samples.any(|red| red != first), "visible smart-material variation in set {ts}");
                execute(&mut session, "textureSet.resolution", &json!({"ts":ts,"resolution":128})).unwrap();
            }

            // Compare a few baked and generated sample pixels at 128².
            let update = link.sync(&mut session, None, None, 0, 0.0);
            let (_, errors) = update.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            assert!(errors.is_empty(), "{errors:?}");
            session.update();
            for ts in 0..2 {
                let cache = &session.caches[ts];
                let indices: Vec<_> = cache
                    .gbuf
                    .tri
                    .iter()
                    .enumerate()
                    .step_by(101)
                    .filter_map(|(index, &tri)| (tri != texturecraft_engine::geom::NO_TRI).then_some(index))
                    .collect();
                let tiles = [(0, 0), (0, 1), (1, 0), (1, 1)];
                for (map, kind) in DisplayKind::ALL.into_iter().take(3).enumerate() {
                    let (width, _, bytes) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[ts][map]).unwrap();
                    assert_eq!(width, 128);
                    let cpu: BTreeMap<_, _> =
                        tiles.into_iter().map(|(x, y)| ((x, y), display_tile_bytes(&cache.composite, session.baked(ts).unwrap(), kind, x, y, 0))).collect();
                    let stride = kind.bytes_per_texel();
                    let compare = |i: usize| {
                        let (x, y) = (i % 128, i / 128);
                        let local = (y % 64 * 64 + x % 64) * stride;
                        let cpu = &cpu[&((x / 64) as u32, (y / 64) as u32)][local..local + stride];
                        let gpu = &bytes[i * stride..(i + 1) * stride];
                        match kind {
                            DisplayKind::Normal => u16::from_le_bytes([cpu[4], cpu[5]]).abs_diff(u16::from_le_bytes([gpu[4], gpu[5]])) <= 2,
                            DisplayKind::Orm => cpu[0].abs_diff(gpu[0]) <= 1,
                            _ => cpu[..3].iter().zip(gpu).all(|(a, b)| a.abs_diff(*b) <= 8),
                        }
                    };
                    let agreeing = indices.iter().filter(|&&i| compare(i)).count();
                    assert!(agreeing * 20 >= indices.len() * 19, "{kind:?}, set {ts}: {agreeing}/{} covered samples agree", indices.len());
                }
            }
        }
        println!("sample compositing checks completed in {:?}", opened.elapsed());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "dense native 8K GPU regression; run on an adapter with sufficient VRAM"]
    fn painted_sample_resizes_and_composites_at_4k_and_8k_on_gpu() {
        use texturecraft_engine::{
            ResizeProgress,
            texel::{TILE, TILE_AREA},
        };
        use texturecraft_render::{headless::Headless, readback::Readback};

        let Ok(mut gpu) = Headless::new() else { return };
        let name = gpu.adapter_name.to_lowercase();
        if name.contains("llvmpipe") || name.contains("lavapipe") || name.contains("warp") || gpu.device.limits().max_texture_dimension_2d < 8192 {
            return;
        }
        let mut session = Session::closed().unwrap();
        execute(&mut session, "project.sample", &json!({})).unwrap();
        let layer = *session.doc.project.texture_sets[0].stack.last().unwrap();
        let paint = session.doc.pixels.layer_mut(layer, Channel::BaseColor, (2048, 2048));
        let red = Arc::new([65535, 0, 0, 65535].repeat(TILE_AREA));
        let blue = Arc::new([0, 0, 65535, 65535].repeat(TILE_AREA));
        for y in 0..paint.tiles_y() {
            for x in 0..paint.tiles_x() {
                paint.set_tile(x, y, Some(if y < paint.tiles_y() / 2 { red.clone() } else { blue.clone() }));
            }
        }
        let mut link = link();
        link.gpu_composite_enabled = true;
        link.material_start_preview = false;
        let initial = link.sync(&mut session, None, None, 0, 0.0);
        let (reset, errors) = initial.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        assert!(errors.is_empty(), "{errors:?}");
        record_recreated_uploads(&mut link.recreated.lock().unwrap(), &reset, &initial);
        gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();

        for size in [4096, 8192] {
            let started = std::time::Instant::now();
            let task = session.resize_task(0, size).unwrap();
            let out = crate::resize_for_view(task, &ResizeProgress::default(), Some((gpu.device.clone(), gpu.queue.clone()))).unwrap();
            session.apply_resize(out).unwrap();
            let resize = started.elapsed();
            assert!(session.log.iter().any(|entry| entry.message.contains(&format!("to {size}:")) && entry.message.contains("GPU 1 images")));
            let paint = session.doc.pixels.layer(layer, Channel::BaseColor).unwrap();
            assert!(paint.bytes() > 64 << 20);
            assert_eq!(paint.get(size / 2, size / 4), [65535, 0, 0, 65535]);
            assert_eq!(paint.get(size / 2, size * 3 / 4), [0, 0, 65535, 65535]);

            use std::time::Instant;
            let started = Instant::now();
            let update = link.sync(&mut session, None, None, 0, 0.0);
            assert_eq!(update.composites.iter().map(|(ts, ..)| *ts).collect::<Vec<_>>(), [0], "{}", link.compositor_status().1);
            assert!(update.tiles.is_empty());
            let (reset, errors) = update.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            assert!(errors.is_empty(), "{errors:?}");
            record_recreated_uploads(&mut link.recreated.lock().unwrap(), &reset, &update);
            gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            let compose = started.elapsed();
            assert_eq!(link.compositor_status().0, "Layers: GPU");
            assert!(!link.display_pending(&session));
            let (width, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][0]).unwrap();
            assert_eq!(width, size);
            let cache = &session.caches[0];
            for (rows, color) in [(size / 5..size / 3, [255, 0, 0, 255]), (size * 2 / 3..size * 4 / 5, [0, 0, 255, 255])] {
                let index =
                    rows.flat_map(|y| y * size..(y + 1) * size).find(|&i| cache.gbuf.tri[i as usize] != texturecraft_engine::geom::NO_TRI).unwrap() as usize;
                assert_eq!(&rgba[index * 4..index * 4 + 4], &color);
            }
            println!("painted {size} on {}: resize {resize:?}, GPU composite {compose:?} (document tile edge {TILE})", gpu.adapter_name);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_layers_keep_sparse_paint_and_return_to_cpu_when_requested() {
        use texturecraft_render::{headless::Headless, readback::Readback};

        let Ok(mut gpu) = Headless::new() else { return };
        let mut s = procedural_stack(1024);
        use texturecraft_engine::core::{Effect, EffectKind, Filter, Mask, MaskBackground, Projection, Source};
        let masked = s.doc.project.texture_sets[0].stack[1];
        let fill_id = s.doc.project.alloc_id();
        let invert_id = s.doc.project.alloc_id();
        let levels_id = s.doc.project.alloc_id();
        let masked = s.doc.project.layers.get_mut(&masked).unwrap();
        let mut mask = Mask::new(MaskBackground::Black);
        mask.effects.push(Effect::new(fill_id, EffectKind::Fill { source: Source::Uniform([0.22; 3]), projection: Projection::default() }));
        mask.effects.push(Effect::new(invert_id, EffectKind::Filter(Filter::Invert)));
        masked.mask = Some(mask);
        masked.content_effects.push(Effect::new(levels_id, EffectKind::Filter(Filter::Clamp { min: 0.05, max: 0.75 })));
        let layer = s.selected_layer().unwrap();
        s.doc.pixels.layer_mut(layer, Channel::BaseColor, (1024, 1024)).set(70, 92, &[65535, 0, 0, 65535]);
        s.doc.pixels.layer_mut(layer, Channel::Height, (1024, 1024)).set(70, 92, &[65535, 65535]);
        execute(&mut s, "tool.material", &json!({"channel":"baseColor", "solo":true, "color":[0.9,0.02,0.02]})).unwrap();

        let mut link = link();
        link.gpu_composite_enabled = true;
        let scene = link.sync(&mut s, None, None, 0, 0.0);
        assert!(!scene.composites.is_empty());
        assert!(scene.tiles.is_empty());
        scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        assert!(!link.compositors[0].failed.load(Ordering::Acquire), "the compute pass ran");
        assert!(!link.display_pending(&s));
        assert!(link.sync(&mut s, None, None, 0, 0.0).composites.is_empty(), "idle frames reuse the material");

        let textures = gpu.renderer.display_textures();
        let (width, height, image) = Readback::blocking(&gpu.device, &gpu.queue, textures[0][0]).unwrap();
        assert_eq!((width, height), (1024, 1024));
        let pixel = ((92 * width + 70) * 4) as usize;
        assert_eq!(&image[pixel..pixel + 4], &[255, 0, 0, 255]);
        assert!(!image[pixel + 4..pixel + 8].starts_with(&[255, 0, 0]), "a one-texel stroke does not spill into the next texel");
        assert_eq!(textures[0][3].width(), 1024, "height has a full-size texture too");

        let dab = Vec2::splat(0.95);
        let dab_pixel = ((972 * width + 972) * 4) as usize;
        let before_dab = image[dab_pixel..dab_pixel + 4].to_vec();
        s.begin_stroke_uv(dab, 1.0).unwrap();
        let scene = link.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
        assert!(!scene.composites.is_empty());
        assert!(scene.tiles.is_empty(), "live layer compositing stays on the GPU");
        scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        assert!(!link.compositors[0].failed.load(Ordering::Acquire));
        let (_, _, live) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][0]).unwrap();
        assert_ne!(&live[dab_pixel..dab_pixel + 4], before_dab);
        assert_eq!(&live[pixel..pixel + 4], &[255, 0, 0, 255], "the regional dispatch leaves distant texels alone");
        assert!(link.sync(&mut s, None, None, 0, FRAME_BUDGET_MS).composites.is_empty(), "an unchanged stroke does not redispatch");
        s.cancel_stroke();
        let restored = link.sync(&mut s, None, None, 0, FRAME_BUDGET_MS);
        restored.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        let (_, _, cancelled) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][0]).unwrap();
        assert_eq!(&cancelled[dab_pixel..dab_pixel + 4], before_dab);
        s.begin_stroke_uv(dab, 1.0).unwrap();
        s.end_stroke().unwrap();
        let mut resumed = false;
        for _ in 0..8 {
            let scene = link.sync(&mut s, None, None, 0, 0.0);
            scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            if !scene.composites.is_empty() {
                resumed = true;
                break;
            }
        }
        assert!(resumed, "the GPU can read committed sparse paint");

        link.prefer_gpu_layers = false;
        let cpu = link.sync(&mut s, None, None, 0, 0.0);
        assert!(cpu.composites.is_empty());
        assert!(link.display_pending(&s) || !cpu.tiles.is_empty());
        link.prefer_gpu_layers = true;
        assert!(!link.sync(&mut s, None, None, 0, 0.0).composites.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_compositing_retries_after_editing_a_failed_material() {
        let mut session = procedural_stack(128);
        let mut link = link();
        link.gpu_composite_enabled = true;
        let first = link.sync(&mut session, None, None, 0, 0.0);
        let failed = first.composites[0].2.clone();
        failed.store(true, Ordering::Release);
        for _ in 0..2 {
            assert!(link.sync(&mut session, None, None, 0, 0.0).composites.is_empty());
            assert_eq!(link.compositor_status(), ("Layers: CPU", GPU_COMPOSITE_ERROR));
        }

        execute(&mut session, "layer.add.fill", &json!({"color": [0.45, 0.15, 0.35]})).unwrap();
        let updated = link.sync(&mut session, None, None, 0, 0.0);
        assert!(!updated.composites.is_empty(), "editing the failed material gives the GPU a fresh attempt");
        failed.store(true, Ordering::Release);
        assert_eq!(link.compositor_status().0, "Layers: GPU", "a stale completion cannot reject the updated material");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_brush_previews_empty_layer_and_survives_switch_to_cpu() {
        use texturecraft_render::readback::Readback;

        let Ok(gpu) = Headless::new() else { return };
        let brush = Arc::new(HeadlessBrush(Mutex::new(gpu)));
        let mut s = Session::new_project("GPU brush", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let layer = s.selected_layer().unwrap();
        execute(&mut s, "tool.material", &json!({"channel": "baseColor", "solo": true, "color": [1.0, 0.0, 0.0]})).unwrap();
        let mut link = link();
        link.gpu_composite_enabled = true;
        link.brush_device = Some(brush.clone());

        let display_at_center = |scene: SceneSync| -> Vec<u8> {
            let Headless { renderer, device, queue, .. } = &mut *brush.0.lock().unwrap();
            let (_, errors) = scene.apply_display(renderer, device, queue);
            assert!(errors.is_empty(), "{errors:?}");
            let (w, h, rgba) = Readback::blocking(device, queue, renderer.display_textures()[0][0]).unwrap();
            let center = ((h / 2 * w + w / 2) * 4) as usize;
            rgba[center..center + 4].to_vec()
        };
        let neutral = display_at_center(link.sync(&mut s, None, None, 0, 0.0));

        s.brush_device = link.brush_device(&s);
        s.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
        assert!(s.device_stroke().is_some());
        assert!(s.display_stroke_pixels(0, None).is_none());
        assert!(s.doc.pixels.layer(layer, Channel::BaseColor).is_none());
        let frame = link.sync(&mut s, None, None, 0, 0.0);
        assert!(!frame.composites.is_empty());
        assert!(frame.tiles.is_empty());
        let painted = display_at_center(frame);
        assert_ne!(painted, neutral);

        s.cancel_stroke();
        assert_eq!(display_at_center(link.sync(&mut s, None, None, 0, 0.0)), neutral);
        s.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
        assert!(s.device_stroke().is_some());
        link.prefer_gpu_layers = false;
        let fallback = link.sync(&mut s, None, None, 0, 0.0);
        assert!(fallback.composites.is_empty());
        assert!(s.device_stroke().is_none());
        let cpu_preview = s.display_stroke_pixels(0, None).unwrap();
        let pixel = cpu_preview.layer(layer, Channel::BaseColor).unwrap().get(64, 64);
        assert!(pixel[0] > 0 && pixel.last().copied().unwrap() > 0);
        s.end_stroke().unwrap();
        assert_eq!(s.doc.pixels.layer(layer, Channel::BaseColor).unwrap().get(64, 64), pixel);
        link.prefer_gpu_layers = true;
        assert_eq!(display_at_center(link.sync(&mut s, None, None, 0, 0.0)), painted);
        s.undo().unwrap();
        assert!(s.doc.pixels.layer(layer, Channel::BaseColor).is_none());
        assert_eq!(display_at_center(link.sync(&mut s, None, None, 0, 0.0)), neutral);
        s.brush_device = link.brush_device(&s);
        s.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
        assert!(s.device_stroke().is_some());
        assert_eq!(display_at_center(link.sync(&mut s, None, None, 0, 0.0)), painted);
        s.end_stroke().unwrap();
        assert_eq!(s.doc.pixels.layer(layer, Channel::BaseColor).unwrap().get(64, 64), pixel);
        assert_eq!(display_at_center(link.sync(&mut s, None, None, 0, 0.0)), painted);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_center(gpu: &Headless, texture: &wgpu::Texture) -> [u8; 4] {
        read_pixel(gpu, texture, texture.width() / 2, texture.height() / 2)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_pixel(gpu: &Headless, texture: &wgpu::Texture, x: u32, y: u32) -> [u8; 4] {
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU test pixel"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout::default() },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |mapped| send.send(mapped).unwrap());
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let pixel = buffer.slice(..).get_mapped_range().unwrap();
        pixel[..4].try_into().unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn check_gpu_mask_stroke(size: u32, gpu: Headless) {
        let brush = Arc::new(HeadlessBrush(Mutex::new(gpu)));
        let mut s = Session::new_project("GPU mask", primitives::plane(1.0, 1), "builtin:plane", None, size).unwrap();
        execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap();
        let layer = s.selected_layer().unwrap();
        execute(&mut s, "mask.add.black", &json!({})).unwrap();
        let mut link = link();
        link.gpu_composite_enabled = true;
        link.brush_device = Some(brush.clone());

        let display = |scene: SceneSync, mask_view: bool, dispatch: bool| {
            assert_eq!(scene.composites.len(), usize::from(dispatch), "unexpected GPU program count");
            if let Some((_, program, _)) = scene.composites.first() {
                assert_eq!(program.is_mask_preview(), mask_view);
            }
            assert!(scene.tiles.is_empty() && scene.aux.is_empty(), "the active set does not upload CPU pixels");
            let gpu = &mut *brush.0.lock().unwrap();
            let (_, errors) = scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            assert!(errors.is_empty(), "{size}²: {errors:?}");
            let texture = if mask_view { gpu.renderer.aux_texture(0).unwrap() } else { gpu.renderer.display_textures()[0][0] };
            read_center(gpu, texture)
        };

        let mut original_material = None;
        for mask_view in [false, true] {
            let aux = mask_view.then_some(AuxView::Mask(layer));
            s.view = if mask_view { ViewMode::Mask(layer) } else { ViewMode::Material };
            let initial = display(link.sync(&mut s, aux, None, 0, 0.0), mask_view, true);
            if mask_view {
                assert_eq!(initial, [0, 0, 0, 255]);
            } else {
                original_material = Some(initial);
            }

            s.brush_device = link.brush_device(&s);
            s.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
            assert!(s.device_stroke().is_some(), "{size}², mask view {mask_view}");
            let painted = display(link.sync(&mut s, aux, None, 0, 0.0), mask_view, true);
            assert_ne!(painted, initial);
            assert!(s.doc.pixels.masks.is_empty(), "live GPU painting keeps the document untouched");
            assert!(link.sync(&mut s, aux, None, 0, 0.0).composites.is_empty(), "stationary brush does not redispatch");

            s.tool.mask_value = 0.35;
            let gray = display(link.sync(&mut s, aux, None, 0, 0.0), mask_view, true);
            assert_ne!(gray, painted, "a gray-value change updates existing coverage");
            assert_ne!(gray, initial);
            s.cancel_stroke();
            assert_eq!(display(link.sync(&mut s, aux, None, 0, 0.0), mask_view, true), initial);

            s.brush_device = link.brush_device(&s);
            s.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();
            assert!(s.device_stroke().is_some());
            assert_eq!(display(link.sync(&mut s, aux, None, 0, 0.0), mask_view, true), gray);
            s.end_stroke().unwrap();
            let committed = s.doc.pixels.masks.values().next().expect("a paint effect must persist the stroke").get(size / 2, size / 2);
            assert!(committed[0] > 0 && committed[1] > 0);
            let final_pixel = display(link.sync(&mut s, aux, None, 0, 0.0), mask_view, true);
            assert!(final_pixel.iter().zip(gray).all(|(a, b)| a.abs_diff(b) <= 1), "{final_pixel:?} vs {gray:?}");
            if mask_view {
                // The material display must still compose when leaving the independent mask view.
                s.view = ViewMode::Material;
                assert_ne!(Some(display(link.sync(&mut s, None, None, 0, 0.0), false, true)), original_material);
            } else {
                s.undo().unwrap();
                assert!(s.doc.pixels.masks.is_empty());
            }
            s.tool.mask_value = 1.0;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_mask_brush_previews_commits_and_cancels_in_both_views() {
        let Ok(gpu) = Headless::new() else { return };
        check_gpu_mask_stroke(128, gpu);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "native 4K/8K GPU regression; requires an adapter with sufficient VRAM"]
    fn smoke_gpu_mask_brush_at_4k_and_8k() {
        for size in [4096, 8192] {
            check_gpu_mask_stroke(size, Headless::new().unwrap());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn quick_mask_tracks_delayed_base_color_and_keeps_its_brush_on_gpu() {
        let Ok(gpu) = Headless::new() else { return };
        let brush = Arc::new(HeadlessBrush(Mutex::new(gpu)));
        let mut s = Session::new_project("Mixed Quick Mask", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        execute(&mut s, "layer.add.fill", &json!({"color": [0.0, 0.5, 0.0]})).unwrap();
        s.update();
        execute(&mut s, "tool.paint", &json!({})).unwrap();
        execute(&mut s, "viewport.quickMask", &json!({"enabled": true})).unwrap();
        execute(&mut s, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
        let mut link = link();
        link.gpu_composite_enabled = true;
        link.brush_device = Some(brush.clone());
        let aux = Some(AuxView::QuickMask);
        let display = |scene: &SceneSync| {
            let gpu = &mut *brush.0.lock().unwrap();
            if let Some((mesh, id)) = &scene.mesh {
                gpu.renderer.set_mesh(&gpu.device, &gpu.queue, mesh, *id);
            }
            let (_, errors) = scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            assert!(errors.is_empty(), "{errors:?}");
            read_pixel(gpu, gpu.renderer.aux_texture(0).unwrap(), 32, 32)
        };
        let mut attempted = link.sync(&mut s, aux, None, 0, 0.0);
        let (_, _, failed) = attempted.composites.pop().expect("material was queued for GPU composition");
        assert!(attempted.composites.is_empty());
        failed.store(true, Ordering::Release);
        display(&attempted);
        drop(attempted);

        let mut first = link.sync(&mut s, aux, None, 0, 0.0);
        assert_eq!(link.compositor_status().0, "Layers: CPU");
        assert!(!first.quick_masks.is_empty());
        assert!(first.tiles.iter().any(|(_, kind, ..)| *kind == DisplayKind::BaseColor));
        // Simulate a frame where CPU composition finished but the BaseColor upload remains queued.
        first.tiles.retain(|(_, kind, ..)| *kind != DisplayKind::BaseColor);
        let before = display(&first);
        let revision = (s.caches[0].revision, s.caches[0].material_revision, s.quick_mask.revision);
        s.caches[0].upload.insert((0, 0));
        let second = link.sync(&mut s, aux, None, 0, 0.0);
        assert_eq!(revision, (s.caches[0].revision, s.caches[0].material_revision, s.quick_mask.revision));
        assert!(second.tiles.iter().any(|(_, kind, ..)| *kind == DisplayKind::BaseColor));
        assert_eq!(second.quick_masks.len(), 1);
        assert_ne!(display(&second), before);

        s.camera.set_angles(0.0, 1.45);
        s.camera.ortho = true;
        s.camera.distance = 1.8;
        s.viewport = Vec2::splat(400.0);
        s.brush_device = link.brush_device(&s);
        assert!(s.begin_stroke(Vec2::new(144.0, 200.0), s.viewport, 1.0).unwrap());
        let live = link.sync(&mut s, aux, None, 0, 0.0);
        assert!(s.quick_mask_stroke(0).is_some_and(|stroke| stroke.device.is_some()));
        display(&live);
        assert!(s.continue_stroke(Vec2::new(256.0, 200.0), 1.0));
        let live = link.sync(&mut s, aux, None, 0, 0.0);
        assert!(s.quick_mask_stroke(0).is_some_and(|stroke| stroke.device.is_some()));
        display(&live);

        // The explicit CPU preference still materializes coverage before the CPU overlay.
        link.prefer_gpu_layers = false;
        let fallback = link.sync(&mut s, aux, None, 0, 0.0);
        assert!(fallback.release_quick_mask && fallback.quick_masks.is_empty() && !fallback.aux.is_empty());
        assert!(s.quick_mask_stroke(0).is_some_and(|stroke| stroke.device.is_none()));
        s.end_stroke().unwrap();
        assert!(s.quick_mask.masks[&0].allocated_count() > 0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn check_gpu_quick_mask(size: u32, gpu: Headless, cpu_fallback: bool) {
        use texturecraft_engine::texel::from_u16;

        let brush = Arc::new(HeadlessBrush(Mutex::new(gpu)));
        let mut s = Session::new_project("GPU Quick Mask", primitives::plane(1.0, 1), "builtin:plane", None, size).unwrap();
        execute(&mut s, "layer.add.fill", &json!({"color": [0.0, 0.5, 0.0]})).unwrap();
        execute(&mut s, "layer.add.paint", &json!({})).unwrap();
        let layer = s.selected_layer().unwrap();
        execute(&mut s, "mask.add.white", &json!({})).unwrap();
        execute(&mut s, "tool.brush", &json!({"size": 15, "hardness": 100, "opacity": 100, "flow": 100})).unwrap();
        execute(&mut s, "tool.material", &json!({"channel": "baseColor", "solo": true, "color": [0.0, 0.0, 1.0]})).unwrap();
        let mut link = link();
        link.gpu_composite_enabled = true;
        link.brush_device = Some(brush.clone());
        let center = Vec2::splat(0.5);
        let outside = Vec2::new(0.8, 0.5);
        let aux = Some(AuxView::QuickMask);
        let display = |scene: SceneSync, inspect_aux: bool| {
            let gpu = &mut *brush.0.lock().unwrap();
            if let Some((mesh, id)) = &scene.mesh {
                gpu.renderer.set_mesh(&gpu.device, &gpu.queue, mesh, *id);
            }
            let (reset, errors) = scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
            assert!(errors.is_empty(), "{size}²: {errors:?}");
            let _ = reset;
            for (ts, size, bytes) in &scene.aux {
                gpu.renderer.upload_aux(&gpu.device, &gpu.queue, *ts, *size, bytes);
            }
            let tex = if inspect_aux { gpu.renderer.aux_texture(0).unwrap() } else { gpu.renderer.display_textures()[0][0] };
            [center, outside].map(|uv| {
                let p = (uv * tex.width() as f32).as_uvec2();
                read_pixel(gpu, tex, p.x, p.y)
            })
        };
        let base = display(link.sync(&mut s, None, None, 0, 0.0), false);
        s.view = ViewMode::Mask(layer);
        let layer_mask = display(link.sync(&mut s, Some(AuxView::Mask(layer)), None, 0, 0.0), true);
        assert_eq!(layer_mask[0], [255; 4]);

        execute(&mut s, "viewport.quickMask", &json!({"enabled": true})).unwrap();
        execute(&mut s, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
        let overlay = link.sync(&mut s, aux, None, 0, 0.0);
        assert_eq!(link.compositor_status().0, "Layers: GPU");
        assert_eq!(overlay.quick_masks.len(), 1);
        assert!(overlay.tiles.is_empty() && overlay.aux.is_empty());
        let blocked = display(overlay, true);
        assert!(blocked[0][0] > base[0][0] && blocked[0][1] < base[0][1]);
        let edit = |s: &mut Session, link: &mut GpuLink, tool: &str| {
            execute(s, tool, &json!({})).unwrap();
            s.brush_device = link.brush_device(s);
            s.begin_stroke_uv(center, 1.0).unwrap();
            assert!(s.quick_mask_stroke(0).is_some_and(|stroke| stroke.device.is_some()));
            let scene = link.sync(s, aux, None, 0, 0.0);
            assert!(scene.composites.is_empty() && scene.tiles.is_empty() && scene.aux.is_empty());
            let preview = display(scene, true);
            s.end_stroke().unwrap();
            assert!(s.doc.pixels.layer(layer, Channel::BaseColor).is_none());
            let committed = display(link.sync(s, aux, None, 0, 0.0), true);
            assert_eq!(preview, committed);
            committed
        };
        let clear = edit(&mut s, &mut link, "tool.paint");
        assert!(clear[0].into_iter().zip(base[0]).all(|(a, b)| a.abs_diff(b) <= 1));
        assert_eq!(clear[1], blocked[1]);
        assert_eq!(edit(&mut s, &mut link, "tool.eraser"), blocked);
        assert_eq!(edit(&mut s, &mut link, "tool.paint"), clear);
        execute(&mut s, "viewport.quickMask.invert", &json!({})).unwrap();
        let inverted = display(link.sync(&mut s, aux, None, 0, 0.0), true);
        assert_eq!(inverted, [blocked[0], clear[0]]);

        if cpu_fallback {
            link.prefer_gpu_layers = false;
            assert!(link.brush_device(&s).is_none());
            let scene = link.sync(&mut s, aux, None, 0, 0.0);
            assert!(scene.quick_masks.is_empty() && !scene.aux.is_empty());
            assert_eq!(link.compositor_status().0, "Layers: CPU");
            display(scene, true);
            link.prefer_gpu_layers = true;
            assert_eq!(display(link.sync(&mut s, aux, None, 0, 0.0), true), inverted);
            assert_eq!(link.compositor_status().0, "Layers: GPU");
        }

        execute(&mut s, "viewport.quickMask.edit", &json!({"editing": false})).unwrap();
        assert_eq!(s.view, ViewMode::Mask(layer));
        assert_eq!(display(link.sync(&mut s, Some(AuxView::Mask(layer)), None, 0, 0.0), true), layer_mask);
        s.view = ViewMode::Material;
        s.target = StackTarget::Content;
        display(link.sync(&mut s, None, None, 0, 0.0), false);
        let paint = |s: &mut Session, link: &mut GpuLink, uv| {
            s.brush_device = link.brush_device(s);
            s.begin_stroke_uv(uv, 1.0).unwrap();
            assert!(s.device_stroke().is_some());
            let scene = link.sync(s, None, None, 0, 0.0);
            assert!(scene.tiles.is_empty());
            let pixels = display(scene, false);
            s.end_stroke().unwrap();
            let _ = display(link.sync(s, None, None, 0, 0.0), false);
            pixels
        };
        assert_eq!(paint(&mut s, &mut link, center)[0], base[0], "inverted Quick Mask blocks the painted center");
        let painted = paint(&mut s, &mut link, outside);
        assert!(painted[1][2] > painted[1][1], "inverted Quick Mask allows painting outside");
        execute(&mut s, "viewport.quickMask", &json!({"enabled": false})).unwrap();
        let unlimited = paint(&mut s, &mut link, center);
        assert!(unlimited[0][2] > unlimited[0][1]);
        let center = s.doc.pixels.layer(layer, Channel::BaseColor).unwrap().get(size / 2, size / 2);
        assert!(from_u16(center[3]) > 0.9);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn quick_mask_edits_and_limits_gpu_strokes_without_switching_material_to_cpu() {
        let Ok(gpu) = Headless::new() else { return };
        check_gpu_quick_mask(128, gpu, true);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "native 4K/8K GPU regression; requires an adapter with sufficient VRAM"]
    fn smoke_gpu_quick_mask_at_4k_and_8k() {
        for size in [4096, 8192] {
            check_gpu_quick_mask(size, Headless::new().unwrap(), false);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn projection_session(size: u32, mask_view: bool) -> (Session, LayerId) {
        let mut session = Session::new_project("GPU projection", primitives::plane(1.0, 1), "builtin:plane", None, size).unwrap();
        session.camera.set_angles(0.0, 1.45);
        session.camera.ortho = true;
        session.camera.distance = 1.8;
        session.viewport = Vec2::splat(400.0);
        if mask_view {
            execute(&mut session, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap();
            execute(&mut session, "mask.add.black", &json!({})).unwrap();
        }
        let layer = session.selected_layer().unwrap();
        if mask_view {
            session.view = ViewMode::Mask(layer);
        }
        // Three authored sRGB bands with distinct colors and alpha, away from sample boundaries.
        let stencil = image::RgbaImage::from_fn(24, 24, |x, _| {
            image::Rgba(match x / 8 {
                0 => [241, 107, 35, 0],
                1 => [204, 24, 32, 128],
                _ => [31, 194, 113, 240],
            })
        });
        let mut png = Vec::new();
        stencil.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let name = session.import_resource_bytes("projection-bands.png", png).unwrap();
        assert!(session.images.get(&name).unwrap().srgb);
        execute(&mut session, "tool.projection", &json!({})).unwrap();
        execute(&mut session, "tool.stencil", &json!({"image": name, "scale": 0.48, "rotation": 15, "center": [0.04, -0.02]})).unwrap();
        execute(&mut session, "tool.material", &json!({"channel": "baseColor", "solo": true, "color": [0.25, 0.8, 1.0], "maskValue": 0.65})).unwrap();
        execute(&mut session, "tool.brush", &json!({"size": 12, "hardness": 100, "opacity": 83, "spacing": 20})).unwrap();
        (session, layer)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn check_gpu_projection_stroke(size: u32, gpu: Headless, cpu_oracle: bool) {
        use texturecraft_engine::texel::TILE;

        let brush = Arc::new(HeadlessBrush(Mutex::new(gpu)));
        let screen = [144.0, 200.0, 256.0].map(|x| Vec2::new(x, 200.0));
        let dab = |session: &mut Session| {
            assert!(session.begin_stroke(screen[0], session.viewport, 1.0).unwrap());
            for point in &screen[1..] {
                assert!(session.continue_stroke(*point, 1.0));
            }
        };
        let texel = |uv: Vec2, extent: u32| {
            let pixel = (uv * extent as f32).as_uvec2();
            (pixel.x.min(extent - 1), pixel.y.min(extent - 1))
        };
        for mask_view in [false, true] {
            let (mut session, layer) = projection_session(size, mask_view);
            let uvs = screen.map(|point| session.pick(point, session.viewport).unwrap().uv);
            let aux = mask_view.then_some(AuxView::Mask(layer));
            let mut link = link();
            link.gpu_composite_enabled = true;
            link.brush_device = Some(brush.clone());
            let display = |scene: SceneSync| {
                assert_eq!(scene.composites.len(), 1);
                assert_eq!(scene.composites[0].1.is_mask_preview(), mask_view);
                assert!(scene.tiles.is_empty() && scene.aux.is_empty());
                let gpu = &mut *brush.0.lock().unwrap();
                if let Some((mesh, id)) = &scene.mesh {
                    gpu.renderer.set_mesh(&gpu.device, &gpu.queue, mesh, *id);
                }
                let (_, errors) = scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
                assert!(errors.is_empty(), "{size}², mask view {mask_view}: {errors:?}");
                let texture = if mask_view { gpu.renderer.aux_texture(0).unwrap() } else { gpu.renderer.display_textures()[0][0] };
                uvs.map(|uv| {
                    let (x, y) = texel(uv, texture.width());
                    read_pixel(gpu, texture, x, y)
                })
            };
            let compare_pixels = |actual: [[u8; 4]; 3], expected: [[u8; 4]; 3]| {
                for (actual, expected) in actual.into_iter().zip(expected) {
                    assert!(
                        actual.into_iter().zip(expected).all(|(gpu, cpu)| gpu.abs_diff(cpu) <= 2),
                        "{size}², mask view {mask_view}: {actual:?} vs {expected:?}"
                    );
                }
            };
            let baseline = display(link.sync(&mut session, aux, None, 0, 0.0));
            session.brush_device = link.brush_device(&session);
            dab(&mut session);
            assert!(session.device_stroke().is_some(), "{size}², mask view {mask_view}");
            let live = display(link.sync(&mut session, aux, None, 0, 0.0));
            assert!(session.doc.pixels.layer(layer, Channel::BaseColor).is_none() && session.doc.pixels.masks.is_empty());
            assert!(live[0].into_iter().zip(baseline[0]).all(|(live, before)| live.abs_diff(before) <= 1), "transparent stencil pixels do not paint");
            if mask_view {
                assert!(live[2][0] > live[1][0] && live[1][0] > live[0][0], "mask coverage follows stencil alpha");
                assert!(live.into_iter().all(|[r, g, b, _]| r == g && r == b));
            } else {
                assert!(live[1][0] > live[1][1] && live[2][1] > live[2][0], "the material takes the stencil's two ink colors");
            }

            session.cancel_stroke();
            assert_eq!(display(link.sync(&mut session, aux, None, 0, 0.0)), baseline);
            session.brush_device = link.brush_device(&session);
            dab(&mut session);
            assert!(session.device_stroke().is_some());
            compare_pixels(display(link.sync(&mut session, aux, None, 0, 0.0)), live);
            session.end_stroke().unwrap();
            let fallback =
                session.log.iter().find(|entry| entry.message.starts_with("GPU stencil readback") || entry.message.starts_with("GPU stroke readback failed"));
            assert!(fallback.is_none(), "projection must commit through GPU readback: {fallback:?}");
            if mask_view {
                assert!(session.doc.pixels.masks.values().any(|pixels| pixels.allocated_count() != 0));
            } else {
                assert!(session.doc.pixels.layer(layer, Channel::BaseColor).is_some_and(|pixels| pixels.allocated_count() != 0));
            }
            let committed = display(link.sync(&mut session, aux, None, 0, 0.0));
            compare_pixels(committed, live);

            if cpu_oracle {
                let (mut cpu, layer) = projection_session(size, mask_view);
                dab(&mut cpu);
                assert!(cpu.device_stroke().is_none());
                cpu.end_stroke().unwrap();
                cpu.update();
                let expected = if mask_view {
                    let rgba = cpu.mask_view_rgba(layer, size).unwrap();
                    uvs.map(|uv| {
                        let (x, y) = texel(uv, size);
                        let i = ((y * size + x) * 4) as usize;
                        rgba[i..i + 4].try_into().unwrap()
                    })
                } else {
                    uvs.map(|uv| {
                        let (x, y) = texel(uv, size);
                        let tile = display_tile_bytes(&cpu.caches[0].composite, &Default::default(), DisplayKind::BaseColor, x / TILE, y / TILE, 0);
                        let i = ((y % TILE * TILE + x % TILE) * 4) as usize;
                        tile[i..i + 4].try_into().unwrap()
                    })
                };
                compare_pixels(live, expected);
                compare_pixels(committed, expected);
            }
            session.undo().unwrap();
            assert!(session.doc.pixels.layer(layer, Channel::BaseColor).is_none() && session.doc.pixels.masks.is_empty());
            assert_eq!(display(link.sync(&mut session, aux, None, 0, 0.0)), baseline);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn gpu_projection_previews_cancels_commits_and_undoes_in_both_views() {
        let Ok(gpu) = Headless::new() else { return };
        check_gpu_projection_stroke(128, gpu, true);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "native 4K/8K GPU regression; requires an adapter with sufficient VRAM"]
    fn smoke_gpu_projection_at_4k_and_8k() {
        for size in [4096, 8192] {
            check_gpu_projection_stroke(size, Headless::new().unwrap(), false);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "GPU benchmark"]
    fn bench_gpu_layer_stack() {
        use std::time::Instant;
        use texturecraft_render::headless::Headless;
        let mut h = Headless::new().unwrap();
        for size in [1024, 4096, 8192] {
            let mut s = procedural_stack(size);
            let program = CompositeProgram::compile(&s.doc.project, &s.doc.pixels, 0).unwrap();
            h.renderer.set_texture_sets(&h.device, &h.queue, &[((size, size), 0)], 1);
            for pass in ["first", "warm"] {
                let t = Instant::now();
                h.renderer.composite_on_gpu(&h.device, &h.queue, 0, &program).unwrap();
                h.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                println!("[{size}²] {} GPU compositor, {pass}: {:?}", h.adapter_name, t.elapsed());
            }
            let t = Instant::now();
            s.update();
            println!("[{size}²] CPU compositor: {:?}", t.elapsed());
        }
    }

    /// Input to the first rendered pixel of the new color. Readback synchronizes every
    /// offscreen frame; window-system presentation and monitor latency are excluded.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "GPU benchmark"]
    fn bench_brush_upload_latency() {
        use texturecraft_render::{Shade, ViewKind, headless::Headless};
        let mut s = Session::new_project("latency", primitives::plane(1.0, 1), "builtin:plane", None, 1024).unwrap();
        execute(&mut s, "tool.brush", &json!({"size": 3, "hardness": 100})).unwrap();
        let mut gpu = link();
        let mut h = Headless::new().unwrap();
        h.renderer.set_mesh(&h.device, &h.queue, &s.mesh, s.mesh_id);
        h.renderer.set_texture_sets(&h.device, &h.queue, &[((1024, 1024), 0)], 1);
        let uv = Vec2::new(15.5, 5.5) / 16.0;
        let frame = Frame {
            view_id: 2,
            size: [256, 256],
            kind: ViewKind::Uv { ts: 0, zoom: 2.0 / 0.92, pan: [(1.0 - 2.0 * uv.x) * 2.0, (2.0 * uv.y - 1.0) * 2.0], rotation: 0.0 },
            camera: s.camera,
            shade: Shade { view_mode: 1, channel: 0, ..Default::default() },
            visible: vec![true],
            overlay: vec![],
        };
        let mut draw = |s: &mut Session| {
            let scene = gpu.sync(s, None, None, 0, FRAME_BUDGET_MS);
            for (ts, kind, x, y, data) in &scene.tiles {
                h.renderer.upload_display_tile(&h.device, &h.queue, *ts, *kind, *x, *y, data, scene.tile_shift);
            }
            let (width, height, pixels) = h.render(&frame).unwrap();
            let offset = ((height / 2 * width + width / 2) * 4) as usize;
            [pixels[offset], pixels[offset + 1]]
        };
        s.update();
        while !s.caches[0].upload.is_empty() {
            draw(&mut s);
        }
        let all = texturecraft_engine::compose::all_tiles(1024, 1024);
        for backlog in [false, true] {
            let mut elapsed = Vec::new();
            let mut frames = Vec::new();
            for pass in 0..36 {
                let red = pass % 2 == 0;
                let color = if red { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
                execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": color})).unwrap();
                if backlog {
                    s.caches[0].upload.extend(all.iter().copied());
                }
                let start = std::time::Instant::now();
                s.begin_stroke_uv(uv, 1.0).unwrap();
                let mut count = 0;
                loop {
                    let [r, g] = draw(&mut s);
                    count += 1;
                    if (red && r.saturating_sub(g) > 64) || (!red && g.saturating_sub(r) > 64) {
                        break;
                    }
                    assert!(count < 6, "brush pixel must change, got {r}, {g} at pass {pass}");
                }
                if pass >= 4 {
                    elapsed.push(start.elapsed());
                    frames.push(count);
                }
                s.end_stroke().unwrap();
                s.update();
                while !s.caches[0].upload.is_empty() {
                    draw(&mut s);
                }
            }
            elapsed.sort_unstable();
            frames.sort_unstable();
            println!("backlog={backlog}, 32 dabs: {} frames at median, input to GPU readback P50 {:.2?}, P95 {:.2?}", frames[15], elapsed[15], elapsed[30]);
        }
    }

    /// Time the UI's actual tile packing, excluding GPU submission and presentation.
    #[test]
    #[ignore = "benchmark"]
    fn bench_viewport_upload_preparation() {
        let mut gpu = link();
        let mut s = session(true);
        s.update();
        for shift in [0, 1, 2] {
            let mut times = Vec::new();
            let mut payload = 0;
            for pass in 0..105 {
                s.caches[0].upload = [(0, 0), (1, 0), (0, 1), (1, 1)].into();
                let start = std::time::Instant::now();
                let scene = gpu.sync(&mut s, None, None, shift, 0.0);
                let elapsed = start.elapsed();
                payload = scene.tiles.iter().map(|tile| tile.4.len()).sum::<usize>();
                std::hint::black_box(scene);
                if pass >= 5 {
                    times.push(elapsed);
                }
            }
            times.sort_unstable();
            println!("display shift {shift}: {payload} staging bytes, 100 frames, packing P50 {:.2?} P95 {:.2?}", times[49], times[94]);
        }
    }

    #[test]
    fn removed_texture_sets_release_kind_records() {
        let mut gpu = link();
        for texture_set in 0..64 {
            gpu.sent.insert(texture_set, BTreeSet::from([DisplayKind::Emissive]));
        }
        gpu.sync(&mut session(false), None, None, 0, 0.0);

        assert_eq!(gpu.sent.len(), 1);
        assert!(gpu.sent.contains_key(&0));
        eprintln!("sent records: 64 -> {}", gpu.sent.len());
    }
}
