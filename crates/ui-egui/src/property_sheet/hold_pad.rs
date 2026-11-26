//! A picture that is also a press-and-hold surface: a badge in its corner says so, and the edge and badge light while hovered or held

use egui::{Align2, CornerRadius, FontId, Rect, Response, Role, Sense, Stroke, StrokeKind, WidgetInfo, vec2};

use super::PropertySheet;

/// Side of the badge in the picture's bottom-right corner, and its inset from the picture's edges
const BADGE_SIZE: f32 = 16.0;
const BADGE_INSET: f32 = 4.0;
const BADGE_ROUNDING: u8 = 3;

/// Size of the badge's glyph
const BADGE_GLYPH_SIZE: f32 = 11.0;

impl PropertySheet<'_> {
    /// Makes the already painted `picture` a press-and-hold surface named `name`, marked by `glyph` in a badge; `held` lights it as hover does. It follows a drag that starts on it, wherever the pointer goes
    pub(crate) fn hold_pad(&mut self, picture: Rect, glyph: &str, name: &str, held: bool) -> Response {
        let id = self.next_id();
        let response = self.ui.interact(picture, id, Sense::click_and_drag());
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));

        let tokens = self.tokens;
        let lit = held || response.hovered();
        let painter = self.ui.painter();
        let badge = Rect::from_min_size(picture.right_bottom() - vec2(BADGE_INSET + BADGE_SIZE, BADGE_INSET + BADGE_SIZE), vec2(BADGE_SIZE, BADGE_SIZE));
        let (fill, edge) = if lit { (tokens.accent_dim, tokens.accent) } else { (tokens.button, tokens.button_border) };
        painter.rect(badge, CornerRadius::same(BADGE_ROUNDING), fill, Stroke::new(1.0, edge), StrokeKind::Inside);
        painter.text(badge.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(BADGE_GLYPH_SIZE), tokens.text);
        if lit {
            painter.rect_stroke(picture, 0, Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
        }

        self.paint_control_focus(&response);
        response
    }
}
