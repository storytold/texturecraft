//! The legacy source card for a mask fill with a placed projector.

use egui::{Align2, FontId, Rect, Stroke, Ui, pos2};
use egui_phosphor::regular as ic;

use crate::theme::Tokens;
use crate::widgets::elided;

pub(crate) const SLOT_HEIGHT: f32 = 52.0;
const SLOT_ROUNDING: f32 = 3.0;
const BORDER_WIDTH: f32 = 1.0;
/// Match the text column of preview cards by reserving their thumbnail width and insets
const TEXT_LEADING_SPACE: f32 = 52.0;
const CAPTION_FONT_SIZE: f32 = 11.0;
const NAME_FONT_SIZE: f32 = 13.0;
/// Gap between the caption's and the name's centers
const LINE_GAP: f32 = 15.0;
const CARET_INSET: f32 = 14.0;
const CARET_FONT_SIZE: f32 = 14.0;
/// Gap between the slot and its keyboard focus outline
const FOCUS_MARGIN: f32 = 2.0;

/// Draw the placed fill's source label and picker affordance
pub(crate) fn paint(ui: &Ui, rect: Rect, (highlighted, focused): (bool, bool), (caption, name): (&str, &str)) {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let (fill, border) = if highlighted { (tokens.row_hover, tokens.button_border) } else { (tokens.field, tokens.field_border) };
    painter.rect(rect, SLOT_ROUNDING, fill, Stroke::new(BORDER_WIDTH, border), egui::StrokeKind::Inside);
    if focused {
        painter.rect_stroke(rect.expand(FOCUS_MARGIN), SLOT_ROUNDING, ui.visuals().selection.stroke, egui::StrokeKind::Outside);
    }

    // Keep long names inside the text column
    let text_left = rect.left() + TEXT_LEADING_SPACE;
    let text_right = rect.right() - CARET_INSET * 2.0;
    let text_center_x = (text_left + text_right) * 0.5;
    let lines = [(caption, CAPTION_FONT_SIZE, tokens.text_dim, -LINE_GAP * 0.5), (name, NAME_FONT_SIZE, tokens.text_strong, LINE_GAP * 0.5)];
    for (text, size, color, offset) in lines {
        let galley = elided(painter, text, size, color, text_right - text_left);
        painter.galley(pos2(text_center_x, rect.center().y + offset) - galley.size() * 0.5, galley, color);
    }

    let caret_color = if highlighted { tokens.text } else { tokens.text_dim };
    painter.text(pos2(rect.right() - CARET_INSET, rect.center().y), Align2::CENTER_CENTER, ic::CARET_DOWN, FontId::proportional(CARET_FONT_SIZE), caret_color);
}
