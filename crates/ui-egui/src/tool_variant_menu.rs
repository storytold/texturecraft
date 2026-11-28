//! The tool shelf's variant menus: the button with its corner arrow, the gestures that open it and the menu of variants

use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Rect, Response, Sense, Ui, vec2};
use serde_json::json;
use texturecraft_engine::Tool;
use texturecraft_engine::tool_variant::ToolVariant;

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use crate::tool_variant_icon;
use crate::widgets::painted_icon_button;

/// The corner arrow's size, and its centre's inset from the button's lower-right corner, as fractions of the button height
const ARROW_SCALE: f32 = 0.32;
const ARROW_INSET_SCALE: f32 = 0.16;

/// The button's lower-right square a press must start in to open the menu rather than select the tool, as a fraction of its height
const ARROW_CORNER_SCALE: f32 = 0.36;

/// The menu's distance from the button: past the shelf card's padding
const MENU_GAP: f32 = 6.0;

/// The menu's rows: their height (the icon's square), the inset around them, the space between them and their rounding
const ROW_HEIGHT: f32 = 24.0;
const ROW_INSET: i8 = 3;
const ROW_SPACING: f32 = 2.0;
const ROW_RADIUS: u8 = 4;

/// The gap between a row's icon and its label, and the space after the longest label
const ROW_LABEL_GAP: f32 = 4.0;
const ROW_END_PADDING: f32 = 12.0;

/// The accent bar at the current variant's left edge
const CURRENT_BAR_WIDTH: f32 = 3.0;

/// How long a press on the button must be held to open the menu instead of selecting the tool, in seconds
const HOLD_SECONDS: f64 = 0.4;

/// The keys a focused row takes from egui's focus moves: the menu moves between its rows, and Tab and Escape leave it
const ROW_KEYS: egui::EventFilter = egui::EventFilter { tab: true, horizontal_arrows: false, vertical_arrows: true, escape: true };

/// Temp-data keys under the menu's id: focus the current row on the next frame, and the last pass a row had the focus
const FOCUS_CURRENT_ROW: &str = "focus_current_row";
const FOCUS_IN_MENU: &str = "focus_in_menu";

/// A tool's shelf button, without a tooltip: its current variant's icon and the arrow; screen readers get the tool's `name` with the variant as its description
pub(crate) fn button(ui: &mut Ui, variant: ToolVariant, name: &str, selected: bool, size: f32) -> Response {
    let tokens = Tokens::get(ui.ctx());
    let variant_name = tr(variant.label());
    let response = painted_icon_button(ui, name, selected, size, |painter, rect, color, fill| {
        // Clear at rest over the shelf card, a greyed fill faded over it; the arrow stays quieter than the icon until the button lights up
        let (ground, arrow_color) =
            if fill == Color32::TRANSPARENT { (tokens.panel, tokens.text_dim) } else { (tokens.panel.lerp_to_gamma(fill, painter.opacity()), color) };
        tool_variant_icon::paint(painter, rect, variant, color, ground);
        let arrow_centre = rect.right_bottom() - vec2(size, size) * ARROW_INSET_SCALE;
        painter.text(arrow_centre, Align2::CENTER_CENTER, egui_phosphor::regular::CARET_DOWN, FontId::proportional(size * ARROW_SCALE), arrow_color);
    });
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(variant_name));
    response
}

/// Opens, shows and closes `tool`'s variant menu beside its shelf `button`; true when this frame's click on the button was the menu's, not a selection
pub(crate) fn menu(app: &mut TextureCraftApp, button: &Response, tool: Tool) -> bool {
    let ctx = button.ctx.clone();
    let popup_id = button.id.with("variants");

    // A greyed tool offers no variants, and an open menu goes when its tool stops being usable
    if !button.enabled() {
        if egui::Popup::is_id_open(&ctx, popup_id) {
            egui::Popup::close_id(&ctx, popup_id);
        }
        return false;
    }

    let (arrow_click, held_open, release_of_hold) = pointer_gestures(button);
    let from_keyboard = button.has_focus()
        && ctx.input_mut(|input| {
            input.consume_key(Modifiers::ALT, Key::ArrowDown) || input.consume_key(Modifiers::SHIFT, Key::F10) || input.consume_key(Modifiers::NONE, Key::Menu)
        });
    if from_keyboard {
        ctx.data_mut(|data| data.insert_temp(popup_id.with(FOCUS_CURRENT_ROW), true));
    }

    // A second click on the arrow closes the menu: it started outside the open menu
    if arrow_click || held_open || from_keyboard || button.secondary_clicked() {
        egui::Popup::open_id(&ctx, popup_id);
    }
    let frame = egui::Frame::menu(&ctx.global_style()).inner_margin(ROW_INSET);
    let name = trf("{tool} variants", &[("tool", &tr(tool.label()))]);
    egui::Popup::from_response(button)
        .id(popup_id)
        .kind(egui::PopupKind::Menu)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .align(egui::RectAlign::RIGHT_START)
        .gap(MENU_GAP)
        .frame(frame)
        .accessible_name(name)
        .show(|ui| rows(app, ui, tool, popup_id));

    // The button says it has a menu and whether it is open; a menu the keyboard was in gives the focus back when it closes
    let open = egui::Popup::is_id_open(&ctx, popup_id);
    ctx.accesskit_node_builder(button.id, |node| {
        node.set_has_popup(egui::accesskit::HasPopup::Menu);
        node.set_expanded(open);
    });
    if !open && let Some(focus_pass) = ctx.data_mut(|data| data.remove_temp::<u64>(popup_id.with(FOCUS_IN_MENU))) {
        // Only a menu that just closed under the keyboard: not one left long ago, and not one a click went elsewhere from
        if focus_pass + 1 >= ctx.cumulative_pass_nr() && !ctx.input(|input| input.pointer.any_click()) {
            button.request_focus();
        }
    }
    arrow_click || release_of_hold
}

/// This frame's arrow click, a press held long enough to open the menu, and that held press's release; a press's start is kept, as its release forgets it
fn pointer_gestures(button: &Response) -> (bool, bool, bool) {
    let ctx = &button.ctx;
    let (now, pressed, down, position) =
        ctx.input(|input| (input.time, input.pointer.primary_pressed(), input.pointer.primary_down(), input.pointer.interact_pos()));
    let [on_arrow_id, held_since_id, held_open_id] = ["pressed_on_arrow", "held_since", "held_open"].map(|key| button.id.with(key));
    if button.hovered() && pressed {
        let on_arrow = position.is_some_and(|position| arrow_corner(button.rect).contains(position));
        ctx.data_mut(|data| {
            data.insert_temp(on_arrow_id, on_arrow);
            data.insert_temp(held_since_id, now);
        });
    }

    // The app draws only on input, so a still pointer asks for the frame the hold ends in
    let mut held_open = false;
    if let Some(held_since_seconds) = ctx.data(|data| data.get_temp::<f64>(held_since_id)) {
        let remaining_seconds = held_since_seconds + HOLD_SECONDS - now;
        if !down || !position.is_some_and(|position| button.rect.contains(position)) {
            ctx.data_mut(|data| data.remove::<f64>(held_since_id));
        } else if remaining_seconds <= 0.0 {
            held_open = true;
            ctx.data_mut(|data| {
                data.remove::<f64>(held_since_id);
                data.insert_temp(held_open_id, true);
            });
        } else {
            ctx.request_repaint_after_secs(remaining_seconds as f32);
        }
    }

    let flag = |id| ctx.data(|data| data.get_temp::<bool>(id)).unwrap_or_default();
    let arrow_click = button.clicked() && flag(on_arrow_id);
    let release_of_hold = button.clicked() && flag(held_open_id);
    if !down {
        ctx.data_mut(|data| {
            data.remove::<bool>(on_arrow_id);
            data.remove::<bool>(held_open_id);
        });
    }
    (arrow_click, held_open, release_of_hold)
}

/// The square at the button's lower right that holds the arrow
fn arrow_corner(button: Rect) -> Rect {
    let side = button.height() * ARROW_CORNER_SCALE;
    Rect::from_min_max(button.right_bottom() - vec2(side, side), button.right_bottom())
}

/// One row per variant of `tool`, the current one lit; the menu moves the focus between its rows itself, and Tab leaves it for the button
fn rows(app: &mut TextureCraftApp, ui: &mut Ui, tool: Tool, popup_id: Id) {
    let tokens = Tokens::get(ui.ctx());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let current = ToolVariant::current(&app.session.tool, tool);
    let variants = ToolVariant::of(tool);
    let label_width =
        variants.iter().map(|variant| ui.painter().layout_no_wrap(tr(variant.label()).to_owned(), font.clone(), tokens.text).size().x).fold(0.0, f32::max);
    let width = ROW_HEIGHT + ROW_LABEL_GAP + label_width + ROW_END_PADDING;
    ui.spacing_mut().item_spacing = vec2(0.0, ROW_SPACING);

    // A menu opened from the keyboard starts on the current variant
    let focus_current = ui.data_mut(|data| data.remove_temp::<bool>(popup_id.with(FOCUS_CURRENT_ROW))).unwrap_or_default();
    let mut rows = Vec::with_capacity(variants.len());
    for &variant in variants {
        let label = tr(variant.label());
        let is_current = current == Some(variant);
        let (rect, mut response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), is_current, label));
        if focus_current && is_current {
            response.request_focus();
        }

        // The current variant is lit like the active tool and marked with the accent bar, so colour is not its only sign
        let lit = response.hovered() || response.has_focus();
        let fill = if is_current {
            tokens.tool_active
        } else if lit {
            tokens.row_hover
        } else {
            tokens.panel
        };
        let text_color = if is_current || lit { tokens.text_strong } else { tokens.text };
        let painter = ui.painter();
        painter.rect_filled(rect, ROW_RADIUS, fill);
        if is_current {
            let bar = Rect::from_min_size(rect.min, vec2(CURRENT_BAR_WIDTH, ROW_HEIGHT));
            painter.rect_filled(bar, CornerRadius { nw: ROW_RADIUS, sw: ROW_RADIUS, ne: 0, se: 0 }, tokens.accent);
        }
        let icon_rect = Rect::from_min_size(rect.min, vec2(ROW_HEIGHT, ROW_HEIGHT));
        tool_variant_icon::paint(painter, icon_rect, variant, text_color, fill);
        painter.text(icon_rect.right_center() + vec2(ROW_LABEL_GAP, 0.0), Align2::LEFT_CENTER, label, font.clone(), text_color);

        // Particles come only from strokes in the 3D view
        if variant.physical() {
            response = response.on_hover_text(tr("Particles come from strokes in the 3D view only"));
        }
        if response.clicked() {
            app.run(variant.command_id(), json!({}));
            egui::Popup::close_id(ui.ctx(), popup_id);
        }
        rows.push(response);
    }

    let Some(focused) = rows.iter().position(Response::has_focus) else { return };

    // egui's own arrow moves could leave the menu for any widget near it, and its Tab would skip past the button
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|data| data.insert_temp(popup_id.with(FOCUS_IN_MENU), pass));
    ui.memory_mut(|memory| memory.set_focus_lock_filter(rows[focused].id, ROW_KEYS));
    let (down, up, tab) = ui.input(|input| (input.key_pressed(Key::ArrowDown), input.key_pressed(Key::ArrowUp), input.key_pressed(Key::Tab)));
    if tab {
        egui::Popup::close_id(ui.ctx(), popup_id);
    } else if down {
        rows[(focused + 1).min(rows.len() - 1)].request_focus();
    } else if up {
        rows[focused.saturating_sub(1)].request_focus();
    }
}
