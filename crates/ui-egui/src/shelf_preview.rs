//! Shelf assets and their preview renderers

use std::path::PathBuf;
use std::sync::Arc;

use egui::ColorImage;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::materials;
use texturecraft_engine::orb_preview::MAX_PREVIEW_SIZE_PX;
use texturecraft_engine::paint::BrushSettings;
use texturecraft_engine::paint::alphas::LibraryAlpha;
use texturecraft_engine::paint::presets::BrushPreset;
use texturecraft_engine::preview;

use crate::preview_image::{alpha_image, image_preview, procedural_image, sphere_image, stroke_preview};

/// The asset a shelf tile previews
pub enum ShelfPreview {
    /// A built-in plain material on a sphere
    Material(&'static str),
    /// Smart material rendering requires the worker's baked orb
    SmartMaterial(&'static str),
    /// A user smart material file (`.tcmat`), read and rendered on the worker's baked orb
    // The web has no library folder, so nothing builds this there
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    UserMaterial(PathBuf),
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
    /// An image resource by name
    Image(String, Arc<ImageRes>),
}

impl ShelfPreview {
    /// Smart materials preview on the orb, plain materials on a sphere
    pub fn material(id: &'static str) -> ShelfPreview {
        match materials::find(id) {
            Some(found) if found.smart => ShelfPreview::SmartMaterial(id),
            Some(_) | None => ShelfPreview::Material(id),
        }
    }

    /// The size to render at when `requested_px` is wanted: the previews rendered on the baked orb
    /// stay within its camera's limit, which a high display scale would exceed
    pub fn limited_size_px(&self, requested_px: usize) -> usize {
        match self {
            ShelfPreview::SmartMask(_) | ShelfPreview::SmartMaterial(_) | ShelfPreview::UserMaterial(_) => requested_px.clamp(1, MAX_PREVIEW_SIZE_PX),
            ShelfPreview::Material(_)
            | ShelfPreview::Filter(_)
            | ShelfPreview::Procedural(_)
            | ShelfPreview::Alpha(_)
            | ShelfPreview::Brush(_)
            | ShelfPreview::Image(..) => requested_px,
        }
    }

    /// The cache key of the preview (one per asset)
    pub fn key(&self) -> String {
        match self {
            ShelfPreview::Material(id) => format!("mat:{id}"),
            ShelfPreview::SmartMaterial(id) => format!("smart-mat:{id}"),
            ShelfPreview::UserMaterial(path) => format!("user-mat:{}", path.display()),
            ShelfPreview::SmartMask(id) => format!("mask:{id}"),
            ShelfPreview::Filter(id) => format!("filter:{id}"),
            ShelfPreview::Procedural(kind) => format!("proc:{kind:?}"),
            ShelfPreview::Alpha(alpha) => format!("alpha:{}", alpha.id),
            ShelfPreview::Brush(preset) => format!("brush:{}", preset.id),
            ShelfPreview::Image(name, _) => format!("image:{name}"),
        }
    }

    /// Render a `size²` image; unknown assets return None
    /// Smart masks and smart materials return None here because their bake belongs to the worker
    pub fn image(&self, size: usize) -> Option<ColorImage> {
        match self {
            ShelfPreview::Material(id) => Some(sphere_image(size, &preview::material(id, size)?)),
            ShelfPreview::SmartMask(_) | ShelfPreview::SmartMaterial(_) | ShelfPreview::UserMaterial(_) => None,
            ShelfPreview::Filter(id) => Some(sphere_image(size, &preview::filter(id, size)?)),
            ShelfPreview::Procedural(kind) => Some(procedural_image(*kind, size)),
            ShelfPreview::Alpha(alpha) => Some(alpha_image(alpha, size)),
            ShelfPreview::Brush(preset) => Some(stroke_preview(&preset.applied(&BrushSettings::default()), size)),
            ShelfPreview::Image(_, image) => Some(image_preview(image, size)),
        }
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::filter_lib::FILTERS;
    use texturecraft_engine::materials::MATERIALS;
    use texturecraft_engine::paint::alphas;
    use texturecraft_engine::paint::presets::PRESETS;

    use super::*;

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
        assert!(ShelfPreview::UserMaterial("steel.tcmat".into()).image(BIG_PX).is_none(), "user smart materials need the baked orb");
    }

    #[test]
    fn user_material_previews_are_cached_per_file() {
        let (steel, rust) = (ShelfPreview::UserMaterial("library/steel.tcmat".into()), ShelfPreview::UserMaterial("library/rust.tcmat".into()));
        assert_ne!(steel.key(), rust.key());
        assert_ne!(steel.key(), ShelfPreview::SmartMaterial("steel").key(), "a file never shares the key of a built-in material");
    }

    #[test]
    fn previews_on_the_orb_stay_within_the_camera_limit_at_high_display_scales() {
        // The hover preview asks for 256 points: 768 pixels at a 300 % display scale
        let requested_px = MAX_PREVIEW_SIZE_PX * 3 / 2;
        let on_the_orb =
            [ShelfPreview::SmartMask("edges_sharp"), ShelfPreview::SmartMaterial("steel_painted_worn"), ShelfPreview::UserMaterial("steel.tcmat".into())];
        for preview in on_the_orb {
            assert_eq!(preview.limited_size_px(requested_px), MAX_PREVIEW_SIZE_PX, "{}", preview.key());
            assert_eq!(preview.limited_size_px(0), 1, "{}", preview.key());
        }

        assert_eq!(ShelfPreview::Material("steel").limited_size_px(requested_px), requested_px, "previews drawn without the orb keep the size asked for");
    }

    #[test]
    fn smart_materials_and_only_they_preview_on_the_orb() {
        for material in MATERIALS {
            let on_the_orb = matches!(ShelfPreview::material(material.id), ShelfPreview::SmartMaterial(_));
            assert_eq!(on_the_orb, material.smart, "{}", material.id);
        }
    }
}
