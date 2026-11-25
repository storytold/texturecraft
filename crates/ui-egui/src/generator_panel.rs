//! A generator effect's properties, laid out like the reference app's: its slot, Parameters, Image inputs, Resolution, Attributes

use egui::{CornerRadius, CursorIcon, Painter, Rect, Role, Sense, Stroke, StrokeKind, TextureHandle, WidgetInfo};
use serde_json::json;
use texturecraft_engine::core::{
    CookResolution, Generator, GeneratorKind, KindControls, LayerId, LightParameters, ResolutionMode, SectionParameters, WearParameters,
};

use crate::TextureCraftApp;
use crate::fill_material_panel::{band_size, texture_set_size};
use crate::generator_input_card::input_card;
use crate::generator_input_slot::GeneratorInputSlot;
use crate::generator_params::{defaults_of, differs_from_defaults};
use crate::generator_section_rows::{global_rows, ramp_rows, section_rows, texture_card};
use crate::generator_wear_rows::{light_rows, micro_switch_card, wear_rows};
use crate::i18n::tr;
use crate::property_sheet::PropertySheet;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;

/// The outline on the preview while it can open enlarged under the pointer, or is open, in points
const ENLARGE_OUTLINE_WIDTH: f32 = 1.0;

/// The width the preview's orb is rendered at, in pixels: sharp at twice the preview's size
const PREVIEW_ORB_PX: usize = 256;

/// The texture set size a project without one is shown against
const FALLBACK_TEXTURE_SET_SIZE: u32 = 2048;

/// How soon a picture still rendering is looked for again
const PICTURE_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// The settings of generator effect `id` on `layer`
pub(crate) fn generator_properties(app: &mut TextureCraftApp, ui: &mut egui::Ui, layer: LayerId, id: u64, generator: &Generator) {
    PropertySheet::show(ui, ("generator_sheet", id), |sheet| {
        sheet.section(tr("Generator"), false);
        slot_card(app, sheet, id, generator);

        let band = sheet.band(tr("Parameters"), true, Some(differs_from_defaults(generator)), None);
        if band.reset {
            app.run("effect.set", defaults_of(id, generator));
        }
        if band.open {
            parameters(app, sheet, (layer, id), generator);
        }

        // The kinds with only their own controls show the images the reference lets them take: Grayscale Conversion's own
        let slots: Vec<GeneratorInputSlot> = match generator.kind_controls() {
            Some(KindControls::Grayscale(_)) => vec![GeneratorInputSlot::Image],
            Some(_) => Vec::new(),
            None => generator.inputs_in_use().iter().map(|map| GeneratorInputSlot::Map(*map)).collect(),
        };
        if !slots.is_empty() && sheet.band(tr("Image inputs"), false, None, Some(&slots.len().to_string())).open {
            for slot in slots {
                input_card(app, sheet, (layer, id), slot, slot.source(generator), false);
            }
        }

        let texture_set = texture_set_size(app, FALLBACK_TEXTURE_SET_SIZE);
        if let Some(picked) = crate::resolution_band::resolution_band(sheet, band_size(generator.resolution, texture_set), texture_set) {
            let size = match picked {
                CookResolution::Relative(factor) => json!({"resolution": format!("{:?}", ResolutionMode::Auto), "resolutionFactor": factor}),
                CookResolution::Fixed(size) => json!({"resolution": format!("{:?}", ResolutionMode::Custom), "customResolution": size, "resolutionFactor": 0}),
            };
            app.run("effect.set", crate::generator_params::merged(json!({"effect": id}), size));
        }

        if sheet.band(tr("Attributes"), false, None, None).open {
            attributes(sheet, generator.kind);
        }
        if !app.session.doc.baked.contains_key(&app.session.ts()) {
            sheet.caption(tr("Bake mesh maps for curvature, AO and thickness based generators."));
        }
    });
}

/// The card naming the generator: its orb and name; a click opens the generator picker
fn slot_card(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, generator: &Generator) {
    let ctx = sheet.ui.ctx().clone();
    let thumbnail = app.thumbs.generator(&ctx, generator.kind);
    let tokens = Tokens::get(&ctx);
    let picture = |painter: &Painter, rect: Rect| orb(painter, rect, thumbnail.as_ref(), &tokens, &ctx);
    let slot = sheet.slot(&picture, tr(generator.kind.label()), tr("Generator"), None, false, false);
    sheet.close_card();
    crate::generator_slot::picker(app, &slot.pick, id, generator.kind);
}

/// An orb picture in `rect`, a placeholder while it renders
fn orb(painter: &Painter, rect: Rect, texture: Option<&TextureHandle>, tokens: &Tokens, ctx: &egui::Context) {
    match texture {
        Some(texture) => {
            painter.image(texture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        None => {
            painter.rect_filled(rect, 0.0, tokens.button);
            ctx.request_repaint_after(PICTURE_POLL);
        }
    }
}

/// The Parameters band's rows: the kind's own controls, else the global block and the earlier model's ramp
fn parameters(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, (layer, id): (LayerId, u64), generator: &Generator) {
    let ctx = sheet.ui.ctx().clone();
    let tokens = Tokens::get(&ctx);
    let kind = generator.kind;

    // The Curvature size under the pointer shows alone in the preview: the columns are drawn after it, so it is kept from the frame before
    let solo_id = sheet.ui.make_persistent_id(("generator_solo", id));
    let solo: Option<usize> = sheet.ui.data(|data| data.get_temp(solo_id)).flatten();
    let solo_texture = match (kind, generator.detail_in_use(), solo) {
        (GeneratorKind::Curvature, Some(detail), Some(size)) => {
            app.thumbs.generator_picture(&ctx, ShelfPreview::Curvature { mode: detail.mode, solo: Some(size) }, PREVIEW_ORB_PX)
        }
        _ => None,
    };

    // The preview shows the generator as set; a size shows the whole preview until its own render arrives, and the kind at its defaults shows until the first render
    let set = ShelfPreview::generator_set(generator, &app.session.images);
    let texture = solo_texture
        .or_else(|| app.thumbs.generator_set_picture(&ctx, id, set, PREVIEW_ORB_PX))
        .or_else(|| app.thumbs.generator_picture(&ctx, ShelfPreview::Generator(kind), PREVIEW_ORB_PX));
    let drawn_at = std::cell::Cell::new(None);
    let picture = |painter: &Painter, rect: Rect| {
        drawn_at.set(Some(rect));
        orb(painter, rect, texture.as_ref(), &tokens, &ctx);
    };
    rows(app, sheet, (layer, id), generator, &picture, solo_id);

    if let Some(rect) = drawn_at.get() {
        enlarge_on_click(app, sheet, id, rect);
    }
}

/// The preview opens enlarged in its own window on a click: a grey outline shows it can while the pointer is on it, and that it is open
fn enlarge_on_click(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, rect: Rect) {
    let response = sheet.ui.interact(rect, sheet.ui.make_persistent_id(("generator_preview_enlarge", id)), Sense::click()).on_hover_cursor(CursorIcon::ZoomIn);
    let open = app.ui.windows.generator_preview == Some(id);
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, open, tr("Enlarge preview")));

    if response.hovered() || response.has_focus() || open {
        let tokens = Tokens::get(sheet.ui.ctx());
        sheet.ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(ENLARGE_OUTLINE_WIDTH, tokens.text_dim), StrokeKind::Inside);
    }

    if response.clicked() {
        let ctx = sheet.ui.ctx().clone();
        crate::generator_preview_window::toggle(app, &ctx, id, rect);
    }
}

/// The kind's rows, `picture` drawing the preview; `solo_id` keeps the Curvature size under the pointer for the next frame's preview
fn rows(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    (layer, id): (LayerId, u64),
    generator: &Generator,
    picture: &dyn Fn(&Painter, Rect),
    solo_id: egui::Id,
) {
    let kind = generator.kind;
    if let Some(controls) = generator.kind_controls() {
        crate::generator_kind_rows::kind_rows(app, sheet, id, generator, controls, picture);
        return;
    }

    if let Some(defaults) = WearParameters::defaults(kind) {
        // A wear generator saved before its wear controls shows them at their defaults: editing one moves it onto them
        wear_rows(app, sheet, (layer, id), generator, generator.wear.unwrap_or(defaults), picture);
        return;
    }

    if let Some(light) = generator.light {
        light_rows(app, sheet, id, Some(generator.invert), light, picture);
        return;
    }

    global_rows(app, sheet, id, generator, picture);
    if generator.on_earlier_model() {
        ramp_rows(app, sheet, id, generator);
    }

    // Saved before the section or light controls, a generator shows them at their defaults: editing one moves it onto them
    if let Some(defaults) = SectionParameters::defaults(kind) {
        let mut solo = None;
        section_rows(app, sheet, id, generator, generator.section_controls().unwrap_or(defaults), &mut solo);
        texture_card(app, sheet, (layer, id), generator.texture.as_deref().cloned().unwrap_or_default());
        micro_switch_card(app, sheet, (layer, id), generator);
        sheet.ui.data_mut(|data| data.insert_temp(solo_id, solo));
    }
    if let Some(defaults) = LightParameters::defaults(kind) {
        light_rows(app, sheet, id, None, defaults, picture);
    }
}

/// What a generator kind makes, its category and its tags (read only)
fn attributes(sheet: &mut PropertySheet<'_>, kind: GeneratorKind) {
    let (description, tags) = match kind {
        GeneratorKind::EdgeWear => (tr("Worn metal edges, scratched."), [tr("mask"), tr("generator"), tr("grayscale"), tr("curvature")]),
        GeneratorKind::FiberGlassEdgeWear => (tr("Worn fibreglass edges."), [tr("mask"), tr("generator"), tr("grayscale"), tr("curvature")]),
        GeneratorKind::Dirt => (tr("Grime in cavities."), [tr("mask"), tr("generator"), tr("grayscale"), tr("ambient occlusion")]),
        GeneratorKind::ThinAreas => (tr("Thin parts of the mesh."), [tr("mask"), tr("generator"), tr("grayscale"), tr("thickness")]),
        GeneratorKind::Curvature => (tr("Convex edges and cavities."), [tr("mask"), tr("generator"), tr("grayscale"), tr("curvature")]),
        GeneratorKind::AmbientOcclusion => (tr("Occluded areas."), [tr("mask"), tr("generator"), tr("grayscale"), tr("ambient occlusion")]),
        GeneratorKind::Position => (tr("A gradient across the mesh."), [tr("mask"), tr("generator"), tr("grayscale"), tr("position")]),
        GeneratorKind::Direction => (tr("Surfaces facing a light."), [tr("mask"), tr("generator"), tr("grayscale"), tr("normal")]),
        GeneratorKind::WorldSpaceNormals => (tr("Surfaces facing along the world's axes."), [tr("mask"), tr("generator"), tr("grayscale"), tr("normal")]),
        GeneratorKind::ThreeDDistance => (tr("A gradient around a point on the mesh."), [tr("mask"), tr("generator"), tr("grayscale"), tr("position")]),
        GeneratorKind::ThreeDLinearGradient => {
            (tr("A gradient between two points on the mesh."), [tr("mask"), tr("generator"), tr("grayscale"), tr("position")])
        }
        GeneratorKind::UvBorderDistance => (tr("A gradient in from the UV islands' borders."), [tr("mask"), tr("generator"), tr("grayscale"), tr("uv")]),
        GeneratorKind::GrayscaleConversion => (tr("An image turned gray."), [tr("mask"), tr("generator"), tr("grayscale"), tr("image")]),
        GeneratorKind::DrippingRust => {
            (tr("Rust in occluded areas, running down in drips."), [tr("mask"), tr("generator"), tr("grayscale"), tr("ambient occlusion")])
        }
        GeneratorKind::UvRandomColor => (tr("A random value per UV island."), [tr("mask"), tr("generator"), tr("grayscale"), tr("uv")]),
        GeneratorKind::UvChecker => (tr("A checker over the UV layout."), [tr("mask"), tr("generator"), tr("grayscale"), tr("uv")]),
    };
    sheet.attribute_text(tr("Description"), description);
    sheet.attribute(tr("Category"), tr("Generators"));
    sheet.attribute(tr("Tags"), &tags.join(", "));
}
