//! A slider that keeps the link slot at its end, for a pen-pressure button: the Size band's rows

use std::ops::RangeInclusive;

use egui::{Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};
use egui_phosphor::regular as ic;

use super::number_field::{LINK_SIZE, LINK_SLOT};
use super::{CONTROL_RADIUS, ICON_SIZE, PropertySheet, ROW_HEIGHT, RowLead};
use crate::i18n::{tr, trf};

/// Width of the pen button's outline, in points
const PEN_BUTTON_OUTLINE_WIDTH: f32 = 1.0;

impl PropertySheet<'_> {
    /// [`Self::slider`] ending one link slot early so a band's rows line up, with the `pen` button in that slot; returns the edit and whether the pen was clicked (it flips `pen`)
    pub(crate) fn pen_slider(
        &mut self,
        label: &str,
        value: &mut f32,
        range: RangeInclusive<f32>,
        decimals: usize,
        pen: Option<&mut bool>,
    ) -> (Option<bool>, bool) {
        let right = self.right;
        self.right -= LINK_SLOT;
        let edit = self.slider_led(RowLead::label(label), value, range, decimals, "");
        self.right = right;
        let center = Pos2::new(right - LINK_SIZE / 2.0, self.cursor_y() - ROW_HEIGHT / 2.0);
        let clicked = pen.is_some_and(|pressed| self.pen_button(center, label, pressed));
        (edit, clicked)
    }

    /// The pen button: lit while pen pressure drives row `label`'s value
    fn pen_button(&mut self, center: Pos2, label: &str, pressed: &mut bool) -> bool {
        let rect = Rect::from_center_size(center, vec2(LINK_SIZE, LINK_SIZE));
        let id = self.named_id("pen_pressure", label);
        let response = self.ui.interact(rect, id, Sense::click());

        let name = trf("{name} pen pressure", &[("name", &label)]);
        let enabled = self.ui.is_enabled();
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, *pressed, &name));
        let response = self.tip(response, tr("Pen pressure"));
        if response.clicked() {
            *pressed = !*pressed;
        }

        let tokens = self.tokens;
        let (fill, border) = if *pressed { (tokens.accent_dim, tokens.accent) } else { (tokens.button, tokens.button_border) };
        let painter = self.ui.painter();
        painter.rect(rect, CornerRadius::same(CONTROL_RADIUS), fill, Stroke::new(PEN_BUTTON_OUTLINE_WIDTH, border), StrokeKind::Inside);
        painter.text(rect.center(), Align2::CENTER_CENTER, ic::PEN, FontId::proportional(ICON_SIZE), tokens.text);
        self.paint_control_focus(&response);

        response.clicked()
    }
}
