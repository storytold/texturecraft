//! Bridge between the session and the wgpu renderer: per-frame scene sync (mesh, texture set
//! layout, dirty tiles, environment) and the egui paint callback that renders and blits a view.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use rayon::prelude::*;
use texturecraft_engine::Session;
use texturecraft_engine::compose::{DisplayKind, ImageRes, display_tile_bytes, kinds_for};
use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::geom::{Mesh, glam::Vec2};
use texturecraft_render::gpu_composite::{CompositeProgram, CompositeRegion};
use texturecraft_render::{BakeVisualization, CageGeometry, Frame, Renderer, wgpu};

use crate::bake_scene::{BakeSceneLink, CageUpload};
use crate::i18n::n_;

/// Content shown through the aux texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AuxView {
    MeshMap(BakedMap),
    Mask(texturecraft_engine::core::LayerId),
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
    composite: Option<(CompositeProgram, Arc<AtomicBool>)>,
    trace_composite: Option<(f64, Arc<AtomicBool>)>,
    pub(crate) compositor_changed: bool,
    pub tiles: Vec<(usize, DisplayKind, u32, u32, Vec<u8>)>,
    pub aux: Vec<(usize, u32, Vec<u8>)>,
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

impl SceneSync {
    fn apply_display(&self, renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) -> (Vec<usize>, Option<String>) {
        let mut reset = Vec::new();
        if let Some((sets, material_count, shift)) = &self.sets {
            renderer.set_display_shift(*shift);
            reset = renderer.set_texture_sets(device, queue, sets, *material_count);
        }
        for &ts in &self.cleared_overviews {
            renderer.seed_display(device, queue, ts, (1, 1), &[]);
            reset.push(ts);
        }
        let error = self.composite.as_ref().and_then(|(program, failed)| {
            let error = renderer.composite_on_gpu(device, queue, 0, program).err()?;
            log::warn!("Using CPU composition after a GPU error: {error}");
            failed.store(true, Ordering::Release);
            Some(error)
        });
        if let Some((ts, overview)) = &self.overview {
            renderer.seed_display(device, queue, *ts, overview.size, &overview.maps);
        }
        for (texture_set, kind, tile_x, tile_y, bytes) in &self.tiles {
            renderer.upload_display_tile(device, queue, *texture_set, *kind, *tile_x, *tile_y, bytes, self.tile_shift);
        }
        (reset, error)
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
    gpu_composite_enabled: bool,
    pub(crate) prefer_gpu_layers: bool,
    pub(crate) material_start_preview: bool,
    cpu_reason: &'static str,
    gpu_composite_current: Option<CompositeKey>,
    gpu_display_stroke: Option<u64>,
    gpu_composite_rejected: Option<(CompositeKey, &'static str)>,
    gpu_composite_failed: Arc<AtomicBool>,
    gpu_composite_in_flight: Arc<AtomicBool>,
    logged_compositor: Option<(u64, &'static str, &'static str)>,
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
        gpu_composite_enabled: !cfg!(target_arch = "wasm32")
            && info.device_type != wgpu::DeviceType::Cpu
            && info.backend != wgpu::Backend::Gl
            && std::env::var("TEXTURECRAFT_GPU_COMPOSITE").as_deref() != Ok("0"),
        prefer_gpu_layers: true,
        material_start_preview: true,
        cpu_reason: n_("Waiting for the first material update"),
        gpu_composite_current: None,
        gpu_display_stroke: None,
        gpu_composite_rejected: None,
        gpu_composite_failed: Arc::default(),
        gpu_composite_in_flight: Arc::default(),
        logged_compositor: None,
        cpu_composite_started: None,
        cpu_padding_pending: false,
        bake_scene: BakeSceneLink::default(),
    }
}

impl GpuLink {
    /// What the GPU side reported since the last call (see [`GpuReports`]).
    pub fn take_reports(&self) -> GpuReports {
        std::mem::take(&mut *self.reports.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Whether bake mode's visualization is still being worked out off the UI thread
    pub fn bake_busy(&self) -> bool {
        self.bake_scene.busy()
    }

    /// CPU display work can stay deferred while the current detail textures are GPU-resident.
    pub(crate) fn display_pending(&self, s: &Session) -> bool {
        if self.prefer_gpu_layers
            && self.gpu_composite_current.is_some_and(|key| key.mesh == s.mesh_id && key.revision == s.revision)
            && !self.gpu_composite_failed.load(Ordering::Acquire)
        {
            return false;
        }
        s.pending_tiles() > 0 || s.caches.iter().any(|cache| !cache.upload.is_empty())
    }

    pub(crate) fn gpu_composite_pending(&self) -> bool {
        self.gpu_composite_in_flight.load(Ordering::Acquire)
    }

    pub(crate) fn cpu_padding_pending(&self) -> bool {
        self.cpu_padding_pending
    }

    pub(crate) fn compositor_status(&self) -> (&'static str, &'static str) {
        if self.gpu_composite_failed.load(Ordering::Acquire) {
            return (n_("Layers: CPU"), n_("GPU compositing reported a device error. See the app log for details."));
        }
        if self.gpu_composite_current.is_some() {
            (n_("Layers: GPU"), n_("The graphics card composes this material."))
        } else {
            (n_("Layers: CPU"), self.cpu_reason)
        }
    }

    fn log_compositor(&mut self, session: &mut Session, out: &mut SceneSync, gpu: bool, started: f64) {
        let (label, reason) = self.compositor_status();
        let state = (session.mesh_id, label, reason);
        if self.logged_compositor == Some(state) && out.sets.is_none() {
            return;
        }
        self.logged_compositor = Some(state);
        out.compositor_changed = true;
        let (w, h) = session.caches.get(session.ts()).map_or((0, 0), |cache| cache.size);
        session.log.performance(format!("{label} at {w}×{h}: {reason}"));
        if gpu {
            self.cpu_composite_started = None;
            if out.composite.is_some() {
                self.gpu_composite_in_flight = Arc::new(AtomicBool::new(true));
                out.trace_composite = Some((started, self.gpu_composite_in_flight.clone()));
            }
        } else if !cfg!(target_arch = "wasm32") {
            self.cpu_composite_started = Some(started);
        }
    }

    fn plan_gpu_composite(&mut self, session: &mut Session, shift: u32, interactive: bool, out: &mut SceneSync) -> bool {
        self.cpu_reason = if !self.prefer_gpu_layers {
            n_("CPU is selected for layer compositing in Settings.")
        } else if !self.gpu_composite_enabled {
            n_("GPU compositing is unavailable on this device.")
        } else if interactive || session.solo.is_some() {
            n_("The active view uses CPU compositing.")
        } else if session.stroke_active() && session.pending_display_stroke(0).is_none() {
            n_("This painting tool uses CPU compositing.")
        } else if session.caches.len() != 1 {
            n_("GPU compositing currently supports a single texture set.")
        } else if session.doc.baked.get(&0).is_some_and(|maps| maps.maps.contains_key(&BakedMap::AmbientOcclusion) || maps.maps.contains_key(&BakedMap::Normal))
        {
            n_("This material uses baked normal or ambient occlusion maps.")
        } else {
            ""
        };
        if !self.cpu_reason.is_empty() || self.gpu_composite_failed.load(Ordering::Acquire) {
            return false;
        }
        let Some(cache) = session.caches.first() else { return false };
        let padding = cache.dilation.mixed_coverage();
        if !cache.dilation.fully_covered() && padding.is_none() {
            self.cpu_reason = n_("The texture set has no covered UV texels.");
            return false;
        }

        let key = CompositeKey { mesh: session.mesh_id, revision: session.revision, size: cache.size, shift };
        let stroke = session.pending_display_stroke(0);
        let stroke_id = stroke.map(|(id, _)| id);
        let current = self.gpu_composite_current == Some(key);
        let unchanged_stroke = self.gpu_display_stroke == stroke_id;
        if current && unchanged_stroke && stroke.is_none_or(|(_, tiles)| tiles.is_empty()) {
            return true;
        }
        if stroke.is_none()
            && let Some((_, reason)) = self.gpu_composite_rejected.filter(|(rejected, _)| *rejected == key)
        {
            self.cpu_reason = reason;
            return false;
        }
        let changed_tiles = if current
            && (unchanged_stroke || self.gpu_display_stroke.is_none())
            && let Some((_, tiles)) = stroke
        {
            Some(tiles)
        } else if stroke.is_none()
            && let Some(previous_id) = self.gpu_display_stroke
            && let Some(previous) = self.gpu_composite_current.filter(|old| old.mesh == key.mesh && old.size == key.size && old.shift == key.shift)
        {
            session.finished_display_stroke(0, previous_id, previous.revision)
        } else {
            None
        };
        let region = changed_tiles
            .and_then(|tiles| CompositeRegion::from_tiles(tiles.iter().copied()))
            .map(|r| if padding.is_some() { r.expanded(texturecraft_engine::texel::TILE) } else { r });
        if changed_tiles.is_some() && region.is_none() {
            self.gpu_composite_current = Some(key);
            self.gpu_display_stroke = stroke_id;
            return true;
        }
        let preview = session.display_stroke_pixels(0, region.map(|r| r.expanded(texturecraft_engine::texel::TILE).bounds()));
        let pixels = preview.as_ref().unwrap_or(&session.doc.pixels);
        let mut program = match CompositeProgram::try_compile_region(&session.doc.project, pixels, 0, region) {
            Ok(program) => program,
            Err(reason) => {
                self.cpu_reason = reason;
                if stroke_id.is_none() {
                    self.gpu_composite_rejected = Some((key, reason));
                }
                return false;
            }
        };
        if let Some(padding) = padding {
            program.set_coverage(padding, texturecraft_engine::session::DISPLAY_PADDING);
        }
        out.composite = Some((program, self.gpu_composite_failed.clone()));
        self.gpu_composite_current = Some(key);
        self.gpu_display_stroke = stroke_id;
        session.clear_display_stroke_tiles();
        self.pending_overviews.remove(&0);
        self.active_overviews.remove(&0);
        self.sent.entry(0).or_default().extend(DisplayKind::ALL);
        true
    }

    /// Compositing progress (0..1) while tiles wait to be composited.
    pub fn composite_progress(&self, s: &Session) -> Option<f32> {
        let pending = if self.gpu_composite_current.is_some() { 0 } else { s.pending_tiles() };
        (pending > 0 && self.composite_total > 0).then(|| 1.0 - pending as f32 / self.composite_total as f32)
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
        let pending = session.pending_tiles();
        self.composite_total = if pending == 0 { 0 } else { self.composite_total.max(pending) };
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
        if resend || sets != self.sets || shift != self.shift || out.mesh.is_some() {
            self.gpu_composite_current = None;
            self.gpu_display_stroke = None;
            self.gpu_composite_failed = Arc::default();
            self.pending_overviews.retain(|&ts| ts < sets.len());
            for (ts, layout) in sets.iter().enumerate() {
                if resend || out.mesh.is_some() || self.shift != shift || self.sets.get(ts) != Some(layout) {
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
        let interactive = aux_map.is_some() || bake.is_some();
        if self.plan_gpu_composite(session, shift, interactive, &mut out) {
            self.log_compositor(session, &mut out, true, start_ms);
            return out;
        }
        self.gpu_display_stroke = None;
        if self.gpu_composite_current.take().is_some()
            && let Some(cache) = session.caches.first_mut()
        {
            cache.queue_ready_uploads();
        }
        self.log_compositor(session, &mut out, false, start_ms);

        let count = session.caches.len();
        let active = session.ts();
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Start at most one dense map at a time, with the active material first.
            let (before, from_active) = session.caches.split_at_mut(active.min(count));
            self.cpu_padding_pending = from_active
                .iter_mut()
                .chain(before)
                .filter(|cache| cache.size.0.max(cache.size.1) > 1024 && (!cache.dirty.is_empty() || !cache.settle.is_empty()))
                .any(|cache| !cache.dilation.prepare_on_worker());
        }
        // At most one small full-atlas evaluation per frame, active material first.
        let next_overview = if self.pending_overviews.remove(&active) { Some(active) } else { self.pending_overviews.pop_first() };
        if let Some(ts) = next_overview
            && !(ts == active && demand.has_brush())
            && !session.caches[ts].dirty.is_empty()
            && let Some(overview) = session.material_overview(ts, 128)
        {
            self.sent.entry(ts).or_default().extend(overview.maps.iter().map(|(kind, _)| *kind));
            self.active_overviews.insert(ts);
            out.overview = Some((ts, overview));
        }
        if !self.cpu_padding_pending && (out.overview.is_none() || demand.has_brush()) {
            session.update_for_view(COMPOSITE_BUDGET_MS.min(time_left_ms(0)).max(MIN_SLICE_MS), crate::now_ms, demand);
        }
        let first = self.upload_cursor.take().filter(|&set| set < count && out.mesh.is_none()).unwrap_or(active);
        let rotation = (first..count).chain(0..first.min(count));
        // While painting, share the first chunk with the next waiting set. An overloaded
        // frame still submits at most one chunk in total, independent of material count.
        let boost = (session.stroke_active() || demand.focused()) && active != first && session.caches.get(active).is_some_and(|c| !c.upload.is_empty());
        let mut chunk_size = if boost { UPLOAD_CHUNK / 2 } else { UPLOAD_CHUNK };
        let order = boost.then_some((active, true)).into_iter().chain(rotation.map(|set| (set, false)));
        'uploads: for (texture_set, boosted) in order {
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
        if !self.display_pending(session)
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
                if let Some((texture_set, _, _)) = session.doc.project.locate(id) {
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

                    let key = (format!("quickmask{}", session.quick_mask.revision), cache.revision);
                    if self.aux.get(&texture_set) == Some(&key) {
                        continue;
                    }

                    let resolution = texturecraft_render::aux_size(cache.size);
                    if let Some(rgba) = session.quick_mask_rgba(texture_set, resolution) {
                        out.aux.push((texture_set, resolution, rgba));
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
        if let Some(sync) = self.sync.get_mut().unwrap_or_else(PoisonError::into_inner) {
            if let Some((_, pending)) = &sync.trace_composite {
                pending.store(false, Ordering::Release);
            }
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
        if let Some(sync) = sync {
            if let Some((env, image)) = &sync.env {
                gpu.renderer.set_environment(device, queue, env, image.as_deref());
            }
            if let Some((mesh, id)) = &sync.mesh {
                gpu.renderer.set_mesh(device, queue, mesh, *id);
            }
            let (reset, composite_error) = sync.apply_display(&mut gpu.renderer, device, queue);
            if let Some(error) = composite_error {
                if let Some((_, pending)) = &sync.trace_composite {
                    pending.store(false, Ordering::Release);
                }
                gpu.reports.lock().unwrap_or_else(PoisonError::into_inner).performance.push((sync.mesh_id, format!("GPU layer submission failed: {error}")));
                gpu.ctx.request_repaint();
            } else if let Some((started, pending)) = &sync.trace_composite {
                let (started, pending, mesh) = (*started, pending.clone(), sync.mesh_id);
                let cpu_ms = crate::now_ms() - started;
                let (reports, ctx) = (gpu.reports.clone(), gpu.ctx.clone());
                queue.on_submitted_work_done(move || {
                    let elapsed = crate::now_ms() - started;
                    reports.lock().unwrap_or_else(PoisonError::into_inner).performance.push((
                        mesh,
                        format!("GPU layer work completed in {elapsed:.0} ms since viewport update started (CPU preparation and submission {cpu_ms:.0} ms)"),
                    ));
                    pending.store(false, Ordering::Release);
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
            gpu_composite_enabled: false,
            prefer_gpu_layers: true,
            material_start_preview: true,
            cpu_reason: n_("Waiting for the first material update"),
            gpu_composite_current: None,
            gpu_display_stroke: None,
            gpu_composite_rejected: None,
            gpu_composite_failed: Arc::default(),
            gpu_composite_in_flight: Arc::default(),
            logged_compositor: None,
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
        assert!(!s.tile_demand(None).focused(), "the released dab's priority ends after its upload");
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
        assert!(session.pending_tiles() > 0);

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
        assert!(scene.composite.is_some());
        scene.apply_display(&mut headless.renderer, &headless.device, &headless.queue);
        assert!(!link.gpu_composite_failed.load(Ordering::Acquire));
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

        execute(&mut session, "effect.add.filter", &json!({"target":"content", "filter":"blur", "radius":8})).unwrap();
        let scene = link.sync(&mut session, None, None, 0, 0.0);
        assert!(scene.composite.is_none());
        assert_eq!(link.compositor_status().0, "Layers: CPU");
        assert!(session.log.iter().any(|entry| entry.message.contains("A layer effect uses CPU compositing.")));
        assert!(session.memory().geometry > geometry_bytes, "only the CPU path allocates the dense padding map");
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
        assert!(scene.composite.is_some());
        assert!(scene.tiles.is_empty());
        scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        assert!(!link.gpu_composite_failed.load(Ordering::Acquire), "the compute pass ran");
        assert!(!link.display_pending(&s));
        assert!(link.sync(&mut s, None, None, 0, 0.0).composite.is_none(), "idle frames reuse the material");

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
        assert!(scene.composite.is_some());
        assert!(scene.tiles.is_empty(), "live layer compositing stays on the GPU");
        scene.apply_display(&mut gpu.renderer, &gpu.device, &gpu.queue);
        assert!(!link.gpu_composite_failed.load(Ordering::Acquire));
        let (_, _, live) = Readback::blocking(&gpu.device, &gpu.queue, gpu.renderer.display_textures()[0][0]).unwrap();
        assert_ne!(&live[dab_pixel..dab_pixel + 4], before_dab);
        assert_eq!(&live[pixel..pixel + 4], &[255, 0, 0, 255], "the regional dispatch leaves distant texels alone");
        assert!(link.sync(&mut s, None, None, 0, FRAME_BUDGET_MS).composite.is_none(), "an unchanged stroke does not redispatch");
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
            if scene.composite.is_some() {
                resumed = true;
                break;
            }
        }
        assert!(resumed, "the GPU can read committed sparse paint");

        link.prefer_gpu_layers = false;
        let cpu = link.sync(&mut s, None, None, 0, 0.0);
        assert!(cpu.composite.is_none());
        assert!(link.display_pending(&s) || !cpu.tiles.is_empty());
        link.prefer_gpu_layers = true;
        assert!(link.sync(&mut s, None, None, 0, 0.0).composite.is_some());
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
