//! A row of section actions: buttons as wide as their words from the left, an on/off with its label at the right end

use egui::{Align2, CornerRadius, FontId, Rect, Role, Sense, Stroke, StrokeKind, WidgetInfo, pos2, vec2};

use super::switch_control::PILL_SIZE;
use super::{ActionRowResponse, CONTROL_HEIGHT, CONTROL_RADIUS, PropertySheet, ROW_HEIGHT, TEXT_SIZE};

/// Air on each side of a button's word
const BUTTON_WORD_AIR: f32 = 9.0;

/// Air between two buttons
const BUTTON_GAP: f32 = 6.0;

/// Air between the toggle's label and its pill
const TOGGLE_LABEL_AIR: f32 = 9.0;

/// From the right edge to the toggle's centre (the pill ends on the edge), and to its label's end
const TOGGLE_CENTER_INSET: f32 = PILL_SIZE.x / 2.0;
const TOGGLE_LABEL_INSET: f32 = PILL_SIZE.x + TOGGLE_LABEL_AIR;

impl PropertySheet<'_> {
    /// `buttons` (word, enabled) from the left edge, then `toggle_label` and its toggle at the right end
    pub(crate) fn action_row(&mut self, buttons: &[(&str, bool)], toggle_label: &str, on: &mut bool) -> ActionRowResponse {
        let row = self.take(ROW_HEIGHT);
        let center_y = row.center().y;
        self.drawn(center_y + CONTROL_HEIGHT / 2.0);
        let font = FontId::proportional(TEXT_SIZE);
        let tokens = self.tokens;

        let mut clicked = None;
        let mut x = self.left;
        for (index, (word, enabled)) in buttons.iter().enumerate() {
            let width = self.text_width(word, font.clone()) + 2.0 * BUTTON_WORD_AIR;
            let button = Rect::from_min_size(pos2(x, center_y - CONTROL_HEIGHT / 2.0), vec2(width, CONTROL_HEIGHT));
            let id = self.next_id();
            let response = self.ui.interact(button, id, if *enabled { Sense::click() } else { Sense::hover() });
            response.widget_info(|| WidgetInfo::labeled(Role::Button, *enabled, *word));
            if *enabled && response.clicked() {
                clicked = Some(index);
            }

            let fill = if *enabled && response.hovered() { tokens.row_hover } else { tokens.button };
            let painter = self.ui.painter();
            painter.rect(button, CornerRadius::same(CONTROL_RADIUS), fill, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
            painter.text(button.center(), Align2::CENTER_CENTER, *word, font.clone(), if *enabled { tokens.text } else { tokens.text_dim });
            self.paint_control_focus(&response);
            x = button.right() + BUTTON_GAP;
        }

        let label_end = self.right - TOGGLE_LABEL_INSET;
        let pill = pos2(self.right - TOGGLE_CENTER_INSET, center_y);
        let hit = Rect::from_min_max(pos2(label_end - self.text_width(toggle_label, font.clone()), row.top()), pos2(self.right, row.bottom()));
        let id = self.next_id();
        let response = self.ui.interact(hit, id, Sense::click());
        if response.clicked() {
            *on = !*on;
        }

        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, *on, toggle_label));
        self.ui.painter().text(pos2(label_end, center_y), Align2::RIGHT_CENTER, toggle_label, font, tokens.text);
        self.pill(pill, *on, true);
        self.paint_control_focus(&response);
        ActionRowResponse { clicked, toggled: response.clicked() }
    }
}
