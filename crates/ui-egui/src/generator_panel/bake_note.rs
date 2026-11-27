//! One quiet line under a generator's kind when a mesh map it reads is not baked or out of date, with a Bake button

use egui::{Align, Layout, RichText, Stroke, Ui, UiBuilder, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::bake::settings_patch;
use texturecraft_engine::core::{BakeChange, BakedMap, Generator};

use crate::TextureCraftApp;
use crate::i18n::{bake_change_reason, baked_map_label, tr, trf};
use crate::property_sheet::{PropertySheet, SIDE_PADDING_POINTS};
use crate::theme::{Tokens, semibold};

/// Height of the note's line, in points (the mock's 28 px row)
const NOTE_HEIGHT: f32 = 28.0;

/// Height of the Bake button, in points (the panels' 24 px controls)
const BUTTON_HEIGHT: f32 = 24.0;

/// Size of the tooltip's title and reasons, in points
const TOOLTIP_TEXT_SIZE: f32 = 12.0;

/// The note for each mesh map `generator` reads from the active texture set that is not baked or out of date
pub(super) fn bake_notes(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, generator: &Generator) {
    let texture_set = app.session.ts();
    for map in generator.baked_maps_read() {
        if generator.inputs.contains_key(map) {
            continue;
        }

        let baked = app.session.baked(texture_set).is_some_and(|baked| baked.maps.contains_key(map));
        let changes = if baked { app.session.bake_changes(texture_set, *map) } else { Vec::new() };
        if baked && changes.is_empty() {
            continue;
        }

        // A full-width line inside the sheet's side padding
        let (line, _) = sheet.ui.allocate_exact_size(vec2(sheet.ui.available_width(), NOTE_HEIGHT), egui::Sense::hover());
        let builder = UiBuilder::new().id_salt(map).max_rect(line.shrink2(vec2(SIDE_PADDING_POINTS, 0.0))).layout(Layout::left_to_right(Align::Center));
        sheet.ui.scope_builder(builder, |ui| note(app, ui, (texture_set, *map), &changes));
    }
}

/// The line for `map` of `texture_set`: not baked when `changes` is empty, else out of date because of them
fn note(app: &mut TextureCraftApp, ui: &mut Ui, (texture_set, map): (usize, BakedMap), changes: &[BakeChange]) {
    let tokens = Tokens::get(ui.ctx());
    let name = tr(baked_map_label(map));

    let (glyph, text, title) = match changes.is_empty() {
        true => (ic::WARNING, trf("{map} map not baked", &[("map", &name)]), tr("Not baked")),
        false => (ic::CLOCK, trf("{map} map out of date", &[("map", &name)]), tr("Out of date")),
    };
    let checked = app.session.doc.project.texture_sets.get(texture_set).is_some_and(|set| set.bake.maps.is_enabled(map));

    ui.label(RichText::new(glyph).color(tokens.warning));
    let reasons = reasons(changes, checked);
    let line = ui.add(egui::Label::new(&text).truncate());
    ui.ctx().accesskit_node_builder(line.id, |node| node.set_description(reasons.join(" ")));
    line.on_hover_ui(|ui| {
        ui.label(RichText::new(title).font(semibold(TOOLTIP_TEXT_SIZE)).color(tokens.text));
        for reason in reasons {
            ui.label(RichText::new(reason).size(TOOLTIP_TEXT_SIZE).color(tokens.text_dim));
        }
    });

    let bake = ui
        .with_layout(Layout::right_to_left(Align::Center), |ui| {
            let button = egui::Button::new(tr("Bake")).stroke(Stroke::new(1.0, tokens.button_border)).min_size(vec2(0.0, BUTTON_HEIGHT));
            ui.add_enabled(app.bake.is_none(), button)
                .on_hover_text(tr("Bake this texture set's mesh maps in the background"))
                .on_disabled_hover_text(tr("A bake is already running"))
        })
        .inner;

    if bake.clicked() {
        bake_map(app, texture_set, map);
    }
}

/// Why the line shows: what changed since the bake, or what a missing map does to the generator
fn reasons(changes: &[BakeChange], checked: bool) -> Vec<&'static str> {
    if changes.is_empty() {
        let mut reasons = vec![tr("The generator reads a flat value until this map is baked.")];
        if !checked {
            reasons.push(tr("Bake turns this map on in the bake settings first."));
        }
        return reasons;
    }

    changes.iter().map(|change| tr(bake_change_reason(*change))).collect()
}

/// Bake `texture_set` in the background, turning `map` on in its bake settings first when it is off
fn bake_map(app: &mut TextureCraftApp, texture_set: usize, map: BakedMap) {
    let Some(before) = app.session.doc.project.texture_sets.get(texture_set).map(|set| set.bake.clone()) else { return };

    if !before.maps.is_enabled(map) {
        let mut after = before.clone();
        after.maps.set_enabled(map, true);
        app.run("bake.settings", json!({ "ts": texture_set, "patch": settings_patch(&before, &after) }));
    }

    app.start_bake(vec![texture_set]);
}
