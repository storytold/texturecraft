//! One asset on the shelf: its name, the tab it belongs to, and how it previews, drags and applies

use std::path::PathBuf;

use serde_json::json;
use texturecraft_engine::Session;
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::core::project::ENVIRONMENTS;
use texturecraft_engine::filter_lib::{FILTERS, FilterDef};
use texturecraft_engine::materials::{MATERIALS, SmartMaterial};
use texturecraft_engine::paint::alphas::{self, LibraryAlpha};
use texturecraft_engine::paint::presets::{BrushPreset, PRESETS};
use texturecraft_engine::smart_masks::{SMART_MASKS, SmartMask};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::shelf::{Asset, Tab, apply};
use crate::shelf_preview::ShelfPreview;

/// The name of a saved brush whose file name isn't text
const SAVED_BRUSH_NAME: &str = "brush";

/// Probes per side of the grid that compares a procedural under two seeds, off the pattern's grid lines
const SEED_PROBES: usize = 4;
const SEED_PROBE_OFFSET: f32 = 0.13;
/// A smaller difference between the two seeds' samples is rounding
const SEED_TOLERANCE: f32 = 1e-4;

/// The libraries' ids: the open project's images, the built-in assets, the user's saved assets
pub(crate) const PROJECT: &str = "project";
pub(crate) const STARTER: &str = "starter";
pub(crate) const USER: &str = "user";
/// The libraries in the picker's order: id and (English, translated where shown) name
pub(crate) const LIBRARIES: [(&str, &str); 3] = [(PROJECT, "Project"), (STARTER, "Starter assets"), (USER, "Your assets")];

/// A shelf tile's asset
pub(crate) enum ShelfEntry {
    Material(&'static SmartMaterial),
    SmartMask(&'static SmartMask),
    Filter(&'static FilterDef),
    Brush(&'static BrushPreset),
    /// A brush preset file the user saved
    SavedBrush(PathBuf),
    Alpha(&'static LibraryAlpha),
    Procedural(ProceduralKind),
    /// A built-in lighting environment, by name
    Environment(&'static str),
    /// An image resource of the project, by name
    Image(String),
}

impl ShelfEntry {
    /// Every built-in asset, the user's saved brushes and the project's images
    pub(crate) fn all(app: &TextureCraftApp) -> Vec<ShelfEntry> {
        let built_in = MATERIALS
            .iter()
            .map(ShelfEntry::Material)
            .chain(SMART_MASKS.iter().map(ShelfEntry::SmartMask))
            .chain(FILTERS.iter().map(ShelfEntry::Filter))
            .chain(PRESETS.iter().map(ShelfEntry::Brush))
            .chain(alphas::library().iter().map(ShelfEntry::Alpha))
            .chain(ProceduralKind::ALL.into_iter().map(ShelfEntry::Procedural))
            .chain(ENVIRONMENTS.into_iter().map(ShelfEntry::Environment));
        let saved = app.brushes.iter().flatten().cloned().map(ShelfEntry::SavedBrush);
        built_in.chain(saved).chain(app.session.doc.resources.keys().cloned().map(ShelfEntry::Image)).collect()
    }

    /// The tab that lists the asset
    pub(crate) fn tab(&self) -> Tab {
        match self {
            ShelfEntry::Material(material) if material.smart => Tab::Smart,
            ShelfEntry::Material(_) => Tab::Materials,
            ShelfEntry::SmartMask(_) => Tab::Masks,
            ShelfEntry::Filter(_) => Tab::Filters,
            ShelfEntry::Brush(_) | ShelfEntry::SavedBrush(_) => Tab::Brushes,
            ShelfEntry::Alpha(_) => Tab::Alphas,
            ShelfEntry::Procedural(_) => Tab::Textures,
            ShelfEntry::Environment(_) => Tab::Environments,
            ShelfEntry::Image(_) => Tab::Textures,
        }
    }

    /// The asset's identity across frames: its kind and id
    pub(crate) fn identity(&self) -> (&'static str, &str) {
        match self {
            ShelfEntry::Material(material) => ("material", material.id),
            ShelfEntry::SmartMask(mask) => ("mask", mask.id),
            ShelfEntry::Filter(filter) => ("filter", filter.id),
            ShelfEntry::Brush(preset) => ("brush", preset.id),
            ShelfEntry::SavedBrush(path) => ("saved-brush", path.to_str().unwrap_or_default()),
            ShelfEntry::Alpha(alpha) => ("alpha", &alpha.id),
            ShelfEntry::Procedural(kind) => ("procedural", kind.label()),
            ShelfEntry::Environment(name) => ("environment", name),
            ShelfEntry::Image(name) => ("image", name),
        }
    }

    /// Where the asset lives: its library id ([`LIBRARIES`]), its type's folder, then its category if it has one (names in English, shown through `tr`)
    pub(crate) fn path(&self) -> Vec<&str> {
        let (library, folder, category) = match self {
            ShelfEntry::Material(material) if material.smart => (STARTER, "Smart materials", Some(material.category)),
            ShelfEntry::Material(material) => (STARTER, "Materials", Some(material.category)),
            ShelfEntry::SmartMask(_) => (STARTER, "Smart masks", None),
            ShelfEntry::Filter(filter) => (STARTER, "Filters", Some(filter.group)),
            ShelfEntry::Brush(preset) => (STARTER, "Brushes", Some(preset.category)),
            ShelfEntry::SavedBrush(_) => (USER, "Brushes", None),
            ShelfEntry::Alpha(alpha) => (STARTER, "Alphas", Some(alpha.category)),
            ShelfEntry::Procedural(_) => (STARTER, "Procedurals", None),
            ShelfEntry::Environment(_) => (STARTER, "Environments", None),
            ShelfEntry::Image(_) => (PROJECT, "Textures", None),
        };
        [library, folder].into_iter().chain(category).collect()
    }

    /// Whether the asset is in folder `path` (a library id, then folder names) or below it
    pub(crate) fn in_folder(&self, path: &[String]) -> bool {
        let own = self.path();
        path.len() <= own.len() && path.iter().zip(own).all(|(wanted, have)| wanted == have)
    }

    /// The name under the tile, translated where the asset's name is
    pub(crate) fn label(&self) -> &str {
        match self {
            ShelfEntry::Material(material) => material.label,
            ShelfEntry::SavedBrush(path) => path.file_stem().and_then(|stem| stem.to_str()).unwrap_or(SAVED_BRUSH_NAME),
            ShelfEntry::SmartMask(mask) => mask.label,
            ShelfEntry::Filter(filter) => tr(filter.label),
            ShelfEntry::Brush(preset) => preset.label,
            ShelfEntry::Alpha(alpha) => &alpha.label,
            ShelfEntry::Procedural(kind) => tr(kind.label()),
            ShelfEntry::Environment(name) => name,
            ShelfEntry::Image(name) => name,
        }
    }

    /// Whether `filter` (lowercase) matches the asset's name, id or category, in English or translated
    pub(crate) fn matches(&self, filter: &str) -> bool {
        let texts: &[&str] = match self {
            ShelfEntry::Material(material) => &[material.label, material.category, material.id],
            ShelfEntry::SavedBrush(_) => &[self.label()],
            ShelfEntry::SmartMask(mask) => &[mask.label, mask.id],
            ShelfEntry::Filter(filter) => &[filter.label, filter.group, filter.id, tr(filter.label), tr(filter.group)],
            ShelfEntry::Brush(preset) => &[preset.label, preset.category, preset.id],
            ShelfEntry::Alpha(alpha) => &[&alpha.label, alpha.category, &alpha.id],
            ShelfEntry::Procedural(kind) => &[kind.label(), tr(kind.label())],
            ShelfEntry::Environment(name) => &[name],
            ShelfEntry::Image(name) => &[name],
        };
        crate::shelf::matches(filter, texts)
    }

    /// The thumbnail, or None while it renders
    pub(crate) fn thumbnail(&self, app: &mut TextureCraftApp, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        match self {
            ShelfEntry::Material(material) => app.thumbs.material(ctx, material.id),
            ShelfEntry::SmartMask(mask) => app.thumbs.smart_mask(ctx, mask.id),
            ShelfEntry::Filter(filter) => app.thumbs.filter(ctx, filter.id),
            ShelfEntry::Brush(preset) => app.thumbs.brush_preset(ctx, preset),
            ShelfEntry::SavedBrush(path) => app.thumbs.saved_brush(ctx, path),
            ShelfEntry::Alpha(alpha) => app.thumbs.alpha(ctx, alpha),
            ShelfEntry::Procedural(kind) => app.thumbs.procedural(ctx, *kind),
            ShelfEntry::Environment(name) => app.thumbs.environment(ctx, name),
            ShelfEntry::Image(name) => {
                let image = app.session.images.get(name).cloned()?;
                Some(app.thumbs.image(ctx, name, &image))
            }
        }
    }

    /// What the hover popup renders; None for an image that is gone
    pub(crate) fn preview(&self, session: &Session) -> Option<ShelfPreview> {
        Some(match self {
            ShelfEntry::Material(material) => ShelfPreview::material(material.id),
            ShelfEntry::SmartMask(mask) => ShelfPreview::SmartMask(mask.id),
            ShelfEntry::Filter(filter) => ShelfPreview::Filter(filter.id),
            ShelfEntry::Brush(preset) => ShelfPreview::Brush(preset),
            ShelfEntry::SavedBrush(path) => ShelfPreview::SavedBrush(path.clone()),
            ShelfEntry::Alpha(alpha) => ShelfPreview::Alpha(alpha),
            ShelfEntry::Procedural(kind) => ShelfPreview::Procedural(*kind),
            ShelfEntry::Environment(name) => ShelfPreview::Environment(name),
            ShelfEntry::Image(name) => ShelfPreview::Image(name.clone(), session.images.get(name).cloned()?),
        })
    }

    /// The drag payload's asset; brushes and procedurals don't drag
    pub(crate) fn asset(&self) -> Option<Asset> {
        match self {
            ShelfEntry::Material(material) => Some(Asset::Material(material.id)),
            ShelfEntry::SmartMask(mask) => Some(Asset::SmartMask(mask.id)),
            ShelfEntry::Filter(filter) => Some(Asset::Filter(filter.id)),
            ShelfEntry::Alpha(alpha) => Some(Asset::Alpha(&alpha.id)),
            ShelfEntry::Image(name) => Some(Asset::Texture(name.clone())),
            ShelfEntry::Brush(_) | ShelfEntry::SavedBrush(_) | ShelfEntry::Procedural(_) | ShelfEntry::Environment(_) => None,
        }
    }

    /// Whether the tile is outlined as in use: the brush's library alpha, the shown environment
    pub(crate) fn in_use(&self, app: &TextureCraftApp) -> bool {
        match self {
            ShelfEntry::Alpha(alpha) => {
                let brush = &app.session.tool.brush;
                brush.alpha_image.is_none() && brush.alpha_library.as_deref() == Some(alpha.id.as_str())
            }
            ShelfEntry::Environment(name) => app.session.doc.project.display.environment == *name,
            _ => false,
        }
    }

    /// Whether applying the asset gives a different result per random seed (the tile's dots badge)
    pub(crate) fn includes_randomness(&self) -> bool {
        match self {
            ShelfEntry::Procedural(kind) => SEEDED_PROCEDURALS.contains(kind),
            _ => false,
        }
    }

    /// What a click on the tile does
    pub(crate) fn activate(&self, app: &mut TextureCraftApp) {
        match self {
            ShelfEntry::Brush(preset) => {
                app.run("brush.preset.apply", json!({"preset": preset.id}));
            }
            ShelfEntry::SavedBrush(path) => {
                app.run("brush.preset.load", json!({"path": path.display().to_string()}));
            }
            ShelfEntry::Procedural(kind) => {
                app.run("layer.add.fill", json!({"name": kind.label()}));
                app.run("layer.fill.set", json!({"channel": "baseColor", "procedural": kind.label(), "low": [0.1, 0.1, 0.1], "high": [0.9, 0.9, 0.9]}));
            }
            ShelfEntry::Environment(name) => {
                app.run("display.set", json!({"environment": name}));
            }
            entry => {
                if let Some(asset) = entry.asset() {
                    apply(app, &asset);
                }
            }
        }
    }
}

/// The procedurals whose pattern depends on the seed, worked out once
static SEEDED_PROCEDURALS: std::sync::LazyLock<Vec<ProceduralKind>> =
    std::sync::LazyLock::new(|| ProceduralKind::ALL.into_iter().filter(|kind| seed_changes(*kind)).collect());

/// Whether procedural `kind` samples differently under two seeds
fn seed_changes(kind: ProceduralKind) -> bool {
    use texturecraft_engine::compose::pattern::{Coords, procedural};
    use texturecraft_engine::core::{Procedural, Projection};
    use texturecraft_engine::geom::glam::{Vec2, Vec3};

    let sample = |seed: u32, i: usize| {
        let uv = Vec2::new((i % SEED_PROBES) as f32 / SEED_PROBES as f32, (i / SEED_PROBES) as f32 / SEED_PROBES as f32) + SEED_PROBE_OFFSET;
        procedural(&Procedural { seed, ..Procedural::new(kind) }, &Projection::default(), &Coords { uv, pos: Vec3::ZERO, nrm: Vec3::Y })
    };
    (0..SEED_PROBES * SEED_PROBES).any(|i| (sample(1, i) - sample(2, i)).abs() > SEED_TOLERANCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_seeded_procedurals_include_randomness() {
        let seeded = &*SEEDED_PROCEDURALS;
        assert!(seeded.contains(&ProceduralKind::Clouds), "{seeded:?}");
        assert!(!seeded.contains(&ProceduralKind::Checker), "{seeded:?}");
    }
}
