//! A generator's image input as a slot: what it reads, a picker to replace it with a value, an anchor point or an image, and the way back

use egui::{Painter, Rect};
use serde_json::json;
use texturecraft_engine::core::{LayerId, Source};

use crate::TextureCraftApp;
use crate::dock::{anchor_label, image_drop, mesh_map_color, project_effect_anchors_below};
use crate::fill_material_panel::{source_choice, source_tabs};
use crate::generator_input_slot::GeneratorInputSlot;
use crate::i18n::{baked_map_label, tr, trf};
use crate::property_sheet::PropertySheet;
use crate::resource_picker;
use crate::source_choice::SourceChoice;
use crate::theme::Tokens;

/// Glyph size on a slot picture standing for an anchor point
const GLYPH_SIZE: f32 = 12.0;

/// The slot of input `slot` of generator effect `id` on `layer`, holding `source`: a card of its own, or a row of the card open now when `in_card`
pub(crate) fn input_card(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    (layer, id): (LayerId, u64),
    slot: GeneratorInputSlot,
    source: Option<&Source>,
    in_card: bool,
) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let input = slot.id();
    let line = match (source, slot) {
        (None, GeneratorInputSlot::Map(map)) => trf("Mesh map: {map}", &[("map", &tr(baked_map_label(map)))]),
        (None, _) => tr("Empty").to_string(),
        (Some(Source::Uniform(_)), _) => tr("Uniform value").to_string(),
        (Some(Source::Image { name, .. }), _) => trf("Image: {name}", &[("name", name)]),
        (Some(Source::Anchor { anchor, channel, .. }), _) => anchor_label(app, *anchor, *channel),
        (Some(Source::Procedural { .. }), _) => tr("Procedural").to_string(),
    };

    let image = match source {
        Some(Source::Image { name, .. }) => app.session.images.get(name).map(|image| app.thumbs.image(sheet.ui.ctx(), name, image)),
        _ => None,
    };
    let baked = match slot {
        GeneratorInputSlot::Map(map) => app.session.doc.baked.get(&app.session.ts()).is_some_and(|baked| baked.maps.contains_key(&map)),
        _ => false,
    };
    let picture = |painter: &Painter, rect: Rect| {
        let gray = |value: f32| egui::Color32::from_gray((value.clamp(0.0, 1.0) * 255.0).round() as u8);
        match (source, slot) {
            (Some(Source::Image { .. }), _) => match &image {
                Some(texture) => {
                    painter.image(texture.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                None => {
                    painter.rect_filled(rect, 0.0, tokens.button);
                }
            },
            (Some(Source::Uniform(value)), _) => {
                painter.rect_filled(rect, 0.0, gray(value[0]));
            }
            (Some(Source::Anchor { .. } | Source::Procedural { .. }), _) => {
                painter.rect_filled(rect, 0.0, tokens.button);
                let glyph = egui_phosphor::regular::ANCHOR;
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(GLYPH_SIZE), tokens.text_dim);
            }
            (None, GeneratorInputSlot::Map(map)) => {
                painter.rect_filled(rect, 0.0, if baked { mesh_map_color(map) } else { tokens.button });
            }
            (None, _) => {
                painter.rect_filled(rect, 0.0, tokens.button);
            }
        }
    };

    // A mesh map input goes back to its map with ⟲; an image slot empties with ×
    let map_slot = matches!(slot, GeneratorInputSlot::Map(_));
    let response = match (in_card, map_slot) {
        (true, _) => sheet.slot_in_card(&picture, slot.title(), &line, source.is_some()),
        (false, true) => sheet.slot(&picture, slot.title(), &line, None, false, source.is_some()),
        (false, false) => sheet.slot(&picture, slot.title(), &line, None, source.is_some(), false),
    };
    if response.clear || response.reset {
        app.run("effect.set", json!({"effect": id, "input": input, "inputBaked": true}));
    }
    if let Some(name) = image_drop(sheet.ui, &response.pick) {
        app.run("effect.set", json!({"effect": id, "input": input, "inputImage": name}));
    }

    let popup = sheet.ui.make_persistent_id(("generator_input", id, input));
    if response.pick.clicked() {
        resource_picker::toggle(sheet.ui.ctx(), popup);
    }
    let current = source.map(|source| source_choice(&app.session, source));
    let tabs = || {
        let anchors = if slot.takes_anchor() { project_effect_anchors_below(&app.session.doc.project, layer, LayerId(id)) } else { Vec::new() };
        let mut tabs = source_tabs(&app.session, &anchors, tokens.button, false);

        // A generator's inputs read images, values and anchor points, never a procedural
        for tab in &mut tabs {
            for group in &mut tab.groups {
                group.items.retain(|item| !matches!(item.id, SourceChoice::Procedural(_)));
            }
            tab.groups.retain(|group| !group.items.is_empty());
        }
        tabs.retain(|tab| !tab.groups.is_empty());
        tabs
    };
    let picked = resource_picker::show(&response.pick, popup, &mut app.thumbs, current.as_ref(), current.is_some(), tabs);
    match picked.filter(|choice| Some(choice) != current.as_ref()) {
        Some(SourceChoice::Uniform) => {
            app.run("effect.set", json!({"effect": id, "input": input, "inputValue": slot.neutral()}));
        }
        Some(SourceChoice::Image(name)) => {
            app.run("effect.set", json!({"effect": id, "input": input, "inputImage": name}));
        }
        Some(SourceChoice::Anchor(anchor)) => {
            app.run("effect.set", json!({"effect": id, "input": input, "inputAnchor": anchor.0}));
        }
        Some(SourceChoice::Import) => crate::menus::invoke(app, sheet.ui.ctx(), "resource.import"),
        Some(SourceChoice::Procedural(_) | SourceChoice::Alpha(_)) | None => {}
    }

    if let Some(Source::Uniform(value)) = source {
        let mut gray = value[0];
        let edit = sheet.ramp(tr("Value"), &mut gray, 0.0..=1.0);
        if edit.invert {
            app.run("effect.set", json!({"effect": id, "input": input, "inputValue": 1.0 - gray}));
        } else if let Some(merge) = edit.changed {
            app.run("effect.set", json!({"effect": id, "input": input, "inputValue": gray, "merge": merge}));
        }
    }
    if !in_card {
        sheet.close_card();
    }
}
