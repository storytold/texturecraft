//! Shelf assets and their preview renderers

use std::path::PathBuf;
use std::sync::Arc;

use egui::ColorImage;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::compose::ImageStore;
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::materials;
use texturecraft_engine::paint::BrushSettings;
use texturecraft_engine::paint::alphas::LibraryAlpha;
use texturecraft_engine::paint::presets::BrushPreset;
use texturecraft_engine::preview;

use crate::preview_image::{alpha_image, environment_image, image_preview, procedural_image, sphere_image, stroke_preview};

/// Larger brush preset files aren't read (the `brush.preset.load` command's limit)
#[cfg(not(target_arch = "wasm32"))]
const MAX_SAVED_BRUSH_BYTES: u64 = 1 << 20;

/// The asset a shelf tile previews
#[derive(Clone)]
pub enum ShelfPreview {
    /// A built-in plain material on a sphere
    Material(&'static str),
    /// Smart material rendering requires the worker's baked orb
    SmartMaterial(&'static str),
    /// Smart mask rendering requires the worker's baked orb
    SmartMask(&'static str),
    /// A library filter on a sample material on a sphere
    Filter(&'static str),
    /// A procedural pattern as a grayscale swatch
    Procedural(ProceduralKind),
    /// A library brush alpha, white on black
    Alpha(&'static LibraryAlpha),
    /// A brush preset's stroke along an S curve
    Brush(&'static BrushPreset),
    /// A brush preset file the user saved, its stroke along an S curve
    SavedBrush { path: PathBuf, settings: Option<BrushSettings>, image: Option<Arc<ImageRes>> },
    /// An image resource by name
    Image(String, Arc<ImageRes>),
    /// A built-in lighting environment's panorama
    Environment(&'static str),
}

impl ShelfPreview {
    /// Resolve a saved brush's image alpha while the project images are available
    pub fn saved_brush(path: &std::path::Path, images: &ImageStore) -> Self {
        let settings = read_saved_brush(path);
        let image = settings.as_ref().and_then(|brush| brush.alpha_image.as_ref()).and_then(|name| images.get(name)).cloned();
        ShelfPreview::SavedBrush { path: path.to_path_buf(), settings, image }
    }

    /// Preview a brush saved only in the current session
    pub fn session_brush(name: &str, brush: &BrushSettings, images: &ImageStore) -> Self {
        let image = brush.alpha_image.as_ref().and_then(|image_name| images.get(image_name)).cloned();
        ShelfPreview::SavedBrush { path: session_brush_path(name), settings: Some(brush.clone()), image }
    }

    /// Smart materials preview on the orb, plain materials on a sphere
    pub fn material(id: &'static str) -> ShelfPreview {
        match materials::find(id) {
            Some(found) if found.smart => ShelfPreview::SmartMaterial(id),
            Some(_) | None => ShelfPreview::Material(id),
        }
    }

    /// The cache key of the preview (one per asset)
    pub fn key(&self) -> String {
        match self {
            ShelfPreview::Material(id) => format!("mat:{id}"),
            ShelfPreview::SmartMaterial(id) => format!("smart-mat:{id}"),
            ShelfPreview::SmartMask(id) => format!("mask:{id}"),
            ShelfPreview::Filter(id) => format!("filter:{id}"),
            ShelfPreview::Procedural(kind) => format!("proc:{kind:?}"),
            ShelfPreview::Alpha(alpha) => format!("alpha:{}", alpha.id),
            ShelfPreview::Brush(preset) => format!("brush:{}", preset.id),
            ShelfPreview::SavedBrush { path, .. } => format!("saved-brush:{}", path.display()),
            ShelfPreview::Image(name, _) => format!("image:{name}"),
            ShelfPreview::Environment(name) => format!("environment:{name}"),
        }
    }

    /// Render a `size²` image; unknown assets return None
    /// Smart masks and smart materials return None here because their bake belongs to the worker
    pub fn image(&self, size: usize) -> Option<ColorImage> {
        match self {
            ShelfPreview::Material(id) => Some(sphere_image(size, &preview::material(id, size)?)),
            ShelfPreview::SmartMask(_) | ShelfPreview::SmartMaterial(_) => None,
            ShelfPreview::Filter(id) => Some(sphere_image(size, &preview::filter(id, size)?)),
            ShelfPreview::Procedural(kind) => Some(procedural_image(*kind, size)),
            ShelfPreview::Alpha(alpha) => Some(alpha_image(alpha, size)),
            ShelfPreview::Brush(preset) => Some(stroke_preview(&preset.applied(&BrushSettings::default()), size, None)),
            ShelfPreview::SavedBrush { settings, image, .. } => {
                let settings = settings.as_ref()?;
                if settings.alpha_image.is_some() && image.is_none() {
                    return None;
                }

                Some(stroke_preview(settings, size, image.as_deref()))
            }
            ShelfPreview::Image(_, image) => Some(image_preview(image, size)),
            ShelfPreview::Environment(name) => Some(environment_image(name, size)),
        }
    }
}

/// A cache key for a preset without a file
pub(crate) fn session_brush_path(name: &str) -> PathBuf {
    PathBuf::from(format!("memory:{name}"))
}

/// A saved brush preset's settings; None when the file can't be read (or in the browser, which has no files)
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_saved_brush(path: &std::path::Path) -> Option<BrushSettings> {
    use std::io::Read as _;

    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|file| file.take(MAX_SAVED_BRUSH_BYTES + 1).read_to_end(&mut bytes)).ok()?;
    if bytes.len() as u64 > MAX_SAVED_BRUSH_BYTES {
        return None;
    }

    serde_json::from_slice::<texturecraft_engine::brush_preset_cmds::UserBrush>(&bytes)
        .map(|preset| preset.brush)
        .or_else(|_| serde_json::from_slice::<BrushSettings>(&bytes))
        .ok()
        .map(|settings| settings.sanitized())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_saved_brush(_: &std::path::Path) -> Option<BrushSettings> {
    None
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::filter_lib::FILTERS;
    use texturecraft_engine::materials::MATERIALS;
    use texturecraft_engine::paint::alphas;
    use texturecraft_engine::paint::presets::PRESETS;

    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn named_brush_files_render_their_saved_settings() {
        use texturecraft_engine::brush_preset_cmds::UserBrush;

        let path = std::env::temp_dir().join(format!("tc-named-preview-{}.tcbrush", std::process::id()));
        let mut brush = BrushSettings::default();
        brush.size = 0.07;
        let preset = UserBrush { name: "Ink".into(), brush: brush.clone(), channels: None, file: None };
        std::fs::write(&path, serde_json::to_vec(&preset).unwrap()).unwrap();

        let loaded = read_saved_brush(&path).unwrap();
        assert_eq!(loaded.size, brush.size);
        let _ = std::fs::remove_file(path);
    }

    /// The hover preview's size at 100 % scale
    const BIG_PX: usize = 256;

    #[test]
    fn every_tab_renders_a_big_preview() {
        let image = ImageRes { width: 4, height: 2, rgba: vec![[0.8, 0.4, 0.2, 1.0]; 8], srgb: true };
        let previews = [
            ShelfPreview::Material(MATERIALS[0].id),
            ShelfPreview::Filter(FILTERS[0].id),
            ShelfPreview::Procedural(ProceduralKind::ALL[0]),
            ShelfPreview::Alpha(&alphas::library()[0]),
            ShelfPreview::Brush(&PRESETS[0]),
            ShelfPreview::Image("image".into(), Arc::new(image)),
            ShelfPreview::Environment(texturecraft_engine::core::project::ENVIRONMENTS[0]),
        ];
        for preview in previews {
            let key = preview.key();
            let image = preview.image(BIG_PX).unwrap_or_else(|| panic!("{key}"));
            assert_eq!(image.size, [BIG_PX, BIG_PX], "{key}");
            assert!(image.pixels.iter().any(|color| color.a() > 0 && color.r().max(color.g()).max(color.b()) > 0), "{key}: something shows");
        }

        let finite = |samples: Vec<Option<preview::Sample>>| {
            samples.iter().flatten().all(|sample| sample.base.into_iter().chain([sample.metallic, sample.roughness, sample.height]).all(f32::is_finite))
        };
        assert!(finite(preview::material(MATERIALS[0].id, BIG_PX).unwrap_or_default()));
        assert!(finite(preview::filter(FILTERS[0].id, BIG_PX).unwrap_or_default()));
        assert!(ShelfPreview::SmartMask("edges_sharp").image(BIG_PX).is_none(), "smart masks need the baked orb");
        assert!(ShelfPreview::SmartMaterial("steel_painted_worn").image(BIG_PX).is_none(), "smart materials need the baked orb");
    }

    #[test]
    fn smart_materials_and_only_they_preview_on_the_orb() {
        for material in MATERIALS {
            let on_the_orb = matches!(ShelfPreview::material(material.id), ShelfPreview::SmartMaterial(_));
            assert_eq!(on_the_orb, material.smart, "{}", material.id);
        }
    }

    #[test]
    fn saved_brush_preview_uses_its_project_image_alpha() {
        let settings = BrushSettings { alpha_image: Some("alpha.png".into()), ..BrushSettings::default() };
        let transparent = Arc::new(ImageRes { width: 2, height: 2, rgba: vec![[1.0, 1.0, 1.0, 0.0]; 4], srgb: false });
        let preview = ShelfPreview::SavedBrush { path: PathBuf::from("brush.tcbrush"), settings: Some(settings.clone()), image: Some(transparent) };

        assert!(preview.image(64).is_some_and(|image| image.pixels.iter().all(|pixel| pixel.r() == 0)));
        let missing = ShelfPreview::SavedBrush { path: PathBuf::from("brush.tcbrush"), settings: Some(settings), image: None };
        assert!(missing.image(64).is_none());
    }
}
