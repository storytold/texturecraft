//! Shelf and brush preview caches
//! Small synchronous renders are limited per frame
//! Smart masks, smart materials and hover previews use [`PreviewWorker`]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use egui::{Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions};
use texturecraft_engine::compose::{ImageRes, ImageStore};
use texturecraft_engine::core::ProceduralKind;
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
            }
        }

        self.worker_rendered(ctx, preview, size_px, true)
    }

    /// The big preview of `preview` if the worker already rendered it (requests nothing)
    pub fn rendered_big(&mut self, ctx: &egui::Context, preview: &ShelfPreview, size_px: usize) -> Option<TextureHandle> {
        self.receive(ctx);
        self.ready(&self.worker_key(preview, size_px))
    }

    /// Material thumbnail: smart materials come from the worker's orb, plain ones are spheres
    pub fn material(&mut self, ctx: &egui::Context, id: &'static str) -> Option<TextureHandle> {
        match ShelfPreview::material(id) {
            preview @ ShelfPreview::SmartMaterial(_) => self.worker_rendered(ctx, preview, SPHERE_PX, false),
            preview => self.shelf(ctx, preview, SPHERE_PX),
        }
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
        let key = format!("saved-brush:{}", path.display());
        let cached_preview = self.saved_brush_previews.contains_key(path).then(|| self.saved_brush_preview(path, images));
        if let Some(texture) = self.shelf.get(&key) {
            return Some(texture.clone());
        }
        if !self.spend_budget(ctx, 1) {
            return None;
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

    /// A saved-brush key also identifies the refresh and image version of its request
    fn worker_key(&self, preview: &ShelfPreview, size_px: usize) -> String {
        let key = format!("{}@{size_px}", preview.key());
        if let ShelfPreview::SavedBrush { path, .. } = preview {
            let revision = self.saved_brush_previews.get(path).map_or(0, |(_, _, revision)| *revision);
            format!("{key}#{}:{revision}", self.saved_files_epoch)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::orb_shape::OrbShape;

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

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
        let image = |value| Arc::new(ImageRes { width: 2, height: 2, rgba: vec![[value; 4]; 4], srgb: false });
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
    fn saved_brush_cache_releases_old_images_on_change_and_refresh() {
        const IMAGE_SIDE: u32 = 512;
        const IMAGE_TEXELS: usize = IMAGE_SIDE as usize * IMAGE_SIDE as usize;
        const IMAGE_BYTES: usize = IMAGE_TEXELS * std::mem::size_of::<[f32; 4]>();

        let mut thumbs = Thumbs::default();
        let first_path = PathBuf::from("first.tcbrush");
        let second_path = PathBuf::from("second.tcbrush");
        let settings = BrushSettings { alpha_image: Some("alpha.png".into()), ..BrushSettings::default() };
        let old_image = Arc::new(ImageRes { width: IMAGE_SIDE, height: IMAGE_SIDE, rgba: vec![[1.0; 4]; IMAGE_TEXELS], srgb: false });
        let other_image = Arc::new(ImageRes { width: IMAGE_SIDE, height: IMAGE_SIDE, rgba: vec![[0.5; 4]; IMAGE_TEXELS], srgb: false });
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

        let replacement = Arc::new(ImageRes { width: IMAGE_SIDE, height: IMAGE_SIDE, rgba: vec![[0.0; 4]; IMAGE_TEXELS], srgb: false });
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
        let image = |value| Arc::new(ImageRes { width: 2, height: 2, rgba: vec![[value; 4]; 4], srgb: false });

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
            images.insert(name, Arc::new(ImageRes { width: 2, height: 2, rgba: vec![[index as f32; 4]; 4], srgb: false }));
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

        images.insert("image-3".into(), Arc::new(ImageRes { width: 2, height: 2, rgba: vec![[0.7; 4]; 4], srgb: false }));
        drop(thumbs.image(&context, "image-3", &images["image-3"]));
        thumbs.sync_images(&images, 2);
        assert_eq!(thumbs.images.len(), 2);

        images.clear();
        thumbs.sync_images(&images, 3);
        eprintln!("cached previews after clearing resources: {}", thumbs.images.len());
        assert!(thumbs.images.is_empty());
    }
}
