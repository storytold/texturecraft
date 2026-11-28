use egui::{Align2, CornerRadius, FontId, Frame, Margin, Rect, RectAlign, RichText, Role, Sense, Shadow, Stroke, WidgetInfo, pos2, vec2};
use egui_phosphor::regular as ic;

use super::{PropertySheet, ROW_HEIGHT, SMALL_TEXT_SIZE};
use crate::theme::HARD_SHADOW;

/// The i's slot at the panel edge; a row with an i at its left starts this much further right
pub(super) const INFO_SLOT: f32 = 16.0;

/// The i's centre, from the panel's left padding
const INFO_CENTER: f32 = 6.0;

/// Size of the i glyph
const INFO_ICON_SIZE: f32 = 13.0;

/// Room the tooltip leaves at the panel's right edge, so it covers no number, dial or switch
const TIP_VALUE_ROOM: f32 = 70.0;

/// How far the tooltip reaches up into its row
const TIP_OVERLAP: f32 = 2.0;

/// The tooltip's text padding, line pitch, rounding and shadow
const TIP_MARGIN: Margin = Margin::symmetric(8, 5);
const TIP_LINE_HEIGHT: f32 = 15.0;
const TIP_ROUNDING: u8 = 5;
const TIP_SHADOW: Shadow = Shadow { offset: [2, 3], blur: 0, spread: 0, color: HARD_SHADOW };

impl<'u> PropertySheet<'u> {
    /// Gives the next row led by a label or picture an i at the panel edge that explains it on hover; its lead and its child rows move right to make room
    pub(crate) fn info(&mut self, tip: &'u str) -> &mut Self {
        self.info = Some(tip);
        self
    }

    /// How far the next row's lead moves right: past a pending i
    pub(super) fn lead_shift(&self) -> f32 {
        if self.info.is_some() { INFO_SLOT } else { 0.0 }
    }

    /// Paints the pending i at the panel edge on the row centred at `center_y`, and its tooltip while the pointer rests on it
    pub(super) fn paint_info(&mut self, center_y: f32) {
        self.paint_info_in(self.left, self.left + INFO_CENTER, center_y);
    }

    /// [`Self::paint_info`] at the row's right end, the i's right edge on the values' edge
    pub(super) fn paint_info_at_end(&mut self, center_y: f32) {
        self.paint_info_in(self.right - INFO_SLOT, self.right - INFO_ICON_SIZE / 2.0, center_y);
    }

    /// Paints the pending i at `glyph_x` in its slot from `slot_left`
    fn paint_info_in(&mut self, slot_left: f32, glyph_x: f32, center_y: f32) {
        let Some(tip) = self.info.take() else { return };

        let row_top = center_y - ROW_HEIGHT / 2.0;
        let slot = Rect::from_min_size(pos2(slot_left, row_top), vec2(INFO_SLOT, ROW_HEIGHT));
        let id = self.next_id();
        let response = self.ui.interact(slot, id, Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(Role::Image, true, tip));

        let tokens = self.tokens;
        let color = if response.hovered() { tokens.text } else { tokens.text_dim };
        self.ui.painter().text(pos2(glyph_x, center_y), Align2::CENTER_CENTER, ic::INFO, FontId::proportional(INFO_ICON_SIZE), color);

        // Under the row from the i, ending before the value column; above it where the window ends first
        let anchor = Rect::from_min_max(pos2(self.left, row_top), pos2(self.right - TIP_VALUE_ROOM, row_top + ROW_HEIGHT));
        let frame = Frame::NONE
            .fill(tokens.chrome)
            .stroke(Stroke::new(1.0, tokens.field_border))
            .corner_radius(CornerRadius::same(TIP_ROUNDING))
            .inner_margin(TIP_MARGIN)
            .shadow(TIP_SHADOW);
        let text_width = anchor.width() - TIP_MARGIN.sum().x;

        let mut tooltip = egui::Tooltip::for_enabled(&response);
        tooltip.popup = tooltip
            .popup
            .anchor(anchor)
            .align(RectAlign::BOTTOM_START)
            .align_alternatives(&[RectAlign::TOP_START])
            .gap(-TIP_OVERLAP)
            .frame(frame)
            .width(text_width);
        tooltip.show(|ui| {
            ui.set_width(text_width);
            ui.label(RichText::new(tip).size(SMALL_TEXT_SIZE).color(tokens.text).line_height(Some(TIP_LINE_HEIGHT)));
        });
    }
}
