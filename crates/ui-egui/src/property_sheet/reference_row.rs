//! One row of a list of things to go to: a glyph, a name and its kind (dim); hovered, a face and an arrow show the click selects it

use egui::{Align2, CornerRadius, FontId, Sense, pos2};
use egui_phosphor::regular as ic;

use super::{GLYPH_SIZE, PropertySheet, ROW_HEIGHT, ROW_RADIUS, TEXT_SIZE};

/// From the left edge to the glyph's centre, and from there to the name
const GLYPH_CENTER: f32 = 6.0;
const GLYPH_TO_NAME: f32 = 14.0;

/// Size of the arrow
const ARROW_SIZE: f32 = 12.0;

/// Air between the name and its kind
const NAME_TO_KIND: f32 = 6.0;

/// From the right edge to the arrow's centre
const ARROW_INSET: f32 = 6.0;

impl PropertySheet<'_> {
    /// The row of `name` (a `kind`, drawn with `glyph`); returns true when it was clicked
    pub(crate) fn reference_row(&mut self, glyph: &str, name: &str, kind: &str) -> bool {
        let rect = self.take(ROW_HEIGHT);
        let center_y = rect.center().y;
        let face = self.bled_row(rect);
        let id = self.next_id();
        let tokens = self.tokens;
        let painter = self.ui.painter().clone();

        // The kind and the arrow keep their room; a name too long for the rest is cut short and shows in full on hover
        let kind_galley = painter.layout_no_wrap(kind.to_owned(), FontId::proportional(TEXT_SIZE), tokens.text_dim);
        let name_left = self.left + GLYPH_CENTER + GLYPH_TO_NAME;
        let name_room = self.right - ARROW_INSET - ARROW_SIZE * 0.5 - kind_galley.size().x - NAME_TO_KIND - name_left;
        let name_galley = crate::widgets::elided(&painter, name, TEXT_SIZE, tokens.text, name_room);
        let cut_short = name_galley.elided;

        let response = self.ui.interact(face, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = if cut_short { response.on_hover_text(name) } else { response };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{name} {kind}")));

        if response.hovered() {
            painter.rect_filled(face, CornerRadius::same(ROW_RADIUS), tokens.button);
            painter.text(pos2(self.right - ARROW_INSET, center_y), Align2::CENTER_CENTER, ic::CARET_RIGHT, FontId::proportional(ARROW_SIZE), tokens.text);
        }
        painter.text(pos2(self.left + GLYPH_CENTER, center_y), Align2::CENTER_CENTER, glyph, FontId::proportional(GLYPH_SIZE), tokens.text_dim);
        let name_right = name_left + name_galley.size().x;
        painter.galley(pos2(name_left, center_y - name_galley.size().y * 0.5), name_galley, tokens.text);
        painter.galley(pos2(name_right + NAME_TO_KIND, center_y - kind_galley.size().y * 0.5), kind_galley, tokens.text_dim);
        self.drawn(rect.bottom());
        response.clicked()
    }
}
