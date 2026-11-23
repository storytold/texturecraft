use egui::{Align2, FontId, Id, Rect, Response, Sense, Stroke, pos2};
use egui_phosphor::regular as ic;

use super::{
    CONTROL_HEIGHT, ICON_SIZE, LABEL_MAX, LABEL_MIN_AIR, LOOSE_LABEL_AIR, PropertySheet, ROW_HEIGHT, ROW_PICTURE_AIR, ROW_PICTURE_SIZE, RowLead,
    SMALL_TEXT_SIZE, TEXT_SIZE,
};
use crate::theme::semibold;

/// Air above a stacked label
const STACK_AIR: f32 = 4.0;

/// Height of a stacked label's line, from its air to the control row under it
const STACK_LABEL_LINE: f32 = 14.0;

/// A stacked label's centre below its line's top
const STACK_LABEL_CENTER: f32 = 8.0;

/// A caption line's height
pub(super) const CAPTION_LINE: f32 = 18.0;

/// How far small text reaches below its centre line
const SMALL_TEXT_HALF_LINE: f32 = 6.0;

/// The square a row-end glyph answers the pointer in
const GLYPH_BOX: f32 = 20.0;

/// Air above a section caption that follows other rows, and the hairline's distance below it
const SECTION_AIR: f32 = 6.0;

/// The hairline above a later section caption, to the caption's top
pub(super) const SECTION_RULE_TO_CAPTION: f32 = 12.0;

/// Air above the panel's first section caption
const FIRST_SECTION_AIR: f32 = 12.0;

/// Height of a section caption's line, including the air under it
const SECTION_CAPTION_LINE: f32 = 24.0;

/// From a section caption's line top to its text's centre
pub(super) const SECTION_CAPTION_CENTER: f32 = 7.0;

/// Size of a section's menu glyph
const SECTION_MENU_ICON_SIZE: f32 = 14.0;

/// Distance from the right edge to the centre of a right-edge glyph (caret, menu)
pub(super) const EDGE_GLYPH_INSET: f32 = 6.0;

/// Size of the round reset glyph on a section caption or a band
pub(super) const RESET_ICON_SIZE: f32 = 14.0;

/// Opacity of a reset glyph at the defaults, where it does nothing
const RESET_AT_DEFAULT_OPACITY: f32 = 0.4;

/// From the right edge to the centre of a section caption's reset: its 14 px glyph ends at the dropdowns' right edge
const SECTION_RESET_INSET: f32 = RESET_ICON_SIZE / 2.0;

impl PropertySheet<'_> {
    /// A section caption in capitals, a hairline above a later one; `menu` adds the ≡ button
    pub(crate) fn section(&mut self, title: &str, menu: bool) -> Option<Response> {
        let text = self.tokens.text;
        self.section_in(title, menu, text)
    }

    /// [`Self::section`] with the round section reset at the caption's right end: faint and inert at the defaults (`changed` false); returns whether it was clicked
    pub(crate) fn section_reset(&mut self, title: &str, changed: bool) -> bool {
        let text = self.tokens.text;
        let center_y = self.section_caption(title, text);

        let glyph = Rect::from_center_size(pos2(self.right - SECTION_RESET_INSET, center_y), egui::Vec2::splat(GLYPH_BOX));
        let id = self.id.with(("section_reset", title));
        let sense = if changed { Sense::click() } else { Sense::hover() };
        let response = self.ui.interact(glyph, id, sense);

        let label = crate::i18n::tr("Reset to defaults");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, changed, format_args!("{title}: {label}")));

        if changed {
            // Left of the glyph, on the caption's line: it covers no value
            let mut tooltip = egui::Tooltip::for_enabled(&response);
            tooltip.popup = tooltip.popup.anchor(glyph).align(egui::RectAlign::LEFT).align_alternatives(&[egui::RectAlign::BOTTOM_END]);
            tooltip.show(|ui| ui.label(crate::i18n::tr("Restore defaults")));
        }

        let color = self.reset_glyph_color(changed, response.hovered(), self.tokens.text_strong);
        self.ui.painter().text(glyph.center(), Align2::CENTER_CENTER, ic::CLOCK_COUNTER_CLOCKWISE, FontId::proportional(RESET_ICON_SIZE), color);
        self.paint_control_focus(&response);
        changed && response.clicked()
    }

    /// A reset or clear glyph's colour: faint where it does nothing (`live` false), `hover_color` under the pointer, dim otherwise
    pub(super) fn reset_glyph_color(&self, live: bool, hovered: bool, hover_color: egui::Color32) -> egui::Color32 {
        match (live, hovered) {
            (false, _) => self.tokens.text_dim.gamma_multiply(RESET_AT_DEFAULT_OPACITY),
            (true, true) => hover_color,
            (true, false) => self.tokens.text_dim,
        }
    }

    /// [`Self::section`] with its caption in `color`
    pub(super) fn section_in(&mut self, title: &str, menu: bool, color: egui::Color32) -> Option<Response> {
        let center_y = self.section_caption(title, color);
        if !menu {
            return None;
        }

        let glyph = Rect::from_center_size(pos2(self.right - EDGE_GLYPH_INSET, center_y), egui::Vec2::splat(GLYPH_BOX));
        let id = self.id.with(("section_menu", title));
        let response = self.ui.interact(glyph, id, Sense::click());
        let color = if response.hovered() { self.tokens.text } else { self.tokens.text_dim };
        self.ui.painter().text(glyph.center(), Align2::CENTER_CENTER, ic::LIST, FontId::proportional(SECTION_MENU_ICON_SIZE), color);
        Some(response)
    }

    /// A section caption in capitals in `color`, a hairline above a later one; returns the caption's centre line
    fn section_caption(&mut self, title: &str, color: egui::Color32) -> f32 {
        self.close_card();

        // Named state before the section's first band is keyed by the section, not by the band above it
        if self.card.is_none() {
            let section = self.id.with(("section", title));
            self.band_scope = section;
            self.scope = section;
        }

        if !self.empty {
            self.space(SECTION_AIR);
            let y = self.cursor_y();
            self.ui.painter().line_segment([pos2(self.left, y), pos2(self.right, y)], Stroke::new(1.0, self.tokens.separator));
            self.space(SECTION_RULE_TO_CAPTION);
        } else {
            self.space(FIRST_SECTION_AIR);
        }

        let rect = self.take(SECTION_CAPTION_LINE);
        self.sections.push((title.to_owned(), rect.top()));
        if super::section_tabs::take_jump(self.ui, title) {
            // Scroll by distance because top alignment includes the scroll area's item spacing
            let hairline = rect.top() - SECTION_RULE_TO_CAPTION;
            let to_top = self.ui.clip_rect().top() - hairline;
            self.ui.scroll_with_delta(egui::vec2(0.0, to_top));
        }

        let center_y = rect.top() + SECTION_CAPTION_CENTER;
        self.ui.painter().text(pos2(self.left, center_y), Align2::LEFT_CENTER, title.to_uppercase(), semibold(SMALL_TEXT_SIZE), color);
        self.drawn(center_y + SMALL_TEXT_HALF_LINE);
        self.air_at = None;
        center_y
    }

    /// One setting's row: label beside the controls column or stacked above; returns the control's box and centre
    pub(crate) fn row(&mut self, label: &str) -> (Rect, f32) {
        self.row_with(RowLead::label(label))
    }

    /// [`Self::row`] led by `lead`
    pub(crate) fn row_with(&mut self, lead: RowLead<'_>) -> (Rect, f32) {
        let x0 = self.lead_with(lead);
        let rect = self.take(ROW_HEIGHT);
        let center_y = rect.center().y;
        let picture = matches!(lead, RowLead::Picture { .. });
        self.drawn(center_y + if picture { ROW_PICTURE_SIZE } else { CONTROL_HEIGHT } / 2.0);
        (self.control_box(x0, center_y), center_y)
    }

    /// Paints a row's lead and returns where its control starts; pictures and children never widen the column
    pub(crate) fn lead_with(&mut self, lead: RowLead<'_>) -> f32 {
        let font = FontId::proportional(TEXT_SIZE);
        let center_y = self.cursor_y() + ROW_HEIGHT / 2.0;
        let shift = self.lead_shift();
        let label_x = self.left + shift + ROW_PICTURE_SIZE + ROW_PICTURE_AIR;
        let (label, stack, text_color) = match lead {
            RowLead::Label { text, stack } => (text, stack, self.tokens.text),
            RowLead::Tinted { text, color } => (text, false, color),
            RowLead::Loose(text) if shift + self.text_width(text, font.clone()) <= self.column() - self.left - LOOSE_LABEL_AIR => {
                self.paint_info(center_y);
                self.ui.painter().text(pos2(self.left + shift, center_y), Align2::LEFT_CENTER, text, font, self.tokens.text);
                return self.column();
            }
            RowLead::Loose(text) => return self.stacked_label(text, shift),
            RowLead::Picture { picture, text } => {
                self.paint_info(center_y);
                let square = Rect::from_center_size(pos2(self.left + shift + ROW_PICTURE_SIZE / 2.0, center_y), egui::vec2(ROW_PICTURE_SIZE, ROW_PICTURE_SIZE));
                picture(self.ui.painter(), square);
                self.ui.painter().text(pos2(label_x, center_y), Align2::LEFT_CENTER, text, font, self.tokens.text);
                return self.column();
            }
            RowLead::Child(text) => {
                self.ui.painter().text(pos2(self.left, center_y), Align2::LEFT_CENTER, text, font, self.tokens.text_dim);
                return self.column();
            }
        };

        // The i's slot counts as part of the label
        let width = shift + self.text_width(label, font.clone());
        if width <= LABEL_MAX {
            self.seen.label_width = self.seen.label_width.max(width);
        }

        if width <= self.column() - self.left - LABEL_MIN_AIR && !stack {
            self.paint_info(center_y);
            self.ui.painter().text(pos2(self.left + shift, center_y), Align2::LEFT_CENTER, label, font, text_color);
            return self.column();
        }

        self.stacked_label(label, shift)
    }

    /// Paints `label` on its own line above a full-width control, `shift` right of the edge after its i; returns the control's left edge
    fn stacked_label(&mut self, label: &str, shift: f32) -> f32 {
        self.space(STACK_AIR);
        let rect = self.take(STACK_LABEL_LINE);
        let center_y = rect.top() + STACK_LABEL_CENTER;
        self.paint_info(center_y);
        let text = self.tokens.text;
        self.ui.painter().text(pos2(self.left + shift, center_y), Align2::LEFT_CENTER, label, FontId::proportional(TEXT_SIZE), text);
        self.left
    }

    /// A dim caption line above its control (Offset, Size…)
    pub(crate) fn caption(&mut self, text: &str) {
        let rect = self.take(CAPTION_LINE);
        let dim = self.tokens.text_dim;
        self.ui.painter().text(pos2(self.left, rect.top() + CAPTION_LINE / 2.0), Align2::LEFT_CENTER, text, FontId::proportional(SMALL_TEXT_SIZE), dim);
        self.drawn(rect.bottom());
    }

    /// A caret pointing down (open) or right (folded), centred on `center`
    pub(super) fn caret(&self, center: egui::Pos2, open: bool, color: egui::Color32) {
        let glyph = if open { ic::CARET_DOWN } else { ic::CARET_RIGHT };
        self.ui.painter().text(center, Align2::CENTER_CENTER, glyph, FontId::proportional(ICON_SIZE), color);
    }

    /// The next control's id: stable while the panel shows the same rows
    pub(super) fn next_id(&mut self) -> Id {
        let id = self.ui.next_auto_id();
        self.ui.skip_ahead_auto_ids(1);
        id
    }

    /// The id of a group or colour bar whose state outlives the frame: keyed by its name inside the open band and card, so rows shown or hidden above it never hand it another's state
    pub(super) fn named_id(&mut self, kind: &str, title: &str) -> Id {
        let mut id = self.scope.with((kind, title));

        // A repeated name in one scope (two cards of one effect) takes the next free id
        while self.named.contains(&id) {
            id = id.with(kind);
        }
        self.named.push(id);
        id
    }
}
