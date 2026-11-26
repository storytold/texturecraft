//! Bake Mode presentation; persisted settings live in core and edits use engine commands

mod bake_checks;
mod bake_paint_brush;
mod bake_visualization_panel;
mod bake_visualization_toggles;
mod breadcrumb;
mod common_settings;
mod glyph_look;
mod live_preview;
mod live_preview_target;
mod map_preview_fold;
mod preview_input;
mod running_preview;
pub(crate) use live_preview::LivePreview;
mod section_opening;
pub(crate) use bake_checks::bake_warnings;
pub use bake_paint_brush::BakePaintBrush;
pub(crate) use bake_visualization_panel::bake_visualization_panel;
pub use bake_visualization_toggles::BakeVisualizationToggles;
mod controls;
mod map_settings;
mod map_thumbnails;
mod mesh_map_bakers;
pub use map_thumbnails::MapThumbnails;
mod set_picker;
mod set_picker_purpose;
mod set_picker_row;

use egui::{Ui, vec2};
use egui_phosphor::regular as icon;
use serde_json::{Value, json};
use texturecraft_engine::Session;
use texturecraft_engine::bake::settings_patch;
use texturecraft_engine::core::{BakeSettings, BakeSyncGroup, BakedMap, CageType};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::property_sheet::PropertySheet;
use crate::state::Mode;
use crate::widgets::{icon_button, primary_button};

/// Whether the high poly shows its matching error for `session`'s selected texture set: the automatic cage has no distances to fall short, as in the reference
pub(crate) fn matching_error_applies(session: &Session) -> bool {
    session.doc.project.texture_sets.get(session.ts()).is_none_or(|set| set.bake.cage.kind != CageType::Automatic)
}

pub fn texture_sets(app: &mut TextureCraftApp, ui: &mut Ui) {
    egui::ScrollArea::vertical().id_salt("bake_texture_sets").auto_shrink([false; 2]).show(ui, |ui| {
        for set_index in 0..app.session.set_count() {
            let Some(texture_set) = app.session.doc.project.texture_sets.get(set_index) else { continue };

            let name = texture_set.name.clone();
            let resolution = texture_set.resolution;
            let mut is_enabled = texture_set.bake.common.is_enabled;
            let visible = texture_set.visible;
            let tiles: Vec<_> = texture_set
                .uv_tiles
                .iter()
                .map(|tile| (*tile, texture_set.tile_resolutions.get(tile).copied(), !texture_set.bake_excluded_tiles.contains(tile)))
                .collect();

            ui.push_id(set_index, |ui| {
                let row = ui.horizontal(|ui| {
                    let button_size = ui.spacing().interact_size.y;
                    if icon_button(ui, if visible { icon::EYE } else { icon::EYE_SLASH }, tr("Show or hide the texture set"), false, button_size).clicked() {
                        app.run("textureSet.visible", json!({ "ts": set_index }));
                    }
                    if ui.checkbox(&mut is_enabled, "").on_hover_text(tr("Include in Bake")).changed() {
                        app.run("bake.settings", json!({ "ts": set_index, "enabled": is_enabled }));
                    }

                    // The name takes the room the resolution leaves, so it only shortens when the row is too narrow
                    let resolution_text = format!("{resolution}x{resolution}");
                    let name_width = (ui.available_width() - resolution_menu_width(ui, &resolution_text) - ui.spacing().item_spacing.x).max(0.0);
                    let name_size = egui::vec2(name_width, ui.spacing().interact_size.y);
                    let name_button = ui.allocate_ui_with_layout(name_size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(name_width);
                        ui.add(egui::Button::selectable(set_index == app.session.ts(), &name).truncate())
                    });
                    if name_button.inner.on_hover_text(&name).clicked() {
                        app.run("textureSet.select", json!({ "ts": set_index }));
                    }
                    if let Some(selected) = resolution_menu(ui, tr("Texture set resolution"), resolution_text, resolution, None)
                        && selected != resolution
                    {
                        app.start_resize(set_index, selected);
                    }
                });

                if ui.is_enabled() && ui.rect_contains_pointer(row.response.rect) {
                    app.next_hovered_texture_set = Some((app.session.project_id, set_index));
                }

                if !tiles.is_empty() {
                    egui::CollapsingHeader::new(format!("{} ({})", tr("UV tiles (UDIM)"), tiles.len())).default_open(true).show(ui, |ui| {
                        for (tile, override_resolution, enabled) in &tiles {
                            let mut enabled = *enabled;
                            ui.push_id(tile, |ui| {
                                ui.horizontal(|ui| {
                                    if ui.checkbox(&mut enabled, tile.to_string()).on_hover_text(tr("Include in Bake")).changed() {
                                        app.run("bake.tile.set", json!({"ts": set_index, "tile": tile, "enabled": enabled}));
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let text = override_resolution.map(|value| format!("{value}x{value}*")).unwrap_or_default();
                                        let label = trf("UV tile {tile} resolution", &[("tile", &tile.to_string())]);
                                        if let Some(selected) = resolution_menu(
                                            ui,
                                            &label,
                                            text,
                                            override_resolution.unwrap_or(resolution),
                                            override_resolution.map(|_| resolution),
                                        ) {
                                            app.run("textureSet.tile.resolution", json!({"ts": set_index, "tile": tile, "resolution": selected}));
                                        }
                                    });
                                })
                            });
                        }
                    });
                }
            });
        }
    });
}

/// How wide [`resolution_menu`] draws `text`, with a button's padding on both sides
fn resolution_menu_width(ui: &Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter().layout_no_wrap(text.to_owned(), font, ui.visuals().text_color()).size().x + ui.spacing().button_padding.x * 2.0
}

fn resolution_menu(ui: &mut Ui, label: &str, text: String, resolution: u32, inherited_resolution: Option<u32>) -> Option<u32> {
    let effective_size = format!("{resolution}x{resolution}");
    let measured = if text.is_empty() { &effective_size } else { &text };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let width = resolution_menu_width(ui, measured);
    let response = ui.add_sized([width, ui.spacing().interact_size.y], egui::Button::new("").frame(false));
    let popup_id = response.id.with("resolution_menu");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id) || response.clicked();
    let shown = if text.is_empty() && (response.hovered() || open) { effective_size } else { text };
    ui.painter().text(response.rect.center(), egui::Align2::CENTER_CENTER, &shown, font, ui.style().interact(&response).text_color());

    let mut selected = None;
    egui::Popup::menu(&response).id(popup_id).show(|ui| {
        if let Some(inherited) = inherited_resolution {
            if ui.button(tr("Remove resolution override")).clicked() {
                selected = Some(inherited);
                ui.close();
            }
            ui.separator();
        }
        for candidate in [128, 256, 512, 1024, 2048, 4096, 8192] {
            if ui.selectable_label(candidate == resolution, format!("{candidate}x{candidate}")).clicked() {
                selected = Some(candidate);
            }
        }
    });
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), label);
        info.current_text_value = Some(shown.clone());
        info
    });
    response.on_hover_text(label);
    selected
}

/// The Mesh Map Bakers panel: which maps bake and which one Mesh Map Settings shows
pub fn mesh_map_bakers_panel(app: &mut TextureCraftApp, ui: &mut Ui) {
    let texture_set = json!(app.session.ts());
    edit_settings(app, ui, "mesh_map_bakers", texture_set, |app, ui, settings| {
        mesh_map_bakers::show(app, ui, settings);
        None
    });
}

/// The Mesh Map Settings panel: the breadcrumb, then the selected map's parameters
pub fn mesh_map_settings_panel(app: &mut TextureCraftApp, ui: &mut Ui) {
    let Some(texture_set) = app.session.doc.project.texture_sets.get(app.session.ts()) else { return };

    let selected = selected_map(app.ui.selected_baked_map, &texture_set.bake);
    let restore = ui.horizontal(|ui| {
        ui.add_space(crate::property_sheet::SIDE_PADDING_POINTS);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - crate::property_sheet::SIDE_PADDING_POINTS);
            breadcrumb::show(app, ui, selected, map_settings::has_parameters(selected))
        })
        .inner
    });
    let texture_sets = json!(app.session.doc.project.bake_synced_sets(app.session.ts(), BakeSyncGroup::Map(selected)));
    let changed = edit_settings(app, ui, "mesh_map_settings", texture_sets, |_, ui, settings| {
        PropertySheet::show(ui, ("mesh_map_settings_sheet", selected), |sheet| map_settings::show(sheet, selected, &mut settings.maps, restore.inner))
    });

    // Other changes reach the preview through its check of what its picture was made from
    if changed && ui.ctx().dragged_id().is_some() {
        let ctx = ui.ctx().clone();
        live_preview::slider_dragged(app, &ctx);
    }
}

/// Once a frame: the map preview follows the selected map and takes its finished bakes
pub(crate) fn poll_live_preview(app: &mut TextureCraftApp, ctx: &egui::Context) {
    live_preview::poll(app, ctx);
}

/// The Common Settings panel: output, high poly, cage and skew settings
pub fn common_settings_panel(app: &mut TextureCraftApp, ui: &mut Ui) {
    let texture_sets = json!(app.session.doc.project.bake_synced_sets(app.session.ts(), BakeSyncGroup::Common));
    edit_settings(app, ui, "bake_common_settings", texture_sets, |app, ui, settings| {
        PropertySheet::show(ui, "bake_common_sheet", |sheet| common_settings::show(app, sheet, settings))
    });
}

/// The map whose settings show: the chosen one, else the first enabled map, else Normal
pub(super) fn selected_map(selection: Option<BakedMap>, settings: &BakeSettings) -> BakedMap {
    selection.or_else(|| settings.maps.enabled().next()).unwrap_or(BakedMap::Normal)
}

/// Send changed fields and explicitly reset sections through the same undoable command
fn edit_settings(
    app: &mut TextureCraftApp,
    ui: &mut Ui,
    scroll_id: &str,
    texture_sets: Value,
    contents: impl FnOnce(&mut TextureCraftApp, &mut Ui, &mut BakeSettings) -> Option<Value>,
) -> bool {
    let set_index = app.session.ts();
    let Some(before) = app.session.doc.project.texture_sets.get(set_index).map(|texture_set| texture_set.bake.clone()) else { return false };

    let mut settings = before.clone();
    let reset = egui::ScrollArea::vertical().id_salt(scroll_id).auto_shrink([false; 2]).show(ui, |ui| contents(app, ui, &mut settings)).inner;
    if settings == before && reset.is_none() {
        return false;
    }

    let mut patch = settings_patch(&before, &settings);
    if let (Some(fields), Some(Value::Object(reset))) = (patch.as_object_mut(), reset) {
        fields.extend(reset);
    }
    app.run("bake.settings", json!({ "ts": texture_sets, "patch": patch }));
    true
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

/// How many maps baking `texture_sets` makes, counted as the reference's Bake button counts them: checked maps × included UV tiles, per set
fn maps_to_bake(app: &TextureCraftApp, texture_sets: &[usize]) -> usize {
    texture_sets
        .iter()
        .filter_map(|index| app.session.doc.project.texture_sets.get(*index))
        .map(|set| {
            let tiles = if set.uv_tiles.is_empty() { 1 } else { set.uv_tiles.iter().filter(|tile| !set.bake_excluded_tiles.contains(tile)).count() };
            set.bake.maps.enabled().count() * tiles
        })
        .sum()
}

/// The bake bar's gap from the bottom of the 3D view, in points (the pre-bake warnings sit above it)
pub(super) const BAKE_BAR_MARGIN: f32 = 16.0;

pub(crate) fn bake_bar(app: &mut TextureCraftApp, ctx: &egui::Context, viewport: egui::Rect) {
    egui::Area::new(egui::Id::unique("bake_bar"))
        .constrain_to(viewport)
        .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -BAKE_BAR_MARGIN))
        .default_width(0.0)
        .show(ctx, |ui| {
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
                        .filter(|(_, texture_set)| texture_set.bake.common.is_enabled && texture_set.has_enabled_bake_tiles())
                        .map(|(index, _)| index)
                        .collect();
                    let maps = maps_to_bake(app, &texture_sets);
                    let label = trf("Bake ({n})", &[("n", &maps)]);
                    if ui.add_enabled(maps > 0, primary_button(ui, &label)).clicked() {
                        app.start_bake(texture_sets);
                    }
                }

                if ui.button(tr("Return to Painting Mode")).clicked() {
                    app.ui.mode = Mode::Painting;
                }
            });
        });
}
