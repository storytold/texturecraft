//! Shelf and brush preview caches
//! Small synchronous renders are limited per frame
//! Smart masks, smart materials and hover previews use [`PreviewWorker`]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use egui::{Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions};
use texturecraft_engine::Session;
use texturecraft_engine::compose::{ImageRes, ImageStore};
use texturecraft_engine::core::{BakedMap, GeneratorKind, ProceduralKind};
use texturecraft_engine::paint::presets::BrushPreset;
use texturecraft_engine::paint::{BrushAlpha, BrushSettings, alphas};
use texturecraft_engine::texel::linear_to_srgb;

use crate::preview_image::{aces, image_preview, shade};
use crate::preview_request::PreviewRequest;
use crate::preview_worker::PreviewWorker;
use crate::shelf_preview::ShelfPreview;
use crate::worker_preview::WorkerPreview;

/// UI-thread renders per frame at most; a web worker request uses the whole budget
const PER_FRAME: usize = 2;
/// Shelf sphere and smart mask thumbnail size in pixels
const SPHERE_PX: usize = 72;
/// Shelf procedural swatch size in pixels
const PROCEDURAL_PX: usize = 64;
/// Shelf alpha and brush stroke swatch size in pixels
const SWATCH_PX: usize = 64;
/// Image resource preview size in pixels
const IMAGE_PX: usize = 64;
/// The [`ShelfPreview::key`] prefix of a project generator preview
const GENERATOR_SET_KEY_PREFIX: &str = "generator-set:";
/// The [`ShelfPreview::key`] prefix of the user's saved files
const SAVED_FILE_KEY_PREFIX: &str = "saved-";
/// Largest stencil preview dimension in pixels
const STENCIL_MAX_SIZE_PX: f32 = 1024.0;
/// Big hover previews kept at most (the least recently shown go first)
const BIG_PREVIEWS_KEPT: usize = 32;

/// Parsed settings, weak image identity and preview revision for a saved brush
type SavedBrushCacheEntry = (Option<BrushSettings>, Option<Weak<ImageRes>>, u64);

/// The shelf's and the brush panel's preview textures
#[derive(Default)]
pub struct Thumbs {
    /// Shelf thumbnails by [`ShelfPreview::key`]
    shelf: HashMap<String, TextureHandle>,
    /// Parsed settings, weak image identity and preview revision for each saved brush
    saved_brush_previews: HashMap<PathBuf, SavedBrushCacheEntry>,
    /// Refresh generation; queued results from earlier generations must not match new requests
    saved_files_epoch: u64,
    images: HashMap<String, (Weak<ImageRes>, TextureHandle)>,
    images_revision: Option<u64>,
    preview_session: Option<(u64, u64)>,
    /// Weak identities for images read by configured generator previews
    generator_images: HashMap<String, Weak<ImageRes>>,
    /// Separates cached pixels and late completions when an image allocation is replaced
    generator_images_epoch: u64,
    /// Each texture set's ID mesh map preview, with the map it shows (a rebake's new map replaces it)
    id_maps: HashMap<usize, (Weak<Vec<f32>>, TextureHandle)>,
    /// (frame, thumbnails rendered in it)
    budget: (u64, usize),
    /// Renders the orb thumbnails and the big previews
    worker: PreviewWorker,
    /// Worker-rendered previews by [`worker_key`]
    rendered: HashMap<String, WorkerPreview>,
    /// The keys of the big previews in `rendered`, least recently shown first
    big_shown: VecDeque<String>,
    /// The frame that last took in the worker's results
    received_in_pass: u64,
    /// Each user smart material file as the last library scan found it
    #[cfg(not(target_arch = "wasm32"))]
    user_material_versions: HashMap<std::path::PathBuf, FileVersion>,
    /// How many times a library scan has dropped each user smart material's previews; part of its worker keys, so a result
    /// still in flight for the old file cannot fill the request that replaced it
    user_material_generations: HashMap<std::path::PathBuf, u64>,
    /// Per active generator picture size: the current effect, pending key and last finished picture
    set_pictures: HashMap<(u64, usize), (Option<String>, Option<TextureHandle>)>,
}

/// A file's modified time and size, which change when it is saved again
#[cfg(not(target_arch = "wasm32"))]
type FileVersion = (Option<std::time::SystemTime>, u64);

/// The version of the file at `path`; a file that cannot be read has none
#[cfg(not(target_arch = "wasm32"))]
fn file_version(path: &std::path::Path) -> FileVersion {
    match std::fs::metadata(path) {
        Ok(metadata) => (metadata.modified().ok(), metadata.len()),
        Err(_) => (None, 0),
    }
}

impl Thumbs {
    /// A larger cached texture of an image resource for the stencil overlay (up to 1024 px)
    pub fn stencil(&mut self, ctx: &egui::Context, name: &str, image: &Arc<ImageRes>) -> TextureHandle {
        let key = format!("stencil:{name}");
        if let Some((cached_image, texture)) = self.images.get(&key)
            && cached_image.ptr_eq(&Arc::downgrade(image))
        {
            return texture.clone();
        }

        let scale = (STENCIL_MAX_SIZE_PX / image.width.max(image.height).max(1) as f32).min(1.0);
        let (w, h) = (((image.width as f32 * scale) as usize).max(1), ((image.height as f32 * scale) as usize).max(1));
        let mut color_image = ColorImage::filled([w, h], Color32::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                let sample = image.sample((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let encode = |value: f32| {
                    if image.srgb { (value.clamp(0.0, 1.0) * 255.0) as u8 } else { (linear_to_srgb(value) * 255.0) as u8 }
                };
                color_image.pixels[y * w + x] =
                    Color32::from_rgba_unmultiplied(encode(sample[0]), encode(sample[1]), encode(sample[2]), (sample[3].clamp(0.0, 1.0) * 255.0) as u8);
            }
        }

        let texture = ctx.load_texture(key.clone(), color_image, TextureOptions::LINEAR);
        self.images.insert(key, (Arc::downgrade(image), texture.clone()));
        texture
    }

    /// Cached preview of texture set `set_index`'s ID mesh map, its colors as stored (the ID view's); a rebake makes a new one
    pub fn id_mesh_map(&mut self, ctx: &egui::Context, set_index: usize, baked: &texturecraft_engine::compose::BakedMaps) -> Option<TextureHandle> {
        let data = baked.maps.get(&BakedMap::Id)?;

        // The weak reference keeps the shown map's allocation, so a new map never takes its address
        if let Some((shown, texture)) = self.id_maps.get(&set_index)
            && std::ptr::eq(shown.as_ptr(), Arc::as_ptr(data))
        {
            return Some(texture.clone());
        }

        let mut image = ColorImage::filled([IMAGE_PX, IMAGE_PX], Color32::TRANSPARENT);
        for (index, pixel) in image.pixels.iter_mut().enumerate() {
            let uv = [((index % IMAGE_PX) as f32 + 0.5) / IMAGE_PX as f32, ((index / IMAGE_PX) as f32 + 0.5) / IMAGE_PX as f32];
            if let Some(color) = baked.sample_uv(BakedMap::Id, uv) {
                let [red, green, blue] = color.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8);
                *pixel = Color32::from_rgb(red, green, blue);
            }
        }
        let texture = ctx.load_texture(format!("mesh_id_{set_index}"), image, TextureOptions::NEAREST);
        self.id_maps.insert(set_index, (Arc::downgrade(data), texture.clone()));
        Some(texture)
    }

    /// Cached preview of an image resource (64 px, letterboxed)
    pub fn image(&mut self, ctx: &egui::Context, name: &str, image: &Arc<ImageRes>) -> TextureHandle {
        if let Some((cached_image, texture)) = self.images.get(name)
            && cached_image.ptr_eq(&Arc::downgrade(image))
        {
            return texture.clone();
        }

        let texture = ctx.load_texture(format!("res_{name}"), image_preview(image, IMAGE_PX), TextureOptions::LINEAR);
        self.images.insert(name.to_string(), (Arc::downgrade(image), texture.clone()));
        texture
    }

    /// Release previews for deleted or replaced resources after a session change
    pub fn sync_images(&mut self, images: &ImageStore, revision: u64) {
        if self.images_revision == Some(revision) {
            return;
        }

        self.images_revision = Some(revision);
        self.images.retain(|key, (cached_image, _)| {
            let name = key.strip_prefix("stencil:").unwrap_or(key);
            images.get(name).is_some_and(|image| cached_image.ptr_eq(&Arc::downgrade(image)))
        });
    }

    /// Keep project previews only while they refer to current resources or bake maps.
    pub(crate) fn sync_project_previews(&mut self, session: &Session) -> bool {
        let current = (session.project_id, session.revision);
        let previous = self.preview_session.replace(current);
        if previous == Some(current) {
            return false;
        }

        let project_changed = previous.is_some_and(|(project, _)| project != session.project_id);
        if project_changed || previous.is_none() {
            self.images_revision = None;
            self.id_maps.clear();
            self.set_pictures.clear();
            self.rendered.retain(|key, _| {
                let keep = !key.starts_with(GENERATOR_SET_KEY_PREFIX);
                if !keep {
                    self.worker.cancel(key);
                }
                keep
            });
            self.big_shown.retain(|key| !key.starts_with(GENERATOR_SET_KEY_PREFIX));
        }

        let previous_inputs = self.generator_images.len();
        self.generator_images.retain(|name, identity| session.images.get(name).is_some_and(|image| std::ptr::eq(identity.as_ptr(), Arc::as_ptr(image))));
        if self.generator_images.len() != previous_inputs {
            self.generator_images_epoch = self.generator_images_epoch.wrapping_add(1);
        }

        self.sync_images(&session.images, session.revision);
        self.id_maps.retain(|&set, (shown, _)| {
            set < session.doc.project.texture_sets.len()
                && session.baked(set).and_then(|baked| baked.maps.get(&BakedMap::Id)).is_some_and(|map| std::ptr::eq(shown.as_ptr(), Arc::as_ptr(map)))
        });
        project_changed
    }

    /// Render within the frame budget; deferred thumbnails request a repaint
    fn shelf(&mut self, ctx: &egui::Context, preview: ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        let key = preview.key();
        if let Some(texture) = self.shelf.get(&key) {
            return Some(texture.clone());
        }

        if !self.spend_budget(ctx, 1) {
            return None;
        }

        // An unknown asset shows as an empty thumbnail
        let image = preview.image(size_px).unwrap_or_else(|| ColorImage::filled([size_px, size_px], Color32::TRANSPARENT));
        let texture = ctx.load_texture(format!("thumb_{key}"), image, TextureOptions::LINEAR);
        self.shelf.insert(key, texture.clone());
        Some(texture)
    }

    /// Spend `renders` of this frame's UI-thread budget; when it is spent, request a repaint
    fn spend_budget(&mut self, ctx: &egui::Context, renders: usize) -> bool {
        let frame = ctx.cumulative_pass_nr();
        if self.budget.0 != frame {
            self.budget = (frame, 0);
        }
        if self.budget.1 + renders > PER_FRAME {
            ctx.request_repaint();
            return false;
        }

        self.budget.1 += renders;
        true
    }

    /// Request once per asset and size; return None while pending or after failure
    /// Hover requests take priority in the native worker
    fn worker_rendered(&mut self, ctx: &egui::Context, preview: ShelfPreview, size_px: usize, hover: bool) -> Option<TextureHandle> {
        self.receive(ctx);

        let key = self.worker_key(&preview, size_px);
        if !self.rendered.contains_key(&key) && self.may_request(ctx) {
            let queued = self.worker.request(ctx, PreviewRequest { key: key.clone(), preview, size_px, hover });
            self.rendered.insert(key.clone(), if queued { WorkerPreview::Rendering } else { WorkerPreview::Failed });
        }

        self.ready(&key)
    }

    /// Native workers take every request; the web renders inline, so it takes one per frame
    fn may_request(&mut self, ctx: &egui::Context) -> bool {
        !cfg!(target_arch = "wasm32") || self.spend_budget(ctx, PER_FRAME)
    }

    /// The worker-rendered preview under `key` if it arrived
    fn ready(&self, key: &str) -> Option<TextureHandle> {
        match self.rendered.get(key) {
            Some(WorkerPreview::Ready(t)) => Some(t.clone()),
            Some(WorkerPreview::Rendering | WorkerPreview::Failed) | None => None,
        }
    }

    /// Poll once per pass and discard results evicted from the cache
    fn receive(&mut self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        if self.received_in_pass == pass {
            return;
        }

        self.received_in_pass = pass;

        for (key, image) in self.worker.poll() {
            let Some(entry) = self.rendered.get_mut(&key) else { continue };

            *entry = match image {
                Some(image) => WorkerPreview::Ready(ctx.load_texture(format!("thumb_{key}"), image, TextureOptions::LINEAR)),
                None => WorkerPreview::Failed,
            };
        }

        // A stopped worker never answers the rest
        if self.worker.is_stopped() {
            for entry in self.rendered.values_mut().filter(|e| matches!(e, WorkerPreview::Rendering)) {
                *entry = WorkerPreview::Failed;
            }
        }
    }

    /// Request a hover preview and evict the least recently shown beyond the cache limit
    pub fn big(&mut self, ctx: &egui::Context, preview: ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        let key = self.worker_key(&preview, size_px);
        self.big_shown.retain(|k| *k != key);
        self.big_shown.push_back(key);
        while self.big_shown.len() > BIG_PREVIEWS_KEPT {
            if let Some(oldest) = self.big_shown.pop_front() {
                self.rendered.remove(&oldest);
                self.worker.cancel(&oldest);
            }
        }

        self.worker_rendered(ctx, preview, size_px, true)
    }

    /// The big preview of `preview` if the worker already rendered it (requests nothing)
    pub fn rendered_big(&mut self, ctx: &egui::Context, preview: &ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        self.receive(ctx);
        let key = self.worker_key(preview, size_px);
        self.ready(&key)
    }

    /// Material thumbnail: smart materials come from the worker's orb, plain ones are spheres
    pub fn material(&mut self, ctx: &egui::Context, id: &'static str) -> Option<TextureHandle> {
        match ShelfPreview::material(id) {
            preview @ ShelfPreview::SmartMaterial(_) => self.worker_rendered(ctx, preview, SPHERE_PX, false),
            preview => self.shelf(ctx, preview, SPHERE_PX),
        }
    }

    /// Thumbnail of the user smart material file `path` on the worker's orb; None until the worker returns it, or for a file it cannot preview
    #[cfg(not(target_arch = "wasm32"))]
    pub fn user_material(&mut self, ctx: &egui::Context, path: &std::path::Path) -> Option<TextureHandle> {
        self.worker_rendered(ctx, ShelfPreview::UserMaterial(path.to_path_buf()), SPHERE_PX, false)
    }

    /// Take in the user smart material `files` a library scan found, and drop the previews of those saved again or removed since the last scan
    #[cfg(not(target_arch = "wasm32"))]
    pub fn scan_user_materials(&mut self, files: &[std::path::PathBuf]) {
        let versions: HashMap<std::path::PathBuf, FileVersion> = files.iter().map(|file| (file.clone(), file_version(file))).collect();
        let outdated: Vec<std::path::PathBuf> =
            self.user_material_versions.iter().filter(|(file, version)| versions.get(*file) != Some(*version)).map(|(file, _)| file.clone()).collect();
        for file in outdated {
            *self.user_material_generations.entry(file.clone()).or_insert(0) += 1;

            // Worker keys are the preview key, a size and a generation: `key@size#generation`
            let key = ShelfPreview::UserMaterial(file).key();
            let is_outdated = |worker_key: &String| {
                let size =
                    worker_key.strip_prefix(&key).and_then(|rest| rest.strip_prefix('@')).map(|rest| rest.split_once('#').map_or(rest, |(size, _)| size));
                size.is_some_and(|size| size.bytes().all(|digit| digit.is_ascii_digit()))
            };
            self.rendered.retain(|worker_key, _| !is_outdated(worker_key));
            self.big_shown.retain(|worker_key| !is_outdated(worker_key));
        }

        self.user_material_versions = versions;
    }

    /// Drop the thumbnails and hover previews of the user's saved files, which may have changed on disk
    pub fn forget_saved_files(&mut self) {
        self.saved_files_epoch = self.saved_files_epoch.wrapping_add(1);
        let current = |key: &String| !key.starts_with(SAVED_FILE_KEY_PREFIX);
        self.shelf.retain(|key, _| current(key));
        self.rendered.retain(|key, _| current(key));
        self.big_shown.retain(current);
        self.saved_brush_previews.clear();
    }

    /// Smart mask thumbnail; None until the worker returns it
    pub fn smart_mask(&mut self, ctx: &egui::Context, id: &'static str) -> Option<TextureHandle> {
        self.worker_rendered(ctx, ShelfPreview::SmartMask(id), SPHERE_PX, false)
    }

    /// Generator thumbnail on the smart masks' orb; None until the worker returns it
    pub fn generator(&mut self, ctx: &egui::Context, kind: GeneratorKind) -> Option<TextureHandle> {
        self.worker_rendered(ctx, ShelfPreview::Generator(kind), SPHERE_PX, false)
    }

    /// A generator orb (`preview` a [`ShelfPreview::Generator`] or [`ShelfPreview::Curvature`]) rendered `size_px` wide, kept like a thumbnail; None until the worker returns it
    pub fn generator_picture(&mut self, ctx: &egui::Context, preview: ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        self.worker_rendered(ctx, preview, size_px.clamp(1, texturecraft_engine::orb_preview::MAX_PREVIEW_SIZE_PX), false)
    }

    /// Generator `effect`'s picture as set (`preview` a [`ShelfPreview::GeneratorSet`]): one render at a time, the last finished one shown while the next renders, so a drag renders only as fast as the worker keeps up
    pub fn generator_set_picture(&mut self, ctx: &egui::Context, effect: u64, preview: ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        self.receive(ctx);
        let size_px = size_px.clamp(1, texturecraft_engine::orb_preview::MAX_PREVIEW_SIZE_PX);
        let key = self.worker_key(&preview, size_px);
        let (in_flight, last) = self.set_pictures.get(&(effect, size_px)).cloned().unwrap_or_default();

        // Use a completed render as the fallback for the next request
        let last = match in_flight.as_ref().map(|key| self.rendered.get(key)) {
            Some(Some(WorkerPreview::Ready(texture))) => Some(texture.clone()),
            _ => last,
        };
        let busy = in_flight.as_ref().is_some_and(|key| matches!(self.rendered.get(key), Some(WorkerPreview::Rendering)));
        let shown = match self.ready(&key) {
            Some(texture) => Some(texture),
            None if busy => last.clone(),
            None => self.big(ctx, preview, size_px).or_else(|| last.clone()),
        };
        let in_flight = if busy { in_flight } else { Some(key) };

        // Only the displayed effect at this size needs a fallback beyond the shared LRU
        self.set_pictures.retain(|&(owner, size), _| size != size_px || owner == effect);
        self.set_pictures.insert((effect, size_px), (in_flight, shown.clone().or(last)));
        shown
    }

    /// Keeps the generator pictures only of the effects `exists` still finds
    pub fn keep_generator_pictures(&mut self, exists: impl Fn(u64) -> bool) {
        self.set_pictures.retain(|(effect, _), _| exists(*effect));
    }

    /// Sphere thumbnail of library filter `id` applied to a sample material
    pub fn filter(&mut self, ctx: &egui::Context, id: &'static str) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Filter(id), SPHERE_PX)
    }

    /// Grayscale swatch of procedural `kind`
    pub fn procedural(&mut self, ctx: &egui::Context, kind: ProceduralKind) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Procedural(kind), PROCEDURAL_PX)
    }

    /// White-on-black swatch of library alpha `alpha`
    pub fn alpha(&mut self, ctx: &egui::Context, alpha: &'static alphas::LibraryAlpha) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Alpha(alpha), SWATCH_PX)
    }

    /// A lighting environment's panorama
    pub fn environment(&mut self, ctx: &egui::Context, name: &'static str) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Environment(name), SWATCH_PX)
    }

    /// A saved brush preset's stroke along an S curve; None while it renders, empty when the file is unreadable
    pub fn saved_brush(&mut self, ctx: &egui::Context, path: &Path, images: &ImageStore) -> Option<TextureHandle> {
        self.saved_brush_with_settings(ctx, path, None, images)
    }

    /// Render a preset held only in this session through the saved-brush cache
    pub fn session_brush(&mut self, ctx: &egui::Context, name: &str, brush: &BrushSettings, images: &ImageStore) -> Option<TextureHandle> {
        let path = crate::shelf_preview::session_brush_path(name);
        self.saved_brush_with_settings(ctx, &path, Some(brush), images)
    }

    /// Seed in-memory settings only when a thumbnail can use the frame budget
    fn saved_brush_with_settings(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        initial_settings: Option<&BrushSettings>,
        images: &ImageStore,
    ) -> Option<TextureHandle> {
        let key = format!("saved-brush:{}", path.display());
        let cached_preview = self.saved_brush_previews.contains_key(path).then(|| self.saved_brush_preview(path, images));
        if let Some(texture) = self.shelf.get(&key) {
            return Some(texture.clone());
        }

        if !self.spend_budget(ctx, 1) {
            return None;
        }

        if let Some(settings) = initial_settings {
            self.saved_brush_previews.entry(path.to_path_buf()).or_insert_with(|| (Some(settings.clone()), None, 0));
        }

        let preview = cached_preview.unwrap_or_else(|| self.saved_brush_preview(path, images));
        let image = preview.image(SWATCH_PX).unwrap_or_else(|| ColorImage::filled([SWATCH_PX, SWATCH_PX], Color32::TRANSPARENT));
        let texture = ctx.load_texture(format!("thumb_{key}"), image, TextureOptions::LINEAR);
        self.shelf.insert(key, texture.clone());
        Some(texture)
    }

    /// Reuse parsed settings until refresh; rebuild previews when their project image changes
    pub fn saved_brush_preview(&mut self, path: &Path, images: &ImageStore) -> ShelfPreview {
        let cached = self.saved_brush_previews.entry(path.to_path_buf()).or_insert_with(|| (crate::shelf_preview::read_saved_brush(path), None, 0));
        let (settings, previous_image, revision) = cached;
        let image = settings.as_ref().and_then(|brush| brush.alpha_image.as_ref()).and_then(|name| images.get(name));
        let image_changed = match (previous_image.as_ref(), image) {
            (Some(previous), Some(current)) => previous.upgrade().is_none_or(|previous| !Arc::ptr_eq(&previous, current)),
            (None, None) => false,
            _ => true,
        };
        *previous_image = image.map(Arc::downgrade);
        if image_changed {
            *revision = revision.wrapping_add(1);
            let key = format!("saved-brush:{}", path.display());
            let rendered_prefix = format!("{key}@");
            self.shelf.remove(&key);
            self.rendered.retain(|rendered, _| !rendered.starts_with(&rendered_prefix));
            self.big_shown.retain(|rendered| !rendered.starts_with(&rendered_prefix));
        }

        ShelfPreview::SavedBrush { path: path.to_path_buf(), settings: settings.clone(), image: image.cloned() }
    }

    /// Project generator and saved-brush keys include the identity of the request owner
    fn worker_key(&mut self, preview: &ShelfPreview, size_px: usize) -> String {
        let key = format!("{}@{size_px}", preview.key());
        if let ShelfPreview::SavedBrush { path, .. } = preview {
            let revision = self.saved_brush_previews.get(path).map_or(0, |(_, _, revision)| *revision);
            format!("{key}#{}:{revision}", self.saved_files_epoch)
        } else if let ShelfPreview::UserMaterial(path) = preview {
            format!("{key}#{}", self.user_material_generations.get(path).copied().unwrap_or(0))
        } else if let ShelfPreview::GeneratorSet { images, .. } = preview {
            let project = self.preview_session.map(|(project, _)| project);
            if images.is_empty() {
                return format!("{key}#{project:?}");
            }

            for (name, image) in images {
                if self.generator_images.get(name).is_some_and(|seen| std::ptr::eq(seen.as_ptr(), Arc::as_ptr(image))) {
                    continue;
                }

                if self.generator_images.insert(name.clone(), Arc::downgrade(image)).is_some() {
                    self.generator_images_epoch = self.generator_images_epoch.wrapping_add(1);
                }
            }

            format!("{key}#{project:?}:{}", self.generator_images_epoch)
        } else {
            key
        }
    }

    /// A brush preset's stroke along an S curve
    pub fn brush_preset(&mut self, ctx: &egui::Context, preset: &'static BrushPreset) -> Option<TextureHandle> {
        self.shelf(ctx, ShelfPreview::Brush(preset), SWATCH_PX)
    }
}

/// A small shaded sphere drawn with a vertex-colored mesh (layer thumbnails, previews).
pub fn paint_sphere(p: &Painter, c: Pos2, r: f32, base: [f32; 3]) {
    let mut mesh = egui::Mesh::default();
    let rings = 10;
    let segs = 28;
    let color = |dx: f32, dy: f32| {
        let d2 = (dx * dx + dy * dy).min(1.0);
        let n = [dx, dy, (1.0 - d2).sqrt()];
        let s = shade(n, base, 0.0, 0.5).map(|v| (linear_to_srgb(aces(v * 1.4)) * 255.0) as u8);
        Color32::from_rgb(s[0], s[1], s[2])
    };
    mesh.colored_vertex(c, color(0.0, 0.0));
    for i in 1..=rings {
        let rr = i as f32 / rings as f32;
        for j in 0..segs {
            let a = j as f32 / segs as f32 * std::f32::consts::TAU;
            let (dx, dy) = (a.cos() * rr, a.sin() * rr);
            mesh.colored_vertex(Pos2::new(c.x + dx * r, c.y - dy * r), color(dx, dy));
        }
    }
    for j in 0..segs {
        let (a, b) = (1 + j, 1 + (j + 1) % segs);
        mesh.add_triangle(0, a as u32, b as u32);
    }
    for i in 1..rings {
        let r0 = 1 + (i - 1) * segs;
        let r1 = 1 + i * segs;
        for j in 0..segs {
            let (a, b) = (r0 + j, r0 + (j + 1) % segs);
            let (c2, d) = (r1 + j, r1 + (j + 1) % segs);
            mesh.add_triangle(a as u32, c2 as u32, d as u32);
            mesh.add_triangle(a as u32, d as u32, b as u32);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Brush alpha preview drawn as small cells (cheap, no texture).
pub fn paint_alpha(p: &Painter, rect: Rect, a: BrushAlpha, hardness: f32) {
    let n = 20;
    let cell = rect.width() / n as f32;
    for y in 0..n {
        for x in 0..n {
            let lx = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let ly = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let v = a.sample(lx, ly, hardness, 7);
            if v > 0.02 {
                let r = Rect::from_min_size(rect.min + egui::vec2(x as f32 * cell, y as f32 * cell), egui::vec2(cell + 0.5, cell + 0.5));
                p.rect_filled(r, 0.0, Color32::from_white_alpha((v * 255.0) as u8));
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod user_material_tests {
    use std::path::Path;

    use super::*;

    /// The key the worker caches the preview of user smart material `file` under at `size_px`
    fn key(file: &Path, size_px: usize) -> String {
        Thumbs::default().worker_key(&ShelfPreview::UserMaterial(file.to_path_buf()), size_px)
    }

    #[test]
    fn saving_a_user_smart_material_again_drops_only_its_previews() {
        let folder = std::env::temp_dir().join(format!("tc-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (steel, rust) = (folder.join("steel.tcmat"), folder.join("rust.tcmat"));
        std::fs::write(&steel, b"first").unwrap();
        std::fs::write(&rust, b"first").unwrap();

        let mut thumbs = Thumbs::default();
        thumbs.scan_user_materials(&[steel.clone(), rust.clone()]);
        let built_in = thumbs.worker_key(&ShelfPreview::SmartMaterial("steel_painted_worn"), SPHERE_PX);
        for size_px in [SPHERE_PX, 256] {
            thumbs.rendered.insert(key(&steel, size_px), WorkerPreview::Failed);
            thumbs.rendered.insert(key(&rust, size_px), WorkerPreview::Failed);
        }
        thumbs.rendered.insert(built_in.clone(), WorkerPreview::Failed);
        thumbs.big_shown.push_back(key(&steel, 256));

        // Scanning the same files again, as every window focus does, keeps every preview
        thumbs.scan_user_materials(&[steel.clone(), rust.clone()]);
        assert_eq!(thumbs.rendered.len(), 5);

        // Saving steel again drops its previews at every size, and nothing else
        std::fs::write(&steel, b"saved again with more contents").unwrap();
        thumbs.scan_user_materials(&[steel.clone(), rust.clone()]);
        assert!(!thumbs.rendered.contains_key(&key(&steel, SPHERE_PX)) && !thumbs.rendered.contains_key(&key(&steel, 256)));
        assert!(thumbs.big_shown.is_empty(), "the hover preview of the old file is forgotten too");
        assert_eq!(thumbs.rendered.len(), 3);

        // A removed file loses its previews as well
        std::fs::remove_file(&rust).unwrap();
        thumbs.scan_user_materials(&[steel]);
        assert_eq!(thumbs.rendered.keys().collect::<Vec<_>>(), [&built_in]);

        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_result_for_a_file_saved_again_does_not_fill_the_request_that_replaced_it() {
        let folder = std::env::temp_dir().join(format!("tc-thumbs-stale-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let steel = folder.join("steel.tcmat");
        std::fs::write(&steel, b"first").unwrap();

        let mut thumbs = Thumbs::default();
        thumbs.worker = PreviewWorker::Inline { shape: Default::default(), results: Vec::new() };
        thumbs.scan_user_materials(std::slice::from_ref(&steel));
        let preview = ShelfPreview::UserMaterial(steel.clone());
        let old_key = thumbs.worker_key(&preview, SPHERE_PX);
        thumbs.rendered.insert(old_key.clone(), WorkerPreview::Rendering);

        // The file is saved again while the worker is still on the old one, and the replacement is requested
        std::fs::write(&steel, b"saved again with more contents").unwrap();
        thumbs.scan_user_materials(std::slice::from_ref(&steel));
        let new_key = thumbs.worker_key(&preview, SPHERE_PX);
        assert_ne!(old_key, new_key, "the replacement is a different request");
        thumbs.rendered.insert(new_key.clone(), WorkerPreview::Rendering);

        // The old completion arrives now
        if let PreviewWorker::Inline { results, .. } = &mut thumbs.worker {
            results.push((old_key, Some(egui::ColorImage::filled([1, 1], egui::Color32::WHITE))));
        }
        let ctx = egui::Context::default();
        let mut frame = ctx.run_pass(Default::default(), |_| {});
        frame.textures_delta.clear();
        thumbs.receive(&ctx);
        assert!(matches!(thumbs.rendered.get(&new_key), Some(WorkerPreview::Rendering)), "the replacement still waits for its own result");

        std::fs::remove_dir_all(&folder).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use texturecraft_engine::compose::BakedMaps;
    use texturecraft_engine::core::Generator;

    use crate::orb_shape::OrbShape;

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn the_pictures_of_a_removed_generator_go() {
        let mut thumbs = Thumbs::default();
        thumbs.set_pictures.insert((1, 8), (None, None));
        thumbs.set_pictures.insert((2, 8), (None, None));

        thumbs.keep_generator_pictures(|effect| effect == 2);

        assert!(!thumbs.set_pictures.contains_key(&(1, 8)) && thumbs.set_pictures.contains_key(&(2, 8)));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn evicting_generator_requests_releases_images_before_the_worker_resumes() {
        use std::sync::mpsc;
        use texturecraft_engine::geom::primitives;

        let ctx = egui::Context::default();
        let (requests, _paused_worker) = mpsc::channel();
        let (_sender, results) = mpsc::channel();
        let mut thumbs = Thumbs { worker: PreviewWorker::Running { requests, pending: Default::default(), results }, ..Thumbs::default() };
        let mut images = Vec::new();
        let project = || Session::new_project("Preview", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        thumbs.sync_project_previews(&project());

        for index in 0..BIG_PREVIEWS_KEPT + 3 {
            let image = Arc::new(ImageRes::new(1, 1, vec![[0.2; 4]], false));
            images.push(Arc::downgrade(&image));
            let generator = Generator { seed: index as u32, ..Generator::new(GeneratorKind::Dirt) };
            let preview = ShelfPreview::GeneratorSet { generator: Box::new(generator), images: vec![("input".into(), image)] };
            thumbs.generator_set_picture(&ctx, index as u64, preview, 8);
        }

        assert_eq!(images.iter().filter(|image| image.strong_count() > 0).count(), BIG_PREVIEWS_KEPT);
        thumbs.sync_project_previews(&project());
        assert!(images.iter().all(|image| image.strong_count() == 0));
    }

    #[test]
    fn editing_an_image_invalidates_its_finished_generator_picture() {
        let context = egui::Context::default();
        let mut thumbs = Thumbs::default();
        let mut image = Arc::new(ImageRes::new(1, 1, vec![[0.0; 4]], false));
        let preview = |image: &Arc<ImageRes>| ShelfPreview::GeneratorSet {
            generator: Box::new(Generator::new(GeneratorKind::GrayscaleConversion)),
            images: vec![("input".into(), image.clone())],
        };
        let key = thumbs.worker_key(&preview(&image), 8);
        let texture = context.load_texture("old", ColorImage::filled([8, 8], Color32::BLACK), TextureOptions::LINEAR);
        thumbs.rendered.insert(key, WorkerPreview::Ready(texture));
        assert!(thumbs.rendered_big(&context, &preview(&image), 8).is_some());

        Arc::make_mut(&mut image).rgba[0] = [1.0; 4];

        assert!(thumbs.rendered_big(&context, &preview(&image), 8).is_none(), "the previous pixels cannot serve the edited image");
    }

    #[test]
    fn a_generator_picture_shows_the_last_one_while_the_next_renders() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbs { worker: PreviewWorker::Inline { shape: OrbShape::default(), results: Vec::new() }, ..Thumbs::default() };
        let set = |invert| ShelfPreview::GeneratorSet { generator: Box::new(Generator { invert, ..Generator::new(GeneratorKind::Dirt) }), images: Vec::new() };
        let last = ctx.load_texture("last", ColorImage::filled([1, 1], Color32::RED), TextureOptions::LINEAR);
        let rendering = thumbs.worker_key(&set(false), 8);
        thumbs.rendered.insert(rendering.clone(), WorkerPreview::Rendering);
        thumbs.set_pictures.insert((1, 8), (Some(rendering), Some(last.clone())));

        let shown = thumbs.generator_set_picture(&ctx, 1, set(true), 8);

        assert_eq!(shown.map(|texture| texture.id()), Some(last.id()), "the last picture stays up");
        let next_key = thumbs.worker_key(&set(true), 8);
        assert!(!thumbs.rendered.contains_key(&next_key), "the newer settings wait for the render in flight");
    }

    #[test]
    fn replacement_projects_release_the_previous_generators_and_discard_their_worker_results() {
        use texturecraft_engine::geom::primitives;

        let ctx = egui::Context::default();
        let mut thumbs = Thumbs { worker: PreviewWorker::Inline { shape: OrbShape::default(), results: Vec::new() }, ..Thumbs::default() };
        let preview = ShelfPreview::GeneratorSet { generator: Box::new(Generator::new(GeneratorKind::Dirt)), images: Vec::new() };
        let mut previous = None;

        for name in ["First", "Second", "Third"] {
            let project = Session::new_project(name, primitives::plane(2.0, 2), "builtin:plane", None, 128).unwrap();
            thumbs.sync_project_previews(&project);

            if let Some((_, previous_id)) = &previous {
                assert!(ctx.tex_manager().read().meta(*previous_id).is_none(), "the previous project's picture has been released");
            }
            let current_key = thumbs.worker_key(&preview, 8);
            assert!(thumbs.generator_set_picture(&ctx, 1, preview.clone(), 8).is_none(), "start with an empty preview for the reused effect ID");
            if let Some((key, _)) = previous.take() {
                let PreviewWorker::Inline { results, .. } = &mut thumbs.worker else { panic!("test worker must render inline") };
                results.push((key, None));
            }

            let mut frame = ctx.run_pass(Default::default(), |_| {});
            frame.textures_delta.clear();
            let shown = thumbs.generator_set_picture(&ctx, 1, preview.clone(), 8).expect("the new project rendered");
            previous = Some((current_key, shown.id()));
        }
    }

    #[test]
    fn saved_brush_refresh_discards_old_worker_result() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbs { worker: PreviewWorker::Inline { shape: OrbShape::default(), results: Vec::new() }, ..Thumbs::default() };
        let preview = ShelfPreview::SavedBrush { path: PathBuf::from("refreshed.tcbrush"), settings: None, image: None };

        assert!(thumbs.big(&ctx, preview.clone(), 8).is_none());
        thumbs.forget_saved_files();
        assert!(thumbs.big(&ctx, preview.clone(), 8).is_none());

        let PreviewWorker::Inline { results, .. } = &mut thumbs.worker else { panic!("test worker must render inline") };
        let old_key = results[0].0.clone();
        let current_key = results[1].0.clone();
        *results = vec![(current_key, None), (old_key, Some(ColorImage::filled([8, 8], Color32::RED)))];

        let mut output = ctx.run_pass(Default::default(), |_| {});
        output.textures_delta.clear();
        assert!(thumbs.rendered_big(&ctx, &preview, 8).is_none(), "an old completion must not replace the current failed result");
    }

    #[test]
    fn saved_brush_image_replacement_preserves_current_worker_result() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbs { worker: PreviewWorker::Inline { shape: OrbShape::default(), results: Vec::new() }, ..Thumbs::default() };
        let path = PathBuf::from("changed-alpha.tcbrush");
        let settings = BrushSettings { alpha_image: Some("alpha.png".into()), ..BrushSettings::default() };
        thumbs.saved_brush_previews.insert(path.clone(), (Some(settings), None, 0));
        let image = |value| Arc::new(ImageRes::new(2, 2, vec![[value; 4]; 4], false));
        let mut images: ImageStore = [("alpha.png".into(), image(0.2))].into();

        let old_preview = thumbs.saved_brush_preview(&path, &images);

        assert!(thumbs.big(&ctx, old_preview, 8).is_none());

        images.insert("alpha.png".into(), image(0.8));
        let current_preview = thumbs.saved_brush_preview(&path, &images);
        assert!(thumbs.big(&ctx, current_preview.clone(), 8).is_none());

        let PreviewWorker::Inline { results, .. } = &mut thumbs.worker else { panic!("test worker must render inline") };
        let old_key = results[0].0.clone();
        let current_key = results[1].0.clone();
        *results = vec![(current_key, Some(ColorImage::filled([8, 8], Color32::RED))), (old_key, None)];

        let mut output = ctx.run_pass(Default::default(), |_| {});
        output.textures_delta.clear();

        assert!(thumbs.rendered_big(&ctx, &current_preview, 8).is_some(), "the current completion must survive an old failure");
    }

    #[test]
    fn saved_brush_file_read_waits_for_the_thumbnail_budget() {
        let path = std::env::temp_dir().join(format!("texturecraft-brush-budget-{}-{}.tcbrush", std::process::id(), NEXT_FILE.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, serde_json::to_vec(&BrushSettings::default()).unwrap()).unwrap();
        let context = egui::Context::default();
        let mut thumbs = Thumbs::default();
        thumbs.budget = (context.cumulative_pass_nr(), PER_FRAME);

        assert!(thumbs.saved_brush(&context, &path, &ImageStore::new()).is_none());
        assert!(thumbs.saved_brush_previews.is_empty(), "a deferred thumbnail has not read its file");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn saved_brush_settings_are_read_once_until_refresh() {
        let path =
            std::env::temp_dir().join(format!("texturecraft-brush-preview-{}-{}.tcbrush", std::process::id(), NEXT_FILE.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, serde_json::to_vec(&BrushSettings::default()).unwrap()).unwrap();
        let mut thumbs = Thumbs::default();
        let images = ImageStore::new();

        assert!(matches!(thumbs.saved_brush_preview(&path, &images), ShelfPreview::SavedBrush { settings: Some(_), .. }));
        std::fs::remove_file(&path).unwrap();
        assert!(
            matches!(thumbs.saved_brush_preview(&path, &images), ShelfPreview::SavedBrush { settings: Some(_), .. }),
            "cached settings avoid a second file read"
        );

        thumbs.forget_saved_files();
        assert!(matches!(thumbs.saved_brush_preview(&path, &images), ShelfPreview::SavedBrush { settings: None, .. }), "refresh reads the file again");
    }

    #[test]
    fn session_brush_preview_uses_memory_and_releases_it_on_refresh() {
        let context = egui::Context::default();
        let mut thumbs = Thumbs::default();
        let brush = BrushSettings { size: 0.07, ..BrushSettings::default() };
        let path = crate::shelf_preview::session_brush_path("Session brush");

        assert!(thumbs.session_brush(&context, "Session brush", &brush, &ImageStore::new()).is_some());
        assert!(matches!(thumbs.saved_brush_previews.get(&path), Some((Some(settings), _, _)) if settings.size == brush.size));

        thumbs.forget_saved_files();
        assert!(thumbs.saved_brush_previews.is_empty());
    }

    #[test]
    fn session_brush_waits_for_thumbnail_budget_before_cloning_settings() {
        let context = egui::Context::default();
        let mut thumbs = Thumbs::default();
        thumbs.budget = (context.cumulative_pass_nr(), PER_FRAME);

        assert!(thumbs.session_brush(&context, "First deferred", &BrushSettings::default(), &ImageStore::new()).is_none());
        assert!(thumbs.session_brush(&context, "Second deferred", &BrushSettings::default(), &ImageStore::new()).is_none());
        assert!(thumbs.saved_brush_previews.is_empty());
    }

    #[test]
    fn saved_brush_cache_releases_old_images_on_change_and_refresh() {
        const IMAGE_SIDE: u32 = 512;
        const IMAGE_TEXELS: usize = IMAGE_SIDE as usize * IMAGE_SIDE as usize;
        const IMAGE_BYTES: usize = IMAGE_TEXELS * std::mem::size_of::<[f32; 4]>();

        let mut thumbs = Thumbs::default();
        let first_path = PathBuf::from("first.tcbrush");
        let second_path = PathBuf::from("second.tcbrush");
        let settings = BrushSettings { alpha_image: Some("alpha.png".into()), ..BrushSettings::default() };
        let old_image = Arc::new(ImageRes::new(IMAGE_SIDE, IMAGE_SIDE, vec![[1.0; 4]; IMAGE_TEXELS], false));
        let other_image = Arc::new(ImageRes::new(IMAGE_SIDE, IMAGE_SIDE, vec![[0.5; 4]; IMAGE_TEXELS], false));
        let old_weak = Arc::downgrade(&old_image);
        let other_weak = Arc::downgrade(&other_image);

        assert!(old_image.rgba.iter().map(|pixel| std::hint::black_box(pixel[0])).sum::<f32>() > 0.0);
        assert!(other_image.rgba.iter().map(|pixel| std::hint::black_box(pixel[0])).sum::<f32>() > 0.0);

        thumbs.saved_brush_previews.insert(first_path.clone(), (Some(settings.clone()), None, 0));
        thumbs.saved_brush_previews.insert(second_path.clone(), (Some(settings), None, 0));
        let mut images: ImageStore = [("alpha.png".into(), old_image.clone())].into();
        drop(thumbs.saved_brush_preview(&first_path, &images));
        images.insert("alpha.png".into(), other_image.clone());
        drop(thumbs.saved_brush_preview(&second_path, &images));
        drop(old_image);
        drop(other_image);

        assert!(old_weak.upgrade().is_none(), "cached settings do not retain a replaced image");
        assert!(other_weak.upgrade().is_some(), "the project still owns its current image");
        eprintln!(
            "saved brush cache after first replacement: 0 of {IMAGE_BYTES} old image bytes retained, {} current image strong owners",
            Arc::strong_count(&images["alpha.png"])
        );

        let replacement = Arc::new(ImageRes::new(IMAGE_SIDE, IMAGE_SIDE, vec![[0.0; 4]; IMAGE_TEXELS], false));
        images.insert("alpha.png".into(), replacement);
        drop(thumbs.saved_brush_preview(&first_path, &images));

        assert!(other_weak.upgrade().is_none(), "another saved brush does not retain the old project image");
        eprintln!("saved brush cache after second replacement: 0 of {IMAGE_BYTES} previous image bytes retained");

        thumbs.forget_saved_files();

        assert!(thumbs.saved_brush_previews.is_empty(), "refresh releases parsed brush settings");
    }

    #[test]
    fn replacing_an_image_rebuilds_its_preview() {
        let context = egui::Context::default();
        let mut thumbs = Thumbs::default();
        let image = |value| Arc::new(ImageRes::new(2, 2, vec![[value; 4]; 4], false));

        let first_image = image(0.2);
        let replacement_image = image(0.8);
        let first = thumbs.image(&context, "sample.png", &first_image);
        let second = thumbs.image(&context, "sample.png", &replacement_image);

        assert_ne!(first.id(), second.id(), "a replaced resource must update its preview");
    }

    #[test]
    fn removing_resources_releases_their_previews() {
        let context = egui::Context::default();
        let mut thumbs = Thumbs::default();
        let mut images = ImageStore::new();
        for index in 0..3 {
            let name = format!("image-{index}");
            images.insert(name, Arc::new(ImageRes::new(2, 2, vec![[index as f32; 4]; 4], false)));
        }

        for (name, image) in &images {
            drop(thumbs.image(&context, name, image));
        }
        let cached_pixel_bytes = |thumbs: &Thumbs| thumbs.images.values().map(|(_, texture)| texture.size()[0] * texture.size()[1] * 4).sum::<usize>();
        let before_count = thumbs.images.len();
        let before_pixel_bytes = cached_pixel_bytes(&thumbs);
        images.remove("image-0");
        images.remove("image-1");
        thumbs.sync_images(&images, 1);
        let after_count = thumbs.images.len();
        let after_pixel_bytes = cached_pixel_bytes(&thumbs);

        eprintln!("cached previews: {before_count} -> {after_count}; texture pixel bytes: {before_pixel_bytes} -> {after_pixel_bytes}");

        assert_eq!((before_count, after_count), (3, 1));
        assert!(after_pixel_bytes < before_pixel_bytes);
        assert!(thumbs.images.contains_key("image-2"));

        images.insert("image-3".into(), Arc::new(ImageRes::new(2, 2, vec![[0.7; 4]; 4], false)));
        drop(thumbs.image(&context, "image-3", &images["image-3"]));
        thumbs.sync_images(&images, 2);

        assert_eq!(thumbs.images.len(), 2);

        images.clear();
        thumbs.sync_images(&images, 3);
        eprintln!("cached previews after clearing resources: {}", thumbs.images.len());
        assert!(thumbs.images.is_empty());
    }

    /// A 2 × 2 ID map of one red, in a map of its own
    fn id_map() -> BakedMaps {
        BakedMaps { resolution: 2, maps: BTreeMap::from([(BakedMap::Id, Arc::new([1.0, 0.0, 0.0].repeat(4)))]) }
    }

    #[test]
    fn an_id_map_preview_is_kept_while_its_map_lives_and_a_rebaked_map_gets_a_new_one() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbs::default();
        let baked = id_map();

        let first = thumbs.id_mesh_map(&ctx, 0, &baked).map(|texture| texture.id());
        let shared = thumbs.id_mesh_map(&ctx, 0, &baked.clone()).map(|texture| texture.id());
        let rebaked = thumbs.id_mesh_map(&ctx, 0, &id_map()).map(|texture| texture.id());

        assert!(first.is_some());
        assert_eq!(shared, first, "the same map keeps its cached preview");
        assert_ne!(rebaked, first, "a new map of the same texture set, even of equal texels, gets a new preview");
    }

    #[test]
    fn dropped_bakes_and_replaced_projects_release_their_id_preview_textures() {
        use texturecraft_engine::geom::primitives;

        let ctx = egui::Context::default();
        let mut thumbs = Thumbs::default();
        let mut project = Session::new_project("A", primitives::material_ball(), "builtin:materialBall", None, 128).unwrap();
        project.doc.baked.insert(0, id_map());
        project.doc.baked.insert(1, id_map());
        thumbs.sync_project_previews(&project);
        let first = thumbs.id_mesh_map(&ctx, 0, project.baked(0).unwrap()).unwrap().id();
        let second = thumbs.id_mesh_map(&ctx, 1, project.baked(1).unwrap()).unwrap().id();

        let _undo_map = project.doc.baked.remove(&0).unwrap();
        project.revision += 1;
        thumbs.sync_project_previews(&project);

        assert!(ctx.tex_manager().read().meta(first).is_none(), "an undo-owned map does not need a preview");
        assert!(ctx.tex_manager().read().meta(second).is_some(), "a live bake keeps its texture");

        let mut replacement = Session::new_project("B", primitives::plane(2.0, 2), "builtin:plane", None, 128).unwrap();
        replacement.revision = project.revision;
        thumbs.sync_project_previews(&replacement);

        assert!(ctx.tex_manager().read().meta(second).is_none(), "a different project with the same revision releases the old texture");
    }
}
