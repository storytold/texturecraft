//! Bake Mode presentation; persisted settings live in core and edits use engine commands

mod bake_paint_brush;
mod breadcrumb;
mod common_settings;
pub use bake_paint_brush::BakePaintBrush;
mod controls;
mod map_settings;
mod mesh_map_bakers;

use egui::{Ui, vec2};
use serde_json::json;
use texturecraft_engine::core::{BakeSettings, BakedMap};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::state::Mode;
use crate::widgets::primary_button;

pub fn texture_sets(app: &mut TextureCraftApp, ui: &mut Ui) {
    egui::ScrollArea::vertical().id_salt("bake_texture_sets").auto_shrink([false; 2]).show(ui, |ui| {
        for set_index in 0..app.session.set_count() {
            let Some(texture_set) = app.session.doc.project.texture_sets.get(set_index) else { continue };

            let name = texture_set.name.clone();
            let (width, height) = texture_set.bake.common.output_size();
            let mut is_enabled = texture_set.bake.common.is_enabled;
            ui.push_id(set_index, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut is_enabled, "").on_hover_text(tr("Include in Bake")).changed() {
                        app.run("bake.settings", json!({ "ts": set_index, "enabled": is_enabled }));
                    }
                    if ui.selectable_label(set_index == app.session.ts(), format!("{name}  ({width} × {height})")).clicked() {
                        app.run("textureSet.select", json!({ "ts": set_index }));
                    }
                });
            });
        }
    });
}

/// The Mesh Map Bakers panel: which maps bake and which one Mesh Map Settings shows
pub fn mesh_map_bakers_panel(app: &mut TextureCraftApp, ui: &mut Ui) {
    edit_settings(app, ui, "mesh_map_bakers", mesh_map_bakers::show);
}

/// The Mesh Map Settings panel: the breadcrumb, then the selected map's parameters
pub fn mesh_map_settings_panel(app: &mut TextureCraftApp, ui: &mut Ui) {
    let Some(texture_set) = app.session.doc.project.texture_sets.get(app.session.ts()) else { return };

    let selected = selected_map(app.ui.selected_baked_map, &texture_set.bake);
    breadcrumb::show(app, ui, selected);
    ui.separator();
    edit_settings(app, ui, "mesh_map_settings", |_, ui, settings| ui.push_id(selected, |ui| map_settings::show(ui, selected, &mut settings.maps)).inner);
}

/// The Common Settings panel: output, high poly, cage and skew settings
pub fn common_settings_panel(app: &mut TextureCraftApp, ui: &mut Ui) {
    edit_settings(app, ui, "bake_common_settings", common_settings::show);
}

/// The map whose settings show: the chosen one, else the first enabled map, else Normal
fn selected_map(selection: Option<BakedMap>, settings: &BakeSettings) -> BakedMap {
    selection.or_else(|| settings.maps.enabled().next()).unwrap_or(BakedMap::Normal)
}

/// Draw `contents` over a copy of the active set's bake settings and run `bake.settings` on change
fn edit_settings(app: &mut TextureCraftApp, ui: &mut Ui, scroll_id: &str, contents: impl FnOnce(&mut TextureCraftApp, &mut Ui, &mut BakeSettings)) {
    let set_index = app.session.ts();
    let Some(before) = app.session.doc.project.texture_sets.get(set_index).map(|texture_set| texture_set.bake.clone()) else { return };

    let mut settings = before.clone();
    egui::ScrollArea::vertical().id_salt(scroll_id).auto_shrink([false; 2]).show(ui, |ui| contents(app, ui, &mut settings));
    if settings != before {
        app.run("bake.settings", json!({ "ts": set_index, "model": settings }));
    }
}

/// What the 3D view shows while a bake runs: the newest finished map, one finished map or the material
fn preview_control(app: &mut TextureCraftApp, ui: &mut Ui) {
    use texturecraft_engine::PreviewShow;
    let Some(preview) = app.session.bake_preview.as_ref() else { return };
    let show = preview.show;
    // Finished maps of the set baking now can be pinned; the others are still neutral
    let rows: Vec<(BakedMap, bool)> =
        preview.current_set().map(|set| set.order.iter().map(|&map| (map, set.baked.maps.contains_key(&map))).collect()).unwrap_or_default();
    let text = match show {
        PreviewShow::Follow => tr("Follow Newest Map").to_owned(),
        PreviewShow::Material => tr("Material").to_owned(),
        PreviewShow::Pin(map) => trf("Mesh map: {map}", &[("map", &tr(crate::i18n::baked_map_label(map)))]),
    };
    let mut choice = None;
    egui::ComboBox::from_id_salt("bake_preview_show")
        .selected_text(text)
        .show_ui(ui, |ui| {
            if ui.selectable_label(show == PreviewShow::Follow, tr("Follow Newest Map")).clicked() {
                choice = Some(PreviewShow::Follow);
            }
            for (map, done) in rows {
                let label = trf("Mesh map: {map}", &[("map", &tr(crate::i18n::baked_map_label(map)))]);
                if ui.add_enabled(done, egui::Button::selectable(show == PreviewShow::Pin(map), label)).clicked() {
                    choice = Some(PreviewShow::Pin(map));
                }
            }
            if ui.selectable_label(show == PreviewShow::Material, tr("Material")).clicked() {
                choice = Some(PreviewShow::Material);
            }
        })
        .response
        .on_hover_text(tr(
            "What the 3D view shows while the bake runs: the newest finished map, one finished map, or the material. Maps not baked yet show a neutral value.",
        ));
    if let Some(show) = choice {
        app.run("bake.preview", json!({ "show": show.id() }));
    }
}

pub(crate) fn bake_bar(app: &mut TextureCraftApp, ctx: &egui::Context, viewport: egui::Rect) {
    egui::Area::new(egui::Id::unique("bake_bar")).constrain_to(viewport).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -16.0)).default_width(0.0).show(
        ctx,
        |ui| {
            if let Some(status) = app.bake_status() {
                ui.vertical_centered(|ui| ui.label(egui::RichText::new(status).strong()));
            }
            ui.horizontal(|ui| {
                if let Some(progress) = app.bake_progress() {
                    ui.add(egui::ProgressBar::new(progress).desired_width(220.0).show_percentage());
                    if ui.button(tr("Cancel")).clicked() {
                        app.cancel_bake();
                    }
                    preview_control(app, ui);
                } else {
                    let texture_sets: Vec<usize> = app
                        .session
                        .doc
                        .project
                        .texture_sets
                        .iter()
                        .enumerate()
                        .filter(|(_, texture_set)| texture_set.bake.common.is_enabled)
                        .map(|(index, _)| index)
                        .collect();
                    let label = trf("Bake Selected Textures ({n})", &[("n", &texture_sets.len())]);
                    if ui.add_enabled(!texture_sets.is_empty(), primary_button(ui, &label)).clicked() {
                        app.start_bake(texture_sets);
                    }
                }

                if ui.button(tr("Return to Painting Mode")).clicked() {
                    app.ui.mode = Mode::Painting;
                }
            });
        },
    );
}
