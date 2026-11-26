use egui::{Align, CornerRadius, FontId, Layout, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use egui_phosphor::regular as icon;
use serde_json::json;
use texturecraft_engine::ViewMode;
use texturecraft_engine::core::{BakeSettings, BakeSyncGroup, BakedMap};

use super::glyph_look::GlyphLook;
use super::set_picker::SetPicker;
use super::set_picker_purpose::SetPickerPurpose;
use crate::TextureCraftApp;
use crate::docking::Panel;
use crate::i18n::{baked_map_label, tr, trf};
use crate::property_sheet::{CHECK_SIZE, paint_check};
use crate::theme::{Tokens, semibold};

/// The presets menu's least width, and its name field's, room for a typical preset name
const PRESETS_MENU_MIN_WIDTH: f32 = 220.0;
const PRESET_NAME_FIELD_WIDTH: f32 = 130.0;

/// How far past its box a row's check takes clicks, so it is easy to hit
const CHECK_HIT_AIR: f32 = 3.0;

/// Height of the header's buttons, the air inside them beside their words, and between the glyph and the words
const HEADER_BUTTON_HEIGHT: f32 = 24.0;
const HEADER_BUTTON_PADDING: f32 = 10.0;
const HEADER_GLYPH_GAP: f32 = 6.0;
/// Air above the header and between its buttons
const HEADER_AIR: f32 = 6.0;
const HEADER_GAP: f32 = 6.0;
/// Height of one baker row, and of the Check All row
const ROW_HEIGHT: f32 = 28.0;
/// Inset of a row's check from its left edge
const ROW_INSET: f32 = 6.0;
/// Side of a row's map thumbnail, and the air on its sides
const THUMBNAIL_SIZE: f32 = 22.0;
const THUMBNAIL_AIR: f32 = 9.0;
/// Side of a row's icon buttons and of the list menu button
const ROW_BUTTON_SIZE: f32 = 20.0;
/// Size of a row's glyphs
const ROW_GLYPH_SIZE: f32 = 14.0;
/// Size of the map names
const NAME_SIZE: f32 = 12.0;
/// Corner radius of a row's face and of a held-down glyph's box
const ROW_RADIUS: u8 = 4;
/// Opacity of a hovered row's face, against the selected row's full one
const HOVER_FACE_OPACITY: f32 = 0.5;
/// Air above and below the hairline under Check All
const RULE_AIR: f32 = 4.0;

/// The Common Settings button (with the sync icon when there are several texture sets), Check All with the list menu, and one row per map (checkbox enables, click selects)
pub(super) fn show(app: &mut TextureCraftApp, ui: &mut Ui, settings: &mut BakeSettings) {
    let several_sets = app.session.doc.project.texture_sets.len() > 1;
    ui.add_space(HEADER_AIR);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = HEADER_GAP;
        if header_button(ui, icon::GEAR, tr("Common Settings")).clicked() {
            app.ui.dock.focus(Panel::CommonSettings);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if several_sets {
                let shares = settings.sync.shares(BakeSyncGroup::Common);
                let look = if shares { GlyphLook::Lit } else { GlyphLook::Plain };
                let (slot, _) = ui.allocate_exact_size(vec2(ROW_BUTTON_SIZE, ROW_BUTTON_SIZE), Sense::hover());
                let glyph = if shares { icon::LINK } else { icon::LINK_BREAK };
                let sync = glyph_button(ui, slot, glyph, tr("Synchronize Common Settings"), look);
                if sync.clicked() {
                    SetPicker::open(app, &sync, SetPickerPurpose::Sync(BakeSyncGroup::Common));
                }
            }
            presets_menu(app, ui, settings);
        });
    });

    SetPicker::show(app, &ui.ctx().clone());

    ui.add_space(RULE_AIR);
    check_all_row(app, ui, settings);
    ui.add_space(RULE_AIR);
    let rule = ui.available_rect_before_wrap();
    ui.painter().hline(rule.x_range(), rule.top(), Stroke::new(1.0, Tokens::get(ui.ctx()).separator));
    ui.add_space(RULE_AIR);

    let selected = super::selected_map(app.ui.selected_baked_map, settings);
    let check_drag_id = ui.scope_id().with("check_drag");
    let drag_from_id = ui.scope_id().with("check_drag_from");
    let mut check_drag: Option<bool> = ui.data(|data| data.get_temp(check_drag_id));
    let drag_from: Option<Pos2> = ui.data(|data| data.get_temp(drag_from_id));
    for map in BakedMap::ALL {
        if ui.push_id(map, |ui| row(app, ui, map, settings, map == selected, &mut check_drag, drag_from)).inner {
            app.ui.selected_baked_map = Some(map);
            app.ui.dock.focus(Panel::MeshMapSettings);
        }
    }

    // Keep the drag state until the release click has been handled
    let is_held = ui.input(|input| input.pointer.primary_down());
    let pointer = ui.input(|input| input.pointer.latest_pos());
    ui.data_mut(|data| {
        if let (Some(value), true) = (check_drag, is_held) {
            data.insert_temp(check_drag_id, value);

            // A pointer back from outside the window starts a new line, so rows it never crossed stay as they were
            if let Some(pointer) = pointer {
                data.insert_temp(drag_from_id, pointer);
            } else {
                data.remove_temp::<Pos2>(drag_from_id);
            }
        } else {
            data.remove_temp::<bool>(check_drag_id);
            data.remove_temp::<Pos2>(drag_from_id);
        }
    });
}

/// The Presets menu: save the active set's settings under a name, load or delete a saved one, copy to all sets
fn presets_menu(app: &mut TextureCraftApp, ui: &mut Ui, settings: &BakeSettings) {
    let (slot, _) = ui.allocate_exact_size(vec2(ROW_BUTTON_SIZE, ROW_BUTTON_SIZE), Sense::hover());
    let button = glyph_button(ui, slot, icon::SLIDERS_HORIZONTAL, tr("Presets"), GlyphLook::Plain);
    egui::Popup::menu(&button).align(egui::RectAlign::BOTTOM_END).align_alternatives(&[egui::RectAlign::BOTTOM_START]).show(|ui| {
        ui.set_min_width(PRESETS_MENU_MIN_WIDTH);
        ui.horizontal(|ui| {
            let name_edit = egui::TextEdit::singleline(&mut app.ui.bake_preset_name).hint_text(tr("Preset name")).desired_width(PRESET_NAME_FIELD_WIDTH);
            let enter = ui.add(name_edit).lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let can_save = !app.ui.bake_preset_name.trim().is_empty();
            if (ui.add_enabled(can_save, egui::Button::new(tr("Save Preset"))).clicked() || (enter && can_save))
                && app.ui.save_bake_preset(&app.ui.bake_preset_name.clone(), settings)
            {
                app.ui.bake_preset_name.clear();
            }
        });

        ui.separator();
        if app.ui.bake_presets.is_empty() {
            ui.weak(tr("No saved presets"));
        }

        let mut load = None;
        let mut delete = None;
        for preset in &app.ui.bake_presets {
            ui.horizontal(|ui| {
                if ui.button(&preset.name).on_hover_text(tr("Load this preset into the active texture set")).clicked() {
                    load = Some(preset.settings.clone());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = trf("Delete preset {name}", &[("name", &preset.name)]);
                    if crate::widgets::icon_button(ui, icon::TRASH, &label, false, ui.spacing().interact_size.y).clicked() {
                        delete = Some(preset.name.clone());
                    }
                });
            });
        }

        if let Some(preset) = load {
            app.run("bake.settings", crate::bake_presets::apply_params(json!(app.session.ts()), &preset));
            ui.close();
        }
        if let Some(name) = delete {
            app.ui.delete_bake_preset(&name);
        }

        ui.separator();
        if ui
            .button(tr("Apply to All Texture Sets"))
            .on_hover_text(tr("Copy the active texture set's bake settings to every texture set. Each keeps its Include in Bake choice."))
            .clicked()
        {
            app.run("bake.settings", crate::bake_presets::apply_params(json!("all"), settings));
            ui.close();
        }
    });
}

/// Check All (mixed while only some maps are checked; a click checks them all, or unchecks them when all are) with the ≡ menu at the right
fn check_all_row(app: &mut TextureCraftApp, ui: &mut Ui, settings: &mut BakeSettings) {
    let tokens = Tokens::get(ui.ctx());
    let enabled_count = BakedMap::ALL.into_iter().filter(|map| settings.maps.is_enabled(*map)).count();
    let all_enabled = enabled_count == BakedMap::ALL.len();
    let state = match enabled_count {
        0 => Some(false),
        _ if all_enabled => Some(true),
        _ => None,
    };

    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let label = tr("Check All");
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(NAME_SIZE), tokens.text);
    let check = Rect::from_center_size(pos2(rect.left() + ROW_INSET + CHECK_SIZE / 2.0, rect.center().y), vec2(CHECK_SIZE, CHECK_SIZE));
    let hit = Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(check.right() + THUMBNAIL_AIR + galley.size().x, rect.bottom()));
    let response = ui.interact(hit, ui.scope_id().with("check_all"), Sense::click());
    response.widget_info(|| match state {
        Some(on) => egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, label),
        None => egui::WidgetInfo::labeled(egui::Role::CheckBox, true, label),
    });
    paint_check(ui.painter(), &tokens, check, state, response.hovered());
    ui.painter().galley(pos2(check.right() + THUMBNAIL_AIR, rect.center().y - galley.size().y / 2.0), galley, tokens.text);
    crate::property_sheet::paint_keyboard_focus(ui, &response, &tokens);
    if response.clicked() {
        for map in BakedMap::ALL {
            settings.maps.set_enabled(map, !all_enabled);
        }
    }

    let menu = Rect::from_center_size(pos2(rect.right() - ROW_BUTTON_SIZE / 2.0, rect.center().y), vec2(ROW_BUTTON_SIZE, ROW_BUTTON_SIZE));
    let mut menu_ui = ui.new_child(UiBuilder::new().max_rect(menu));
    list_menu(app, &mut menu_ui, settings);
}

/// The ≡ menu beside Check All: check states, and copying them or every setting to other texture sets
fn list_menu(app: &mut TextureCraftApp, ui: &mut Ui, settings: &mut BakeSettings) {
    let button = glyph_button(ui, ui.max_rect(), icon::LIST, tr("Mesh Map Bakers Menu"), GlyphLook::Plain);
    egui::Popup::menu(&button).align(egui::RectAlign::BOTTOM_END).align_alternatives(&[egui::RectAlign::BOTTOM_START]).show(|ui| {
        if ui.button(tr("Check All Mesh Maps")).clicked() {
            BakedMap::ALL.into_iter().for_each(|map| settings.maps.set_enabled(map, true));
        }
        if ui.button(tr("Uncheck All Mesh Maps")).clicked() {
            BakedMap::ALL.into_iter().for_each(|map| settings.maps.set_enabled(map, false));
        }
        if ui.button(tr("Invert Checked Mesh Maps")).clicked() {
            BakedMap::ALL.into_iter().for_each(|map| settings.maps.set_enabled(map, !settings.maps.is_enabled(map)));
        }
        copy_items(app, ui, &button, "checked", None, [tr("Apply Checked to More Texture Sets..."), tr("Apply Checked to All Texture Sets")]);
        ui.separator();
        copy_items(app, ui, &button, "settings", None, [tr("Sync All Settings to More Texture Sets..."), tr("Sync All Settings to All Texture Sets")]);
    });
}

/// A copy's two menu items: "...more Texture Sets..." opens the set picker, "...all Texture Sets" copies to every set
fn copy_items(app: &mut TextureCraftApp, ui: &mut Ui, opener: &egui::Response, what: &'static str, map: Option<BakedMap>, [more, all]: [&str; 2]) {
    // With one texture set there is nothing to copy to
    let several_sets = app.session.doc.project.texture_sets.len() > 1;
    if ui.add_enabled(several_sets, egui::Button::new(more)).clicked() {
        SetPicker::open(app, opener, SetPickerPurpose::Copy { what, map });
        ui.close();
    }
    if ui.add_enabled(several_sets, egui::Button::new(all)).clicked() {
        let from = app.session.ts();
        app.run("bake.copy", json!({ "what": what, "map": map.map(BakedMap::id), "from": from, "ts": "all" }));
        ui.close();
    }
}

/// One baker row (`check_drag`: the state a pressed check gives those the pointer then crosses; `drag_from`: where that drag's pointer was last frame); returns whether the row itself was clicked
fn row(
    app: &mut TextureCraftApp,
    ui: &mut Ui,
    map: BakedMap,
    settings: &mut BakeSettings,
    is_selected: bool,
    check_drag: &mut Option<bool>,
    drag_from: Option<Pos2>,
) -> bool {
    let several_sets = app.session.doc.project.texture_sets.len() > 1;
    let tokens = Tokens::get(ui.ctx());
    let name = tr(baked_map_label(map));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, is_selected, name));

    let hovered = ui.rect_contains_pointer(rect);
    if is_selected {
        ui.painter().rect_filled(rect, CornerRadius::same(ROW_RADIUS), tokens.row_hover);
    } else if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(ROW_RADIUS), tokens.row_hover.gamma_multiply(HOVER_FACE_OPACITY));
    }

    // Added after the row's own response, so the check and icons take the clicks over it
    let check = Rect::from_center_size(pos2(rect.left() + ROW_INSET + CHECK_SIZE / 2.0, rect.center().y), vec2(CHECK_SIZE, CHECK_SIZE));
    let checkbox = ui.interact(check.expand(CHECK_HIT_AIR), ui.scope_id().with("include"), Sense::click()).on_hover_text(tr("Include in Bake"));
    checkbox.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, settings.maps.is_enabled(map), name));
    let is_dragged_over = drag_check(ui, &checkbox, rect, settings.maps.is_enabled(map), check_drag, drag_from);
    if let Some(value) = check_drag.filter(|_| is_dragged_over) {
        settings.maps.set_enabled(map, value);
    } else if checkbox.clicked() && check_drag.is_none() {
        settings.maps.set_enabled(map, !settings.maps.is_enabled(map));
    }
    paint_check(ui.painter(), &tokens, check, Some(settings.maps.is_enabled(map)), checkbox.hovered());

    crate::property_sheet::paint_keyboard_focus(ui, &checkbox, &tokens);
    let texture = app.bake_map_thumbnails.texture(ui.ctx(), &app.session, app.session.ts(), map);
    let thumbnail_rect = Rect::from_min_size(pos2(check.right() + THUMBNAIL_AIR, rect.center().y - THUMBNAIL_SIZE / 2.0), vec2(THUMBNAIL_SIZE, THUMBNAIL_SIZE));
    thumbnail(ui, thumbnail_rect, texture.as_ref());

    let mut icons_left = rect.right();
    let focus_in_row = || {
        ui.memory(|memory| memory.focused())
            .and_then(|id| ui.ctx().read_response(id))
            .is_some_and(|widget| widget.layer_id == ui.layer_id() && rect.contains(widget.rect.center()))
    };
    if is_selected || hovered || focus_in_row() {
        let mut next_icon = || {
            let slot =
                Rect::from_center_size(pos2(icons_left - ROW_BUTTON_SIZE / 2.0 - ROW_INSET / 2.0, rect.center().y), vec2(ROW_BUTTON_SIZE, ROW_BUTTON_SIZE));
            icons_left = slot.left() - ROW_INSET / 2.0;
            slot
        };
        if several_sets {
            let shares = settings.sync.shares(BakeSyncGroup::Map(map));
            let look = if shares { GlyphLook::Lit } else { GlyphLook::Plain };
            let glyph = if shares { icon::LINK } else { icon::LINK_BREAK };
            let sync = glyph_button(ui, next_icon(), glyph, &trf("Synchronize {map} Settings", &[("map", &name)]), look);
            if sync.clicked() {
                SetPicker::open(app, &sync, SetPickerPurpose::Sync(BakeSyncGroup::Map(map)));
            }
        }
        let baking = app.bake_progress().is_some();
        let bolt = next_icon();
        let bolt_label = trf("Bake {map} Only", &[("map", &name)]);
        if ui.add_enabled_ui(!baking, |ui| glyph_button(ui, bolt, icon::LIGHTNING, &bolt_label, GlyphLook::Plain)).inner.clicked() {
            let active = app.session.ts();
            app.start_bake_of(vec![active], Some(map));
        }
        let shown = app.session.view == ViewMode::MeshMap(map);
        let look = if shown { GlyphLook::Pressed } else { GlyphLook::Plain };
        if glyph_button(ui, next_icon(), icon::EYE, &trf("Show {map} in the Viewport", &[("map", &name)]), look).clicked() {
            let mode = if shown { "material".to_owned() } else { format!("map:{}", map.id()) };
            app.run("viewport.view", json!({ "mode": mode }));
        }
    }

    let name_left = thumbnail_rect.right() + THUMBNAIL_AIR;
    let font = if is_selected { semibold(NAME_SIZE) } else { FontId::proportional(NAME_SIZE) };
    let color = if is_selected { tokens.text_strong } else { tokens.text };
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((icons_left - name_left).max(0.0));
    let galley = ui.painter().layout_job(job);

    // Expose the full label when its text is elided
    let response = if galley.elided { response.on_hover_text(name) } else { response };
    ui.painter().galley(pos2(name_left, rect.center().y - galley.size().y / 2.0), galley, color);

    crate::property_sheet::paint_keyboard_focus(ui, &response, &tokens);
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_has_popup(egui::accesskit::HasPopup::Menu));
    let mut popup = egui::Popup::context_menu(&response);
    let restore_focus = popup.is_open() && ui.input(|input| input.key_pressed(egui::Key::Escape));
    if response.has_focus()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F10) || input.consume_key(egui::Modifiers::NONE, egui::Key::Menu))
    {
        popup = popup.anchor(&response).open_memory(Some(egui::SetOpenCommand::Bool(true)));
    } else if !response.secondary_clicked() && popup.is_open() && egui::Popup::position_of_id(ui.ctx(), popup.get_id()).is_none() {
        popup = popup.anchor(&response);
    }

    popup.show(|ui| {
        let labels = |more: &str, all: &str| [trf(more, &[("map", &name)]), trf(all, &[("map", &name)])];
        let [check_more, check_all] = labels("Check {map} for More Texture Sets...", "Check {map} for All Texture Sets");
        copy_items(app, ui, &response, "check", Some(map), [&check_more, &check_all]);
        ui.separator();
        let [apply_more, apply_all] = labels("Apply {map} Settings to More Texture Sets...", "Apply {map} Settings to All Texture Sets");
        copy_items(app, ui, &response, "settings", Some(map), [&apply_more, &apply_all]);
        ui.separator();
        if ui.add_enabled(several_sets, egui::Button::new(tr("Sync Settings..."))).clicked() {
            SetPicker::open(app, &response, SetPickerPurpose::Sync(BakeSyncGroup::Map(map)));
            ui.close();
        }
    });
    if restore_focus {
        response.request_focus();
    }

    response.clicked()
}

/// Start a checkbox drag at a primary press on `checkbox`; returns whether the drag's pointer crossed this row's checkbox column (the whole row height counts) since `drag_from`
fn drag_check(ui: &Ui, checkbox: &egui::Response, row: Rect, is_enabled: bool, check_drag: &mut Option<bool>, drag_from: Option<Pos2>) -> bool {
    let is_primary = ui.input(|input| input.pointer.primary_down());
    if check_drag.is_none() && is_primary && checkbox.is_pointer_button_down_on() {
        *check_drag = Some(!is_enabled);
    }
    let Some(pointer) = ui.input(|input| input.pointer.latest_pos()).filter(|_| check_drag.is_some()) else {
        return false;
    };

    let column = Rect::from_x_y_ranges(checkbox.rect.x_range(), row.y_range());
    ui.rect_contains_pointer(column) || crosses(column, drag_from.unwrap_or(pointer), pointer)
}

/// Whether the straight line from `from` to `to` passes through `rect`, so a row skipped between two frames still counts
fn crosses(rect: Rect, from: Pos2, to: Pos2) -> bool {
    if !rect.intersects(Rect::from_two_pos(from, to)) {
        return false;
    }

    // Test the last separating axis, perpendicular to the drag
    let normal = (to - from).rot90();
    (rect.center() - from).dot(normal).abs() <= (rect.size() * 0.5).dot(normal.abs())
}

/// The baked map's thumbnail in `rect`; before it is baked, an empty square crossed by a diagonal
fn thumbnail(ui: &Ui, rect: Rect, texture: Option<&egui::TextureHandle>) {
    let tokens = Tokens::get(ui.ctx());
    let stroke = Stroke::new(1.0, tokens.text_dim);
    match texture {
        Some(texture) => {
            egui::Image::new(texture).paint_at(ui, rect);
        }
        None => {
            ui.painter().line_segment([rect.left_bottom(), rect.right_top()], stroke);
            ui.painter().rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        }
    }
}

/// A glyph in `rect` that answers clicks as `label`, its tooltip
fn glyph_button(ui: &mut Ui, rect: Rect, glyph: &str, label: &str, look: GlyphLook) -> egui::Response {
    let tokens = Tokens::get(ui.ctx());
    let response = ui.interact(rect, ui.scope_id().with(label), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), look != GlyphLook::Plain, label));
    let response = response.on_hover_text(label);
    if look == GlyphLook::Pressed {
        ui.painter().rect(rect, CornerRadius::same(ROW_RADIUS), tokens.separator, Stroke::new(1.0, tokens.button_border), egui::StrokeKind::Inside);
    }
    let color = if look == GlyphLook::Plain && !response.hovered() { tokens.text_dim } else { tokens.text_strong };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, FontId::proportional(ROW_GLYPH_SIZE), color);
    crate::property_sheet::paint_keyboard_focus(ui, &response, &tokens);
    response
}

/// One of the header's bordered buttons, sized to its glyph and words
fn header_button(ui: &mut Ui, glyph: &str, text: &str) -> egui::Response {
    let tokens = Tokens::get(ui.ctx());
    let icon = ui.painter().layout_no_wrap(glyph.to_owned(), FontId::proportional(ROW_GLYPH_SIZE), tokens.text);
    let words = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(NAME_SIZE), tokens.text);
    let width = 2.0 * HEADER_BUTTON_PADDING + icon.size().x + HEADER_GLYPH_GAP + words.size().x;
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEADER_BUTTON_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let fill = if response.hovered() { tokens.row_hover } else { tokens.button };
    ui.painter().rect(rect, CornerRadius::same(ROW_RADIUS), fill, Stroke::new(1.0, tokens.button_border), egui::StrokeKind::Inside);
    let x = rect.left() + HEADER_BUTTON_PADDING;
    ui.painter().galley(pos2(x, rect.center().y - icon.size().y / 2.0), icon.clone(), tokens.text);
    ui.painter().galley(pos2(x + icon.size().x + HEADER_GLYPH_GAP, rect.center().y - words.size().y / 2.0), words, tokens.text);
    crate::property_sheet::paint_keyboard_focus(ui, &response, &tokens);
    response
}
