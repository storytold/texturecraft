use egui::{CornerRadius, FontId, Sense, Stroke, StrokeKind, pos2};

use super::{ButtonIcon, CONTROL_RADIUS, PropertySheet, ROW_HEIGHT, TEXT_SIZE};

/// Air between a full-width button's icon and its words
const BUTTON_ICON_GAP: f32 = 8.0;

/// Size of a full-width button's icon
const BUTTON_ICON_SIZE: f32 = 14.0;

impl PropertySheet<'_> {
    /// A full-width button: centred icon and words
    pub(crate) fn button<'a>(&mut self, icon: impl Into<ButtonIcon<'a>>, text: &str) -> egui::Response {
        self.button_lit(icon, text, false)
    }

    /// A full-width button lit in the accent while what it turns on stays on
    pub(crate) fn button_lit<'a>(&mut self, icon: impl Into<ButtonIcon<'a>>, text: &str, lit: bool) -> egui::Response {
        self.button_enabled(icon, text, lit, true)
    }

    /// [`Self::button_lit`] that answers no clicks and dims its words while not `enabled`
    pub(crate) fn button_enabled<'a>(&mut self, icon: impl Into<ButtonIcon<'a>>, text: &str, lit: bool, enabled: bool) -> egui::Response {
        self.button_sensing(icon.into(), text, lit, if enabled { Sense::click() } else { Sense::hover() })
    }

    fn button_sensing(&mut self, icon: ButtonIcon<'_>, text: &str, lit: bool, sense: Sense) -> egui::Response {
        let enabled = sense != Sense::hover();
        let rect = self.take(ROW_HEIGHT);
        let button = self.control_box(self.left, rect.center().y);
        let id = self.next_id();
        let response = self.ui.interact(button, id, sense);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));

        let tokens = self.tokens;
        let painter = self.ui.painter();
        painter.rect(
            button,
            CornerRadius::same(CONTROL_RADIUS),
            match (lit, enabled && response.hovered()) {
                (true, _) => tokens.accent_dim,
                (false, true) => tokens.row_hover,
                (false, false) => tokens.button,
            },
            Stroke::new(1.0, if lit { tokens.accent } else { tokens.button_border }),
            StrokeKind::Inside,
        );
        let color = if enabled { tokens.text } else { tokens.text_dim };
        let font = FontId::proportional(TEXT_SIZE);
        let (icon_width, icon_galley) = match icon {
            ButtonIcon::Glyph(glyph) => {
                let galley = painter.layout_no_wrap(glyph.to_owned(), FontId::proportional(BUTTON_ICON_SIZE), color);
                (galley.size().x, Some(galley))
            }
        };
        let text_galley = painter.layout_no_wrap(text.to_owned(), font, color);
        let total = icon_width + BUTTON_ICON_GAP + text_galley.size().x;
        let x = button.center().x - total / 2.0;
        if let Some(galley) = icon_galley {
            painter.galley(pos2(x, button.center().y - galley.size().y / 2.0), galley, color);
        }
        painter.galley(pos2(x + icon_width + BUTTON_ICON_GAP, button.center().y - text_galley.size().y / 2.0), text_galley, color);
        self.paint_control_focus(&response);
        self.drawn(button.bottom());
        response
    }
}
