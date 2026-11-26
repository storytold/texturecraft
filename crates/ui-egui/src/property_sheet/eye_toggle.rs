//! A dropdown row that ends in an eye: shows or hides what the setting draws over the 3D view

use egui::{Align2, CornerRadius, FontId, Painter, Rect, Role, Sense, Stroke, StrokeKind, WidgetInfo, pos2, vec2};
use egui_phosphor::regular as ic;

use super::choice::ChoicePicture;
use super::{CONTROL_HEIGHT, CONTROL_RADIUS, PropertySheet};

/// Air between the dropdown and its eye
const EYE_GAP: f32 = 6.0;

/// Size of the eye glyph
const EYE_GLYPH_SIZE: f32 = 14.0;

impl PropertySheet<'_> {
    /// [`Self::dropdown`] narrowed by an eye at the row's end (`eye`: open while shown, else slashed and dim; its tip); returns the pick and the eye's click
    pub(crate) fn dropdown_with_eye<T: Copy + PartialEq>(
        &mut self,
        label: &str,
        current: T,
        options: &[T],
        name: impl Fn(T) -> String,
        picture: Option<ChoicePicture<'_, T>>,
        (shown, tip): (bool, &str),
    ) -> (Option<T>, bool) {
        let right = self.right;
        self.right -= CONTROL_HEIGHT + EYE_GAP;
        let (picked, _) = self.dropdown(label, current, options, name, picture);
        self.right = right;

        let center_y = self.cursor_y() - super::ROW_HEIGHT / 2.0;
        let rect = Rect::from_center_size(pos2(right - CONTROL_HEIGHT / 2.0, center_y), vec2(CONTROL_HEIGHT, CONTROL_HEIGHT));
        let id = self.next_id();
        let response = self.ui.interact(rect, id, Sense::click());
        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, shown, tip));
        let response = self.tip(response, tip);
        paint_eye(self.ui.painter(), rect, shown, response.hovered(), &self.tokens);
        self.paint_control_focus(&response);
        (picked, response.clicked())
    }
}

/// The eye's square: button fill, its edge lit while hovered; the eye open, or slashed and dim
fn paint_eye(painter: &Painter, rect: Rect, shown: bool, hovered: bool, tokens: &crate::theme::Tokens) {
    let border = if hovered { tokens.accent } else { tokens.button_border };
    painter.rect(rect, CornerRadius::same(CONTROL_RADIUS), tokens.button, Stroke::new(1.0, border), StrokeKind::Inside);
    let (glyph, color) = if shown { (ic::EYE, tokens.text) } else { (ic::EYE_SLASH, tokens.text_dim) };
    painter.text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(EYE_GLYPH_SIZE), color);
}
