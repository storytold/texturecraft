//! The Texture Set list: a toolbar, one row per set with its dropdowns, edits in place, a menu and a UV tile set's tiles

use egui::{Align2, CornerRadius, FontId, Popup, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::project::{MAX_RESOLUTION, MIN_RESOLUTION};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::layer_toolbar::{AddItem, ItemIcon, MENU_PADDING, MenuIcon, Pick, menu_button, menu_frame, menu_row, menu_width};
use crate::property_sheet::option_row;
use crate::shader_settings::INSTANCE_NAME;
use crate::theme::{Tokens, semibold};
use crate::widgets::elided;

mod description_mode;
mod list_state;
mod row_action;
mod row_edit;
mod set_row;
mod tile_action;

use description_mode::DescriptionMode;
pub(crate) use list_state::ListState;
use row_action::RowAction;
use row_edit::RowEdit;
use set_row::SetRow;
use tile_action::TileAction;

/// The layer toolbar's height and its menu buttons, so both panels' toolbars line up
const BAR_HEIGHT: f32 = 40.0;
const BAR_PADDING: f32 = 8.0;
const MENU_BUTTON: egui::Vec2 = vec2(32.0, 26.0);
const SOLO_SIZE: f32 = 24.0;
const SOLO_GAP: f32 = 6.0;

/// Row geometry: a set row, the description line under it and a UV tile row
const ROW_HEIGHT: f32 = 28.0;
const DESCRIPTION_HEIGHT: f32 = 14.0;
const TILE_ROW_HEIGHT: f32 = 24.0;
const ROW_GAP: f32 = 2.0;
const ROW_INSET: f32 = 6.0;
const ROW_ROUNDING: u8 = 4;
const GLYPH_SIZE: f32 = 20.0;
const GLYPH_FONT: f32 = 14.0;
const NAME_GAP: f32 = 8.0;
const FOLD_WIDTH: f32 = 14.0;
const TILE_INDENT: f32 = 34.0;
const TILE_BRANCH: f32 = 12.0;

const NAME_SIZE: f32 = 12.0;
const DESCRIPTION_SIZE: f32 = 11.0;
const CARET_SIZE: f32 = 11.0;

/// A row dropdown: its text, the caret after it and the air around both
const DROPDOWN_HEIGHT: f32 = 22.0;
const DROPDOWN_TEXT_X: f32 = 6.0;
const DROPDOWN_CARET_GAP: f32 = 4.0;
const DROPDOWN_RIGHT: f32 = 6.0;
const DROPDOWN_GAP: f32 = 4.0;
const SIZE_LIST_WIDTH: f32 = 72.0;

/// Narrow panels drop the shader dropdown, then the size, before cutting the name
const SHADER_MIN_ROW_WIDTH: f32 = 240.0;
const SIZE_MIN_ROW_WIDTH: f32 = 160.0;

/// From an edit field's left edge to its text: the field starts this far left of the drawn text
const EDIT_TEXT_X: f32 = 4.0;
/// The name field's air above and below inside the row
const EDIT_FIELD_INSET: f32 = 3.0;
/// The description line starts this far up into the set row and stops this far above the row's end
const DESCRIPTION_OVERLAP: f32 = 4.0;
const DESCRIPTION_BOTTOM: f32 = 1.0;
/// The description field's air above and below its text
const DESCRIPTION_FIELD_AIR: f32 = 1.0;
/// How far an empty description's placeholder fades from the dim text
const PLACEHOLDER_FADE: f32 = 0.7;
/// Keyboard focus ring: inset from the row's edge
const FOCUS_INSET: f32 = 2.0;
/// Space between a dropdown and its list
const MENU_GAP: f32 = 1.0;

pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui) {
    // The empty state lists nothing
    if !app.session.project_open {
        return;
    }

    let tokens = Tokens::get(ui.ctx());
    let mut state = ListState::load(ui.ctx(), (app.session.project_id, app.session.mesh_id));

    let rows: Vec<SetRow> = (0..app.session.doc.project.texture_sets.len())
        .filter_map(|index| {
            let set = app.session.doc.project.texture_sets.get(index)?;
            Some(SetRow {
                name: set.name.clone(),
                description: set.description.clone(),
                renamed: app.session.original_set_name(index).is_some_and(|original| original != set.name),
                width: set.resolution,
                height: set.height(),
                visible: set.visible,
                tiles: set.uv_tiles.iter().map(|tile| (*tile, set.tile_resolution(*tile))).collect(),
            })
        })
        .collect();

    // A picked UV tile only stays picked while its set is the active one
    let active = app.session.ts();
    if state.tile.is_some_and(|(set, tile)| set != active || !rows.get(set).is_some_and(|row| row.tiles.iter().any(|(udim, _)| *udim == tile))) {
        state.tile = None;
    }

    toolbar(app, ui, &mut state, &rows, active, &tokens);
    egui::ScrollArea::vertical().id_salt("ts_scroll").show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, ROW_GAP);
        for (index, row) in rows.iter().enumerate() {
            let action = set_row(app, ui, &mut state, index, row, index == active, &tokens);
            if !row.tiles.is_empty() && !state.folded.contains(&index) {
                for (udim, resolution) in &row.tiles {
                    let selected = state.tile == Some((index, *udim));
                    match tile_row(ui, index, *udim, *resolution, row.width, selected, &tokens) {
                        Some(TileAction::Select) => {
                            app.run("textureSet.select", json!({"ts": index}));
                            state.tile = Some((index, *udim));
                        }
                        Some(TileAction::Resize(size)) => {
                            app.run("textureSet.tile.resolution", json!({"ts": index, "tile": udim, "resolution": size}));
                        }
                        None => {}
                    }
                }
            }
            match action {
                Some(RowAction::Select) => {
                    app.run("textureSet.select", json!({"ts": index}));
                    state.tile = None;
                }
                Some(RowAction::ToggleVisible) => {
                    app.run("textureSet.visible", json!({"ts": index}));
                }
                Some(RowAction::Resize(width)) => {
                    app.start_resize(index, width);
                }
                Some(RowAction::Edit(edit)) => {
                    let text = if edit == RowEdit::Name { row.name.clone() } else { row.description.clone() };
                    state.edit = Some((index, edit, text, true));
                }
                Some(RowAction::ResetName) => {
                    app.run("textureSet.rename", json!({"ts": index, "reset": true}));
                }
                Some(RowAction::ToggleFold) => {
                    if state.folded.contains(&index) {
                        state.folded.remove(&index);
                    } else {
                        state.folded.insert(index);
                    }
                }
                None => {}
            }
        }
    });
    state.hovered = ui.ui_contains_pointer().then(|| ui.ctx().cumulative_pass_nr());
    state.store(ui.ctx());
}

/// Whether the pointer was over the list in the last pass, so F2 and the menu keys are the list's and not their bindings
pub(crate) fn pointer_over(ctx: &egui::Context) -> bool {
    // A list no longer drawn (another tab, no project) has none
    ListState::stored(ctx).and_then(|state| state.hovered).is_some_and(|pass| pass + 1 == ctx.cumulative_pass_nr())
}

/// Hands the selected row F2 (`rename`) or Shift+F10 or the Menu key (`menu`), read before the shortcuts ran
pub(crate) fn take_keys(ctx: &egui::Context, rename: bool, menu: bool) {
    if let Some(mut state) = ListState::stored(ctx).filter(|_| rename || menu) {
        state.rename_key |= rename;
        state.menu_key |= menu;
        state.store(ctx);
    }
}

/// Visibility menu and solo at the left, the descriptions menu at the right
fn toolbar(app: &mut TextureCraftApp, ui: &mut Ui, state: &mut ListState, rows: &[SetRow], active: usize, tokens: &Tokens) {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_HEIGHT), Sense::hover());
    let visibility = Rect::from_min_size(pos2(bar.left() + BAR_PADDING, bar.center().y - MENU_BUTTON.y * 0.5), MENU_BUTTON);
    let items = || {
        [(ic::EYE, "Show All", "showAll"), (ic::EYE_SLASH, "Hide All", "hideAll"), (ic::SWAP, "Invert Show/Hide", "invert")]
            .into_iter()
            .map(|(glyph, short, mode)| AddItem {
                icon: ItemIcon::Glyph(glyph),
                short,
                long: short,
                command: "textureSet.visibility",
                params: json!({ "mode": mode }),
                enabled: true,
            })
            .collect()
    };
    if let Some(Pick::Run { command, params }) =
        menu_button(ui, visibility, tr("Show or hide texture sets"), MenuIcon::Glyph(ic::EYE), tr("Visibility"), items, tokens)
    {
        app.run(command, params);
    }

    let solo_rect = Rect::from_min_size(pos2(visibility.right() + SOLO_GAP, bar.center().y - SOLO_SIZE * 0.5), vec2(SOLO_SIZE, SOLO_SIZE));
    let solo = rows.len() > 1 && rows.iter().enumerate().all(|(index, row)| row.visible == (index == active));
    if glyph_button(ui, "ts_solo", solo_rect, ic::NUMBER_CIRCLE_ONE, tr("Show only the selected texture set"), solo, tokens).clicked() {
        app.run("textureSet.visibility", json!({ "mode": "solo" }));
    }

    // The descriptions choice is the list's own look, so it stays in the UI
    let menu = Rect::from_min_size(pos2(bar.right() - BAR_PADDING - MENU_BUTTON.x, bar.center().y - MENU_BUTTON.y * 0.5), MENU_BUTTON);
    let modes = [
        (DescriptionMode::HideEmpty, "Hide Empty Descriptions"),
        (DescriptionMode::ShowAll, "Show All Descriptions"),
        (DescriptionMode::HideAll, "Hide All Descriptions"),
    ];
    let current = state.descriptions;
    let items = || {
        modes
            .iter()
            .enumerate()
            .map(|(index, (mode, short))| AddItem {
                icon: ItemIcon::Glyph(if *mode == current { ic::CHECK } else { "" }),
                short,
                long: short,
                command: "",
                params: json!(index),
                enabled: true,
            })
            .collect()
    };
    if let Some(Pick::Run { params, .. }) = menu_button(ui, menu, tr("Texture set list settings"), MenuIcon::Glyph(ic::LIST), tr("Descriptions"), items, tokens)
        && let Some((mode, _)) = params.as_u64().and_then(|index| modes.get(usize::try_from(index).ok()?))
    {
        state.descriptions = *mode;
    }
}

/// One texture set: eye, fold caret for UV tiles, name (or its field), size and shader dropdowns, description line
fn set_row(app: &mut TextureCraftApp, ui: &mut Ui, state: &mut ListState, index: usize, row: &SetRow, selected: bool, tokens: &Tokens) -> Option<RowAction> {
    let editing_description = matches!(state.edit, Some((set, RowEdit::Description, ..)) if set == index);
    let description_line = editing_description || state.descriptions.shows(&row.description);
    let height = if description_line { ROW_HEIGHT + DESCRIPTION_HEIGHT } else { ROW_HEIGHT };
    let (full, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, ui.is_enabled(), &row.name));
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_selected(selected));
    let rect = Rect::from_min_size(full.min, vec2(full.width(), ROW_HEIGHT));

    let painter = ui.painter();
    if selected {
        painter.rect(full, CornerRadius::same(ROW_ROUNDING), tokens.row_selected, Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
    } else if response.hovered() {
        painter.rect_filled(full, CornerRadius::same(ROW_ROUNDING), tokens.row_hover);
    }
    if response.has_focus() {
        painter.rect_stroke(full.shrink(FOCUS_INSET), CornerRadius::same(ROW_ROUNDING), Stroke::new(1.0, tokens.text_strong), StrokeKind::Inside);
    }

    let mut action = None;
    let eye = Rect::from_center_size(pos2(rect.left() + ROW_INSET + GLYPH_SIZE * 0.5, rect.center().y), vec2(GLYPH_SIZE, GLYPH_SIZE));
    let eye_label = if row.visible { tr("Hide texture set") } else { tr("Show texture set") };
    let eye_response = glyph_button(ui, ("ts_eye", index), eye, if row.visible { ic::EYE } else { ic::EYE_SLASH }, eye_label, false, tokens);
    if eye_response.clicked() {
        action = Some(RowAction::ToggleVisible);
    }
    if ui.is_enabled() && (response.contains_pointer() || eye_response.contains_pointer()) {
        app.next_hovered_texture_set = Some((app.session.project_id, index));
    }

    let mut name_x = eye.right() + NAME_GAP;
    if !row.tiles.is_empty() {
        let folded = state.folded.contains(&index);
        let fold = Rect::from_min_size(pos2(name_x - NAME_GAP * 0.5, rect.top()), vec2(FOLD_WIDTH, ROW_HEIGHT));
        let label = if folded { tr("Show UV tiles") } else { tr("Hide UV tiles") };
        if glyph_button(ui, ("ts_fold", index), fold, if folded { ic::CARET_RIGHT } else { ic::CARET_DOWN }, label, false, tokens).clicked() {
            action = Some(RowAction::ToggleFold);
        }
        name_x = fold.right() + NAME_GAP * 0.5;
    }

    // Dropdowns from the right edge: shader, then size
    let mut right = rect.right() - ROW_INSET;
    let name_color = if !row.visible {
        tokens.text_dim
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text
    };
    if rect.width() >= SHADER_MIN_ROW_WIDTH {
        let shader = tr(INSTANCE_NAME);
        let name = trf("Shader of {name}", &[("name", &row.name)]);
        let dropdown = row_dropdown(ui, ("ts_shader", index), pos2(right, rect.center().y), (shader, &name), true, tokens, |ui| {
            let _ = option_row(ui, shader, true);
        });
        right = dropdown.left() - DROPDOWN_GAP;
    }
    if rect.width() >= SIZE_MIN_ROW_WIDTH {
        let text = if row.width == row.height { row.width.to_string() } else { format!("{}x{}", row.width, row.height) };
        let mut picked = None;
        let name = trf("Size of {name}", &[("name", &row.name)]);
        let dropdown = row_dropdown(ui, ("ts_size", index), pos2(right, rect.center().y), (&text, &name), true, tokens, |ui| {
            picked = size_list(ui, row.width);
        });
        if let Some(size) = picked {
            action = Some(RowAction::Resize(size));
        }
        right = dropdown.left() - DROPDOWN_GAP;
    }

    let editing_name = matches!(state.edit, Some((set, RowEdit::Name, ..)) if set == index);
    if editing_name {
        let field = Rect::from_min_max(pos2(name_x - EDIT_TEXT_X, rect.top() + EDIT_FIELD_INSET), pos2(right, rect.bottom() - EDIT_FIELD_INSET));
        if let Some(name) = edit_field(ui, state, field, NAME_SIZE)
            && name.trim() != row.name
        {
            app.run("textureSet.rename", json!({"ts": index, "name": name}));
        }
    } else {
        let font = if selected { semibold(NAME_SIZE) } else { FontId::proportional(NAME_SIZE) };
        let mut job = egui::text::LayoutJob::simple_singleline(row.name.clone(), font, name_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - name_x).max(0.0));
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(name_x, rect.center().y - galley.size().y * 0.5), galley, name_color);
    }

    if description_line {
        let line = Rect::from_min_max(pos2(name_x, rect.bottom() - DESCRIPTION_OVERLAP), pos2(full.right() - ROW_INSET, full.bottom() - DESCRIPTION_BOTTOM));
        if editing_description {
            if let Some(description) = edit_field(ui, state, line.expand2(vec2(EDIT_TEXT_X, DESCRIPTION_FIELD_AIR)), DESCRIPTION_SIZE)
                && description.trim() != row.description
            {
                app.run("textureSet.settings", json!({"ts": index, "description": description.trim()}));
            }
        } else {
            let (text, color) = if row.description.trim().is_empty() {
                (tr("No description").to_owned(), tokens.text_dim.gamma_multiply(PLACEHOLDER_FADE))
            } else {
                (row.description.clone(), tokens.text_dim)
            };
            let galley = elided(ui.painter(), text, DESCRIPTION_SIZE, color, line.width());
            ui.painter().galley(pos2(line.left(), line.center().y - galley.size().y * 0.5), galley, color);
        }
    }

    // F2 renames the focused row, or the selected one with the pointer over the list, as a double click does
    let rename_key = (selected && std::mem::take(&mut state.rename_key))
        || (response.has_focus() && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F2)));
    if response.double_clicked() || rename_key {
        action = Some(RowAction::Edit(RowEdit::Name));
    } else if action.is_none() && (response.clicked() || response.secondary_clicked()) {
        action = Some(RowAction::Select);
    }
    let menu_key = selected && std::mem::take(&mut state.menu_key);
    context_menu(ui, &response, row, menu_key, tokens).or(action)
}

/// The row's right-click menu in the toolbar menus' look, also from Shift+F10 or the Menu key (`menu_key` over the list)
fn context_menu(ui: &Ui, response: &Response, row: &SetRow, menu_key: bool, tokens: &Tokens) -> Option<RowAction> {
    let visibility = if row.visible { (ic::EYE_SLASH, "Hide Texture Set") } else { (ic::EYE, "Show Texture Set") };
    let entries = [
        (visibility.0, visibility.1, true, RowAction::ToggleVisible),
        (ic::PENCIL_SIMPLE, "Edit Name", true, RowAction::Edit(RowEdit::Name)),
        (ic::ARROW_COUNTER_CLOCKWISE, "Reset Name to Original", row.renamed, RowAction::ResetName),
        (ic::NOTE_PENCIL, "Edit Description", true, RowAction::Edit(RowEdit::Description)),
    ];
    let items = entries.map(|(glyph, short, enabled, _)| AddItem {
        icon: ItemIcon::Glyph(glyph),
        short,
        long: short,
        command: "",
        params: serde_json::Value::Null,
        enabled,
    });
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_has_popup(egui::accesskit::HasPopup::Menu));
    let mut popup = Popup::context_menu(response).frame(menu_frame(tokens));
    let return_focus = popup.is_open() && ui.input(|input| input.key_pressed(egui::Key::Escape));

    // A menu opened from the keyboard has no pointer to open at: it hangs under the row while it stays open
    let from_keyboard = response.id.with("ts_menu_from_keyboard");
    let pressed =
        |input: &mut egui::InputState| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F10) || input.consume_key(egui::Modifiers::NONE, egui::Key::Menu);
    if menu_key || (response.has_focus() && ui.input_mut(pressed)) {
        ui.data_mut(|data| data.insert_temp(from_keyboard, true));
        popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
    } else if !popup.is_open() {
        ui.data_mut(|data| data.remove::<bool>(from_keyboard));
    }
    if ui.data(|data| data.get_temp::<bool>(from_keyboard)).unwrap_or(false) {
        popup = popup.anchor(response);
    }

    let mut picked = None;
    popup.show(|ui| {
        let width = menu_width(ui, &items);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(MENU_PADDING);
        for (item, (.., action)) in items.iter().zip(entries) {
            if menu_row(ui, width, item, tokens).clicked() {
                picked = Some(action);
                ui.close();
            }
        }
        ui.add_space(MENU_PADDING);
    });
    if return_focus {
        response.request_focus();
    }
    picked
}

/// A UV tile under its set: a branch from the set's fold caret, its UDIM and its own size dropdown
fn tile_row(ui: &mut Ui, set: usize, udim: u32, resolution: u32, set_width: u32, selected: bool, tokens: &Tokens) -> Option<TileAction> {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), TILE_ROW_HEIGHT), Sense::click());
    let label = trf("UV tile {udim}", &[("udim", &udim)]);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, ui.is_enabled(), &label));
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_selected(selected));
    let face = rect.with_min_x(rect.left() + TILE_INDENT - ROW_INSET);
    let painter = ui.painter();
    if selected {
        painter.rect(face, CornerRadius::same(ROW_ROUNDING), tokens.row_selected, Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
    } else if response.hovered() {
        painter.rect_filled(face, CornerRadius::same(ROW_ROUNDING), tokens.row_hover);
    }
    if response.has_focus() {
        painter.rect_stroke(face.shrink(FOCUS_INSET), CornerRadius::same(ROW_ROUNDING), Stroke::new(1.0, tokens.text_strong), StrokeKind::Inside);
    }
    let branch_x = rect.left() + ROW_INSET + GLYPH_SIZE + NAME_GAP * 0.5 + FOLD_WIDTH * 0.5;
    painter.vline(branch_x, rect.top() - ROW_GAP..=rect.center().y, Stroke::new(1.0, tokens.separator));
    painter.hline(branch_x..=branch_x + TILE_BRANCH, rect.center().y, Stroke::new(1.0, tokens.separator));
    let color = if selected { tokens.text_strong } else { tokens.text };
    let font = if selected { semibold(NAME_SIZE) } else { FontId::proportional(NAME_SIZE) };
    painter.text(pos2(rect.left() + TILE_INDENT, rect.center().y), Align2::LEFT_CENTER, &label, font, color);

    // An overridden tile shows its size in full text colour, one that follows the set dims it
    let mut picked = None;
    let text = resolution.to_string();
    let name = trf("Size of UV tile {udim}", &[("udim", &udim)]);
    let size_right = rect.right() - ROW_INSET - shader_column_width(ui, rect.width());
    let dim = resolution == set_width;
    row_dropdown(ui, ("ts_tile_size", set, udim), pos2(size_right, rect.center().y), (&text, &name), dim, tokens, |ui| {
        picked = size_list(ui, resolution);
    });
    if let Some(size) = picked {
        return Some(TileAction::Resize(size));
    }

    (response.clicked() || response.secondary_clicked()).then_some(TileAction::Select)
}

/// The shader dropdown's width on set rows, so tile sizes line up under set sizes
fn shader_column_width(ui: &Ui, row_width: f32) -> f32 {
    if row_width < SHADER_MIN_ROW_WIDTH {
        return 0.0;
    }

    dropdown_width(ui, tr(INSTANCE_NAME)) + DROPDOWN_GAP
}

fn dropdown_width(ui: &Ui, text: &str) -> f32 {
    let text_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), FontId::proportional(NAME_SIZE), egui::Color32::WHITE).size().x);
    DROPDOWN_TEXT_X + text_width + DROPDOWN_CARET_GAP + CARET_SIZE + DROPDOWN_RIGHT
}

/// A dropdown drawn as its text (`labels`: shown text, then what it sets) and a small caret, its right end middle at `anchor`: a button face only while hovered or open
fn row_dropdown(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    anchor: egui::Pos2,
    labels: (&str, &str),
    dim: bool,
    tokens: &Tokens,
    list: impl FnOnce(&mut Ui),
) -> Rect {
    let (text, name) = labels;
    let width = dropdown_width(ui, text);
    let rect = Rect::from_min_size(pos2(anchor.x - width, anchor.y - DROPDOWN_HEIGHT * 0.5), vec2(width, DROPDOWN_HEIGHT));
    let response = ui.interact(rect, ui.scope_id().with(id), Sense::click()).on_hover_text(name);
    let accessible = format!("{text}, {name}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), &accessible));
    let popup = Popup::menu(&response).frame(menu_frame(tokens)).width(SIZE_LIST_WIDTH.max(width)).gap(MENU_GAP).align(egui::RectAlign::BOTTOM_END);
    let open = popup.is_open();
    let painter = ui.painter();
    if open || response.hovered() {
        painter.rect(rect, CornerRadius::same(ROW_ROUNDING), tokens.button, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
    }
    let color = if dim && !response.hovered() && !open { tokens.text_dim } else { tokens.text };
    painter.text(pos2(rect.left() + DROPDOWN_TEXT_X, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(NAME_SIZE), color);
    painter.text(
        pos2(rect.right() - DROPDOWN_RIGHT - CARET_SIZE * 0.5, rect.center().y),
        Align2::CENTER_CENTER,
        ic::CARET_DOWN,
        FontId::proportional(CARET_SIZE),
        tokens.text_dim,
    );
    popup.show(|ui| {
        ui.set_min_width(SIZE_LIST_WIDTH.max(width));
        ui.add_space(MENU_PADDING);
        list(ui);
        ui.add_space(MENU_PADDING);
    });
    rect
}

/// The sizes a set or tile can take, the current one ticked; the one picked
fn size_list(ui: &mut Ui, current: u32) -> Option<u32> {
    let mut picked = None;
    let mut size = MIN_RESOLUTION;
    while size <= MAX_RESOLUTION {
        if option_row(ui, &size.to_string(), size == current).clicked() && size != current {
            picked = Some(size);
        }
        size *= 2;
    }
    picked
}

/// The field of the row's text being edited: Enter (or clicking away) gives the text, Esc drops it
fn edit_field(ui: &mut Ui, state: &mut ListState, rect: Rect, size: f32) -> Option<String> {
    let (_, _, text, needs_focus) = state.edit.as_mut()?;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));

    // The text sits where the drawn name or description sat, centred in the field
    let font = FontId::proportional(size);
    let text_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let margin = vec2(EDIT_TEXT_X, ((rect.height() - text_height) * 0.5).max(0.0));
    let response = child.add_sized(rect.size(), egui::TextEdit::singleline(text).font(font).margin(margin));
    if std::mem::take(needs_focus) {
        response.request_focus();
    }
    if !response.lost_focus() {
        return None;
    }

    let edit = state.edit.take()?;

    // Raw Escape: with the field's focus gone, the window's cancel takes it from egui's input
    let escaped = ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    (!escaped).then_some(edit.2)
}

/// A glyph button of the list: dim glyph, bright and framed when hovered or lit
fn glyph_button(ui: &mut Ui, id: impl egui::AsIdSalt, rect: Rect, glyph: &str, label: &str, lit: bool, tokens: &Tokens) -> Response {
    let response = ui.interact(rect, ui.scope_id().with(id), Sense::click()).on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), lit, label));
    if lit || response.hovered() {
        ui.painter().rect(rect, CornerRadius::same(ROW_ROUNDING), tokens.button, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
    }
    let color = if lit || response.hovered() { tokens.text_strong } else { tokens.text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(GLYPH_FONT), color);
    response
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::*;

    fn harness(mesh: &str) -> Harness<'static, TextureCraftApp> {
        let mesh_ref = format!("builtin:{mesh}");
        let geometry = texturecraft_engine::geom::primitives::builtin(mesh).unwrap();
        let session = texturecraft_engine::Session::new_project("Texture set list", geometry, &mesh_ref, None, 128).unwrap();
        let mut harness = Harness::builder()
            // Tall enough that every row clears the panel under the list
            .with_size(vec2(1600.0, 2400.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        harness.run_steps(4);
        harness
    }

    /// Presses and lets go on `label`'s node through the real pointer path, as a person's click lands on a control drawn over its row
    fn click(harness: &mut Harness<'static, TextureCraftApp>, role: egui::Role, label: &str, button: egui::PointerButton) {
        let rect = harness.get_by_role_and_label(role, label).rect();

        // A row's middle is under its size dropdown in a narrow panel: aim at its name
        let at = if role == egui::Role::ListItem { egui::pos2(rect.left() + rect.width() * 0.25, rect.center().y) } else { rect.center() };
        let modifiers = egui::Modifiers::NONE;
        harness.event(egui::Event::PointerMoved(at));
        harness.event(egui::Event::PointerButton { pos: at, button, pressed: true, modifiers });
        harness.run_steps(1);
        harness.event(egui::Event::PointerButton { pos: at, button, pressed: false, modifiers });
        harness.run_steps(2);
    }

    #[test]
    fn the_row_size_dropdown_resizes_its_set() {
        let mut harness = harness("material_ball");
        let name = harness.state().session.doc.project.texture_sets[1].name.clone();

        click(&mut harness, egui::Role::ComboBox, &format!("128, Size of {name}"), egui::PointerButton::Primary);
        click(&mut harness, egui::Role::ListBoxOption, "512", egui::PointerButton::Primary);

        // Wait for the worker and let the next frame apply its actual result
        if let Some(resize) = harness.state_mut().resize.as_mut() {
            let result = resize.rx.recv_timeout(std::time::Duration::from_secs(30)).expect("resize worker did not send a result");
            let (sender, receiver) = std::sync::mpsc::channel();
            assert!(sender.send(result).is_ok(), "resize completion could not be forwarded");
            resize.rx = receiver;
        }

        harness.run_steps(2);

        assert!(harness.state().resize.is_none(), "the app consumed the resize result");
        let sets = &harness.state().session.doc.project.texture_sets;
        assert!(sets[1].resolution > 128 && sets[1].height() == sets[1].resolution, "the set grew and stayed square");
        assert_eq!(sets[0].resolution, 128, "the other set kept its size");
    }

    #[test]
    fn a_width_change_cannot_distort_a_set_at_the_height_limit() {
        let mut harness = harness("material_ball");
        let app = harness.state_mut();

        for (width, height, picked_width) in [(256, 128, MIN_RESOLUTION), (128, 256, MAX_RESOLUTION)] {
            texturecraft_engine::execute(&mut app.session, "textureSet.resolution", &json!({"resolution": width, "height": height})).unwrap();

            app.start_resize(0, picked_width);

            assert!(app.resize.is_none(), "an unrepresentable height must not start a resize worker");
            assert_eq!(app.session.doc.project.texture_sets[0].size(), (width, height));
        }
    }

    #[test]
    fn the_right_click_menu_renames_in_place_and_resets_to_the_material_name() {
        let mut harness = harness("material_ball");
        let original = harness.state().session.doc.project.texture_sets[1].name.clone();

        click(&mut harness, egui::Role::ListItem, &original, egui::PointerButton::Secondary);
        click(&mut harness, egui::Role::Button, "Edit Name", egui::PointerButton::Primary);
        harness.run_steps(3);

        // The field took focus, so typed text goes to it
        harness.event(egui::Event::Text("x".into()));
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        let renamed = harness.state().session.doc.project.texture_sets[1].name.clone();
        assert!(renamed != original && renamed.contains('x'), "{renamed}");

        click(&mut harness, egui::Role::ListItem, &renamed, egui::PointerButton::Secondary);
        click(&mut harness, egui::Role::Button, "Reset Name to Original", egui::PointerButton::Primary);
        harness.run_steps(3);
        assert_eq!(harness.state().session.doc.project.texture_sets[1].name, original);
    }

    #[test]
    fn a_focused_row_renames_with_f2_and_opens_its_menu_from_the_keyboard() {
        let mut harness = harness("material_ball");
        let original = harness.state().session.doc.project.texture_sets[1].name.clone();

        harness.get_by_role_and_label(egui::Role::ListItem, &original).focus();
        harness.run_steps(2);
        harness.key_press(egui::Key::F2);
        harness.run_steps(3);
        harness.event(egui::Event::Text("y".into()));
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        let renamed = harness.state().session.doc.project.texture_sets[1].name.clone();
        assert!(renamed != original && renamed.contains('y'), "{renamed}");

        harness.get_by_role_and_label(egui::Role::ListItem, &renamed).focus();
        harness.run_steps(2);
        assert!(harness.query_by_role_and_label(egui::Role::Button, "Edit Description").is_none());
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F10);
        harness.run_steps(3);
        assert!(harness.query_by_role_and_label(egui::Role::Button, "Edit Description").is_some(), "Shift+F10 opens the row's menu");
    }

    #[test]
    fn escape_drops_a_rename_under_way() {
        let mut harness = harness("material_ball");
        let original = harness.state().session.doc.project.texture_sets[1].name.clone();
        click(&mut harness, egui::Role::ListItem, &original, egui::PointerButton::Secondary);
        click(&mut harness, egui::Role::Button, "Edit Name", egui::PointerButton::Primary);
        harness.run_steps(3);
        harness.event(egui::Event::Text("q".into()));
        harness.run_steps(1);

        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
        assert_eq!(harness.state().session.doc.project.texture_sets[1].name, original);
    }

    #[test]
    fn over_the_list_f2_renames_the_selected_set_and_shift_f10_opens_its_menu() {
        // A clicked row has no keyboard focus, so these keys would otherwise reach the 3D view and Render mode
        let mut harness = harness("material_ball");
        let original = harness.state().session.doc.project.texture_sets[1].name.clone();
        let layout = harness.state().ui.layout;
        click(&mut harness, egui::Role::ListItem, &original, egui::PointerButton::Primary);
        harness.key_press(egui::Key::F2);
        harness.run_steps(3);
        harness.event(egui::Event::Text("w".into()));
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        let renamed = harness.state().session.doc.project.texture_sets[1].name.clone();
        assert!(renamed != original && renamed.contains('w'), "{renamed}");
        assert!(harness.state().ui.layout == layout, "the viewers layout stayed");

        let mode = harness.state().ui.mode;
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F10);
        harness.run_steps(3);
        assert!(harness.query_by_role_and_label(egui::Role::Button, "Edit Description").is_some(), "Shift+F10 opens the row's menu");
        assert!(harness.state().ui.mode == mode, "the mode stayed");
    }

    #[test]
    fn a_rename_under_way_is_dropped_when_another_project_replaces_this_one() {
        let mut harness = harness("material_ball");
        let original = harness.state().session.doc.project.texture_sets[1].name.clone();
        harness.get_by_role_and_label(egui::Role::ListItem, &original).focus();
        harness.run_steps(2);
        harness.key_press(egui::Key::F2);
        harness.run_steps(3);
        harness.event(egui::Event::Text("z".into()));
        harness.run_steps(1);

        // Another project with the same texture sets takes this one's place mid-edit
        let geometry = texturecraft_engine::geom::primitives::builtin("material_ball").unwrap();
        harness.state_mut().session = texturecraft_engine::Session::new_project("Other", geometry, "builtin:material_ball", None, 128).unwrap();
        harness.run_steps(3);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);

        assert_eq!(harness.state().session.doc.project.texture_sets[1].name, original, "the new project's set keeps its name");
    }

    #[test]
    fn a_uv_tile_row_sizes_its_own_tile() {
        let mut harness = harness("udim_plane");
        let tiles = harness.state().session.doc.project.texture_sets[0].uv_tiles.clone();
        assert!(harness.query_by_role_and_label(egui::Role::ListItem, &format!("UV tile {}", tiles[1])).is_some());

        click(&mut harness, egui::Role::ComboBox, &format!("128, Size of UV tile {}", tiles[1]), egui::PointerButton::Primary);
        click(&mut harness, egui::Role::ListBoxOption, "256", egui::PointerButton::Primary);
        harness.run_steps(3);

        let set = &harness.state().session.doc.project.texture_sets[0];
        assert!(set.tile_resolution(tiles[1]) > set.tile_resolution(tiles[0]), "only the picked tile grew");
    }
}
