//! Bridge between the session and the wgpu renderer: per-frame scene sync (mesh, texture set
//! layout, dirty tiles, environment) and the egui paint callback that renders and blits a view.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use rayon::prelude::*;
use texturecraft_engine::Session;
use texturecraft_engine::compose::{DisplayKind, ImageRes, kinds_for, tile_bytes};
use texturecraft_engine::core::{BakedMap, Channel, LayerId};
use texturecraft_engine::geom::Mesh;
use texturecraft_render::{BakeVisualization, CageGeometry, Frame, Renderer, wgpu};

use crate::bake_scene::{BakeSceneLink, CageUpload};

/// Content shown through the aux texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AuxView {
    MeshMap(BakedMap),
    Mask(LayerId),
    /// The quick mask being edited, over the base color.
    QuickMask,
    /// The project image of the view picker's "Other..." (`Session::view_image`)
    Image,
    /// The image a color selection reads as its ID mask on its texture set, the ID mesh map on the others (its pick mode)
    IdMask(LayerId),
}

/// A texture set's texels across and down, and its mesh material.
pub type SetLayout = ((u32, u32), usize);

/// The low poly's hard edges without a UV seam (vertex pairs) per material
pub type MissingSeamEdges = Vec<Vec<(u32, u32)>>;

/// What the renderer must catch up with before drawing.
#[derive(Default)]
pub struct SceneSync {
    pub mesh: Option<(Arc<Mesh>, u64)>,
    /// (texels across and down, material) per texture set, the mesh material count and the
    /// viewport texture resolution (display shift).
    pub sets: Option<(Vec<SetLayout>, usize, u32)>,
    /// The environment name and, for `resource:<name>`, its decoded image.
    pub env: Option<(String, Option<Arc<ImageRes>>)>,
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

/// Lives in egui's callback resources.
pub struct GpuState {
    pub renderer: Renderer,
    /// Raised when texture-set textures were recreated: the UI must upload every tile again.
    pub reset: Arc<AtomicBool>,
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
    /// Weak identity of the last uploaded ID mask image.
    aux_id_mask: Option<Weak<ImageRes>>,
    /// The display kinds sent to the GPU per set: kinds the set has no channels for are not
    /// converted, unless an earlier upload gave them content that must be overwritten. Only the
    /// renderer recreating a set's textures ([`GpuState::recreated`]) replaces its entry with
    /// kinds uploaded since recreation. A set that keeps its size retains its textures.
    sent: HashMap<usize, BTreeSet<DisplayKind>>,
    pub adapter: String,
    /// Most tiles waiting to be composited since compositing last caught up (progress).
    composite_total: usize,
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
    rs.renderer.write().callback_resources.insert(GpuState {
        renderer,
        reset: reset.clone(),
        recreated: recreated.clone(),
        reports: reports.clone(),
        ctx: ctx.clone(),
    });
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
        aux_id_mask: None,
        sent: HashMap::new(),
        adapter: format!("{} ({:?})", info.name, info.backend),
        composite_total: 0,
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

    /// Compositing progress (0..1) while tiles wait to be composited.
    pub fn composite_progress(&self, s: &Session) -> Option<f32> {
        let pending = s.pending_tiles();
        (pending > 0 && self.composite_total > 0).then(|| 1.0 - pending as f32 / self.composite_total as f32)
    }

    /// Collect everything the GPU needs this frame, running the compositor first: `bake` is what bake mode's 3D view draws besides the low poly, `spent_ms` the time this frame already took
    pub fn sync(&mut self, session: &mut Session, aux_map: Option<AuxView>, bake: Option<BakeVisualization>, shift: u32, spent_ms: f64) -> SceneSync {
        let start_ms = crate::now_ms();
        let mut queued = 0usize;
        let time_left_ms = |queued: usize| FRAME_BUDGET_MS - spent_ms - (crate::now_ms() - start_ms) - queued as f64 * UPLOAD_MS_PER_TILE;
        let pending = session.pending_tiles();
        self.composite_total = if pending == 0 { 0 } else { self.composite_total.max(pending) };
        session.update_for(COMPOSITE_BUDGET_MS.min(time_left_ms(0)).max(MIN_SLICE_MS), crate::now_ms);

        let mut out = SceneSync::default();
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
            self.aux_id_mask = None;
        }

        let sets: Vec<SetLayout> = session.doc.project.texture_sets.iter().map(|texture_set| (texture_set.size(), texture_set.material)).collect();
        if sets != self.sets || shift != self.shift || out.mesh.is_some() {
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

        if self.reset.swap(false, Ordering::AcqRel) {
            for cache in &mut session.caches {
                cache.upload.extend(texturecraft_engine::compose::all_tiles(cache.size.0, cache.size.1));
            }
            self.aux.clear();
        }
        for (texture_set, kinds) in std::mem::take(&mut *self.recreated.lock().unwrap_or_else(PoisonError::into_inner)) {
            if texture_set < self.sets.len() {
                self.sent.insert(texture_set, kinds);
            }
        }

        for texture_set in 0..session.caches.len() {
            let tiles = session.take_uploads(texture_set);
            if tiles.is_empty() {
                continue;
            }

            let mut done = 0;
            if let Some(cache) = session.caches.get(texture_set) {
                let empty = Default::default();
                let baked = session.doc.baked.get(&texture_set).unwrap_or(&empty);
                let composite = &cache.composite;
                let channels: Vec<Channel> = composite.channels.keys().copied().collect();
                let sent = self.sent.entry(texture_set).or_default();
                let kinds: Vec<DisplayKind> =
                    DisplayKind::ALL.into_iter().filter(|kind| sent.contains(kind) || kinds_for(&channels).any(|needed| needed == *kind)).collect();
                sent.extend(kinds.iter().copied());

                // In chunks while the frame has time (at least one chunk); the rest waits
                for chunk in tiles.chunks(UPLOAD_CHUNK) {
                    let bytes: Vec<_> = chunk
                        .par_iter()
                        .flat_map_iter(|&(tile_x, tile_y)| kinds.iter().map(move |&kind| (kind, tile_x, tile_y)))
                        .map(|(kind, tile_x, tile_y)| (texture_set, kind, tile_x, tile_y, tile_bytes(composite, baked, kind, tile_x, tile_y)))
                        .collect();
                    queued += bytes.len();
                    out.tiles.extend(bytes);
                    done += chunk.len();
                    if time_left_ms(queued) < MIN_SLICE_MS {
                        break;
                    }
                }
            }
            if let (Some(rest), Some(cache)) = (tiles.get(done..), session.caches.get_mut(texture_set)) {
                cache.upload.extend(rest.iter().copied());
            }
        }

        if self.aux_image.as_ref().is_some_and(|image| image.strong_count() == 0) {
            self.aux_image = None;
            self.aux.retain(|_, (view, _)| view != "image");
        }

        if self.aux_id_mask.as_ref().is_some_and(|image| image.strong_count() == 0) {
            self.aux_id_mask = None;
        }

        match aux_map {
            Some(AuxView::MeshMap(map)) => self.sync_mesh_maps(session, map, None, &mut out),
            Some(AuxView::IdMask(effect)) => {
                let shown = self.sync_id_mask(session, effect, &mut out);
                self.sync_mesh_maps(session, BakedMap::Id, shown, &mut out);
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

    /// Queues mesh map `map` for every texture set but `skip` whose aux texture shows something else
    fn sync_mesh_maps(&mut self, session: &Session, map: BakedMap, skip: Option<usize>, out: &mut SceneSync) {
        for texture_set in 0..session.caches.len() {
            if skip == Some(texture_set) {
                continue;
            }

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

    /// Queues the image color selection `effect` reads as its ID mask on its texture set; the texture set it shows on, None when it reads the ID mesh map
    fn sync_id_mask(&mut self, session: &Session, effect: LayerId, out: &mut SceneSync) -> Option<usize> {
        let Some((texture_set, image)) = session.id_mask_image(effect) else {
            self.aux_id_mask = None;
            return None;
        };

        let cache = session.caches.get(texture_set)?;
        let resolution = texturecraft_render::aux_size(cache.size);
        let key = (format!("idMask{}:{resolution}", effect.0), Arc::as_ptr(image) as u64);
        let same_image = self.aux_id_mask.as_ref().is_some_and(|previous| std::ptr::eq(previous.as_ptr(), Arc::as_ptr(image)));
        if (!same_image || self.aux.get(&texture_set) != Some(&key))
            && let Some(rgba) = session.id_mask_view_rgba(effect, resolution)
        {
            out.aux.push((texture_set, resolution, rgba));
            self.aux.insert(texture_set, key);

            // A weak owner keeps the address unique without retaining the decoded pixels
            self.aux_id_mask = Some(Arc::downgrade(image));
        }

        Some(texture_set)
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
        if let Some(sync) = sync {
            if let Some((env, image)) = &sync.env {
                gpu.renderer.set_environment(device, queue, env, image.as_deref());
            }
            if let Some((mesh, id)) = &sync.mesh {
                gpu.renderer.set_mesh(device, queue, mesh, *id);
            }
            let mut reset = Vec::new();
            if let Some((sets, material_count, shift)) = &sync.sets {
                gpu.renderer.set_display_shift(*shift);
                reset = gpu.renderer.set_texture_sets(device, queue, sets, *material_count);
            }
            for (texture_set, kind, tile_x, tile_y, bytes) in &sync.tiles {
                gpu.renderer.upload_tile(device, queue, *texture_set, *kind, *tile_x, *tile_y, bytes);
            }
            record_recreated_uploads(&mut gpu.recreated.lock().unwrap_or_else(PoisonError::into_inner), &reset, &sync);
            if !reset.is_empty() {
                gpu.reset.store(true, Ordering::Release);
            }
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
mod tests {
    use super::*;
    use serde_json::json;
    use texturecraft_engine::execute;
    use texturecraft_engine::geom::primitives;

    fn link() -> GpuLink {
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
            aux_id_mask: None,
            sent: HashMap::new(),
            adapter: String::new(),
            composite_total: 0,
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

        let replacement = Arc::new(ImageRes::new(1, 1, vec![[1.0, 0.0, 0.0, 1.0]], true));
        s.images.insert(name.clone(), replacement);
        execute(&mut s, "viewport.view", &json!({"mode": "image", "image": name})).unwrap();
        let updated = gpu.sync(&mut s, Some(AuxView::Image), None, 0, 0.0);
        assert_eq!(updated.image_aux.len(), 2);
        assert!(updated.image_aux.values().all(|(_, pixels)| pixels.as_chunks::<4>().0.iter().all(|pixel| *pixel == [255, 0, 0, 255])));
    }

    #[test]
    fn the_id_mask_view_uploads_reimported_pixels_even_if_the_address_was_reused() {
        let mut gpu = link();
        let mut session = session(false);
        execute(&mut session, "layer.add.fill", &json!({})).unwrap();
        let selection = execute(&mut session, "effect.add.colorSelection", &json!({})).unwrap();
        let effect = LayerId(selection["effect"].as_u64().unwrap());
        let resource = "selection.png";
        let image = |color| Arc::new(ImageRes::new(1, 1, vec![color], false));
        session.images.insert(resource.into(), image([1.0, 0.0, 0.0, 1.0]));
        execute(&mut session, "effect.set", &json!({"effect": effect.0, "idMap": resource})).unwrap();

        let first = gpu.sync(&mut session, Some(AuxView::IdMask(effect)), None, 0, 0.0);

        assert!(first.aux[0].2.as_chunks::<4>().0.iter().all(|pixel| *pixel == [255, 0, 0, 255]));

        let (set, original) = session.id_mask_image(effect).unwrap();
        let retired = Arc::downgrade(original);
        drop(session.images.remove(resource));

        assert!(retired.upgrade().is_none(), "upload ownership must drop independently of decoded source pixels");

        let replacement = image([0.0, 0.0, 1.0, 1.0]);

        // Simulate an allocator returning the previous address for the new image
        gpu.aux.get_mut(&set).unwrap().1 = Arc::as_ptr(&replacement) as u64;
        session.images.insert(resource.into(), replacement);

        let updated = gpu.sync(&mut session, Some(AuxView::IdMask(effect)), None, 0, 0.0);

        assert_eq!(updated.aux.len(), 1, "replacement pixels must be sent even when the raw address key matches");
        assert!(updated.aux[0].2.as_chunks::<4>().0.iter().all(|pixel| *pixel == [0, 0, 255, 255]));
        assert!(gpu.sync(&mut session, Some(AuxView::IdMask(effect)), None, 0, 0.0).aux.is_empty());

        gpu.sync(&mut session, None, None, 0, 0.0);

        assert!(gpu.sync(&mut session, Some(AuxView::IdMask(effect)), None, 0, 0.0).aux.is_empty(), "an unchanged pick reuses the existing upload");

        let (_, active) = session.id_mask_image(effect).unwrap();
        let active = Arc::downgrade(active);
        drop(session.images.remove(resource));

        assert!(active.upgrade().is_none(), "only the resource store owns the decoded pixels");

        gpu.sync(&mut session, None, None, 0, 0.0);

        assert!(gpu.aux_id_mask.is_none(), "the upload identity expires with the removed image");
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
