use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use egui_phosphor::regular as ic;

use super::{CONTROL_RADIUS, ICON_SIZE, PropertySheet, SMALL_TEXT_SIZE, TEXT_SIZE};
use crate::theme::semibold;

/// Air between neighbouring fields of one row
const FIELD_GAP: f32 = 4.0;

/// Room kept at the row's end for the link button, on every row of a group that has one
const LINK_SLOT: f32 = 26.0;

/// Size of the link button
pub(super) const LINK_SIZE: f32 = 18.0;

/// From a field's left edge to its axis letter's left edge
const TAG_LETTER_INSET: f32 = 7.0;

/// From a field's left edge to its glyph's centre
const TAG_GLYPH_CENTER: f32 = 10.0;

/// From a field's right edge to its value's right end
const VALUE_INSET: f32 = 6.0;

/// Least air between a field's tag and its value: a value that would come closer drops decimals
const TAG_VALUE_GAP: f32 = 5.0;

/// One part of a row of number fields: its tag (an axis letter in its colour, or a glyph) and value
pub(crate) struct NumberField<'a> {
    pub(crate) tag: &'a str,

    /// The axis letter's colour; `None` draws `tag` as a dim glyph
    pub(crate) tag_color: Option<Color32>,

    pub(crate) value: f32,

    /// Change per pixel dragged
    pub(crate) speed: f32,

    pub(crate) decimals: usize,

    /// A unit after the value ("°")
    pub(crate) suffix: &'a str,
}

/// The air between a field's axis tag and its typing box
const TAG_TO_TYPING: f32 = 6.0;

impl PropertySheet<'_> {
    /// A row of number fields (drag or type); returns the edited index and whether it continues a drag
    pub(crate) fn number_fields(&mut self, label: &str, fields: &mut [NumberField], link: Option<&mut bool>, keep_link_slot: bool) -> Option<(usize, bool)> {
        let (control, center_y) = self.row(label);
        let slot = if link.is_some() || keep_link_slot { LINK_SLOT } else { 0.0 };
        let count = fields.len().max(1) as f32;
        let width = (control.width() - slot - (count - 1.0) * FIELD_GAP) / count;

        let mut edited = None;
        for (index, field) in fields.iter_mut().enumerate() {
            let left = control.left() + index as f32 * (width + FIELD_GAP);
            let rect = Rect::from_min_size(pos2(left, control.top()), vec2(width, control.height()));
            if let Some(merge) = self.number_field(rect, label, field) {
                edited = Some((index, merge));
            }
        }

        if let Some(linked) = link {
            self.link_button(pos2(self.right - LINK_SIZE / 2.0, center_y), linked);
        }
        edited
    }

    /// The link that keeps two values together; returns whether it was clicked
    pub(super) fn link_button(&mut self, center: egui::Pos2, linked: &mut bool) -> bool {
        let rect = Rect::from_center_size(center, vec2(LINK_SIZE, LINK_SIZE));
        let id = self.next_id();
        let response = self.ui.interact(rect, id, Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *linked, crate::i18n::tr("Link")));
        let response = self.tip(response, crate::i18n::tr("Link"));
        if response.clicked() {
            *linked = !*linked;
        }

        let tokens = self.tokens;
        let (fill, border) = if *linked { (tokens.accent_dim, tokens.accent) } else { (tokens.button, tokens.button_border) };
        let painter = self.ui.painter();
        painter.rect(rect, CornerRadius::same(CONTROL_RADIUS), fill, Stroke::new(1.0, border), StrokeKind::Inside);
        let glyph = if *linked { ic::LINK_SIMPLE_HORIZONTAL } else { ic::LINK_SIMPLE_HORIZONTAL_BREAK };
        painter.text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(ICON_SIZE), tokens.text);
        response.clicked()
    }

    /// One field: its tag on the left, the value on the right; drag or click to type. Named "`label` `tag`" ("Scale X")
    fn number_field(&mut self, rect: Rect, label: &str, field: &mut NumberField) -> Option<bool> {
        let id = self.next_id();
        let typing_id = id.with((self.id, "typed"));
        let tokens = self.tokens;
        let painter = self.ui.painter().clone();
        painter.rect(rect, CornerRadius::same(CONTROL_RADIUS), tokens.field, Stroke::new(1.0, tokens.field_border), StrokeKind::Inside);
        match field.tag_color {
            Some(color) => {
                painter.text(pos2(rect.left() + TAG_LETTER_INSET, rect.center().y), Align2::LEFT_CENTER, field.tag, semibold(SMALL_TEXT_SIZE), color);
            }
            None => {
                painter.text(
                    pos2(rect.left() + TAG_GLYPH_CENTER, rect.center().y),
                    Align2::CENTER_CENTER,
                    field.tag,
                    FontId::proportional(ICON_SIZE),
                    tokens.text_dim,
                );
            }
        }

        let decimals = field.decimals;
        if let Some(mut draft) = self.typing_draft(typing_id) {
            let response =
                self.typing_edit(rect.left() + 2.0 * TAG_LETTER_INSET + TAG_TO_TYPING, rect.right() - VALUE_INSET, rect.center().y, typing_id, &mut draft);
            painter.rect_stroke(rect, CornerRadius::same(CONTROL_RADIUS), Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
            if !response.lost_focus() {
                self.keep_typing(typing_id, draft);
                return None;
            }

            let untouched = self.end_typing(typing_id, &draft);
            let text = draft.text.trim();
            if self.ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                return None;
            }

            // Leaving the field as it opened changes nothing: no undo step, no rounding to the shown decimals, no overwriting a value changed meanwhile
            if untouched || text == shown_number(field.value, decimals) {
                return None;
            }

            let typed: f32 = text.trim_end_matches(field.suffix).trim().parse().ok()?;
            if !typed.is_finite() || typed == field.value {
                return None;
            }

            field.value = typed;
            return Some(false);
        }

        self.skip_typing_edit();
        let response = self.ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(field.value), format!("{label} {}", field.tag)));
        let tag_right = match field.tag_color {
            Some(_) => rect.left() + TAG_LETTER_INSET + painter.layout_no_wrap(field.tag.to_owned(), semibold(SMALL_TEXT_SIZE), tokens.text).size().x,
            None => rect.left() + TAG_GLYPH_CENTER + ICON_SIZE / 2.0,
        };
        let room = rect.right() - VALUE_INSET - tag_right - TAG_VALUE_GAP;
        let shown = fitting_number(field.value, decimals, field.suffix, room, |text| {
            painter.layout_no_wrap(text.to_owned(), semibold(TEXT_SIZE), tokens.text).size().x
        });
        painter.text(pos2(rect.right() - VALUE_INSET, rect.center().y), Align2::RIGHT_CENTER, shown, semibold(TEXT_SIZE), tokens.text);
        if response.hovered() || response.dragged() {
            painter.rect_stroke(rect, CornerRadius::same(CONTROL_RADIUS), Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
        }

        if response.clicked() {
            self.start_typing(typing_id, shown_number(field.value, decimals));
            return None;
        }

        if response.dragged() {
            crate::pointer_lock::ask_for_hold(self.ui.ctx());
        }
        let delta = response.drag_delta().x;
        let moved = response.dragged() && delta != 0.0;
        if moved {
            field.value += delta * field.speed;
        }
        crate::widgets::press_edit(self.ui, response.id, moved)
    }
}

/// `value` and `suffix` with up to `decimals` decimals, as many as fit `room` wide (as measured by `width`), at least none
fn fitting_number(value: f32, decimals: usize, suffix: &str, room: f32, width: impl Fn(&str) -> f32) -> String {
    let mut decimals = decimals;
    loop {
        let shown = format!("{}{suffix}", shown_number(value, decimals));
        if decimals == 0 || width(&shown) <= room {
            return shown;
        }

        decimals -= 1;
    }
}

/// `value` with `decimals` decimals, a value that rounds to zero reading 0 without a minus sign
pub(super) fn shown_number(value: f32, decimals: usize) -> String {
    let shown = format!("{value:.decimals$}");
    let zero = shown.trim_start_matches('-').chars().all(|digit| matches!(digit, '0' | '.'));
    if zero { shown.trim_start_matches('-').to_owned() } else { shown }
}

#[cfg(test)]
mod tests {
    use super::fitting_number;

    /// Every character is 7 wide
    fn width(text: &str) -> f32 {
        7.0 * text.chars().count() as f32
    }

    #[test]
    fn a_value_too_wide_for_its_field_drops_decimals_before_it_reaches_the_tag() {
        assert_eq!(fitting_number(1.0, 2, "", 28.0, width), "1.00");
        assert_eq!(fitting_number(1.0, 2, "", 27.0, width), "1.0");
        assert_eq!(fitting_number(-12.5, 2, "°", 30.0, width), "-12°");
        assert_eq!(fitting_number(12345.0, 2, "", 1.0, width), "12345");
    }
}
