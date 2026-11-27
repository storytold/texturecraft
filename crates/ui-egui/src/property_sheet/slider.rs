use std::ops::RangeInclusive;

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, WidgetInfo, pos2};
use egui_phosphor::regular as ic;

use super::number_field::shown_number;
use super::track::Track;
use super::value_text::ValueText;
use super::{CONTROL_RADIUS, ICON_SIZE, PropertySheet, ROW_HIT_HALF_HEIGHT, RowLead, SMALL_TEXT_SIZE, TEXT_SIZE};
use crate::theme::semibold;

/// Radius of a slider's knob
pub(super) const KNOB_RADIUS: f32 = 5.0;

/// Width of a slider's knob outline
pub(super) const KNOB_STROKE: f32 = 1.5;

/// Width of a slider's track
const TRACK_WIDTH: f32 = 2.0;

/// Half the height of a signed slider's zero tick
const ZERO_TICK_HALF: f32 = 4.0;

/// Radius of an angle row's dial
pub(super) const DIAL_RADIUS: f32 = 8.0;

/// The quarter turn back from the screen's 0 rad (pointing right) that points the dial's hand straight up at 0°
const DIAL_ZERO_DEGREES: f32 = 90.0;

/// Air between the dial and the value, and between the track's end and the dial
pub(super) const DIAL_AIR: f32 = 6.0;

/// Air between a tinted slider's track end and its right-hand color name
const TINT_WORD_AIR: f32 = 8.0;

/// Half the height of Intensity's divider chip
const CHIP_HALF_HEIGHT: f32 = 10.0;

/// The chip's room around its number: the caret and the air on both sides
const CHIP_PADDING: f32 = 24.0;

/// Air between the chip and the value, beyond the panel's value column
const CHIP_TO_VALUE: f32 = 18.0;

/// Where a slider's unit chip draws its caret and its text, from the chip's right end
const CHIP_CARET_INSET: f32 = 9.0;
const CHIP_TEXT_INSET: f32 = 18.0;

impl PropertySheet<'_> {
    /// A bounded number; `Some(merge)` when `value` changed, `merge` true for a drag's later steps
    pub(crate) fn slider(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, decimals: usize) -> Option<bool> {
        self.slider_with_suffix(label, value, range, decimals, "")
    }

    /// [`Self::slider`] with a unit after the value ("°", "%")
    pub(crate) fn slider_with_suffix(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, decimals: usize, suffix: &str) -> Option<bool> {
        self.slider_led(RowLead::label(label), value, range, decimals, suffix)
    }

    /// [`Self::slider_with_suffix`] led by `lead` (a picture row, a child setting)
    pub(crate) fn slider_led(&mut self, lead: RowLead<'_>, value: &mut f32, range: RangeInclusive<f32>, decimals: usize, suffix: &str) -> Option<bool> {
        let (control, center_y) = self.row_with(lead);
        let id = self.named_id("slider", lead.text());
        let track = Track { start: control.left() + KNOB_RADIUS, end: self.track_end(), center_y, zero: None, ramp: false };
        let dragged = self.track(id, lead.text(), &track, value, &range);
        let typed = self.value_field(id, lead.text(), center_y, *value, &range, decimals, suffix);
        Self::apply_typed(value, typed, dragged)
    }

    /// A signed range: fills from zero and marks it with a tick
    pub(crate) fn signed_slider(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, decimals: usize) -> Option<bool> {
        self.centered_slider(label, value, range, 0.0, decimals)
    }

    /// A range whose middle `centre` leaves things as they are: fills from it and marks it with a tick, like a signed range
    pub(crate) fn centered_slider(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, centre: f32, decimals: usize) -> Option<bool> {
        let (control, center_y) = self.row(label);
        let id = self.next_id();
        let track = Track { start: control.left() + KNOB_RADIUS, end: self.track_end(), center_y, zero: Some(centre), ramp: false };
        let dragged = self.track(id, label, &track, value, &range);
        let typed = self.value_field(id, label, center_y, *value, &range, decimals, "");
        Self::apply_typed(value, typed, dragged)
    }

    /// A −1..1 balance between two tinted color names, left as the label and right after the track (`rights`: the group's right names)
    pub(crate) fn tinted_slider(&mut self, left: (&str, Color32), right: (&str, Color32), rights: &[&str], value: &mut f32) -> Option<bool> {
        let tokens = self.tokens;
        let readable = |tint: Color32| crate::theme::legible_tint(tint, tokens.row, tokens.text);
        let (control, center_y) = self.row_with(RowLead::Tinted { text: left.0, color: readable(left.1) });
        let id = self.next_id();
        let widest = rights.iter().map(|name| self.text_width(name, FontId::proportional(TEXT_SIZE))).fold(0.0, f32::max);
        let word_x = self.track_end() - widest;
        let track = Track { start: control.left() + KNOB_RADIUS, end: word_x - TINT_WORD_AIR, center_y, zero: Some(0.0), ramp: false };
        let range = -1.0..=1.0;
        let dragged = self.track(id, left.0, &track, value, &range);
        self.ui.painter().text(pos2(word_x, center_y), Align2::LEFT_CENTER, right.0, FontId::proportional(TEXT_SIZE), readable(right.1));
        let typed = self.value_field(id, left.0, center_y, *value, &range, 2, "");
        Self::apply_typed(value, typed, dragged)
    }

    /// An angle in degrees: the track, then a small dial just before the value
    pub(crate) fn angle(&mut self, label: &str, degrees: &mut f32, range: RangeInclusive<f32>) -> Option<bool> {
        self.angle_led(RowLead::label(label), degrees, range)
    }

    /// [`Self::angle`] led by `lead`
    pub(crate) fn angle_led(&mut self, lead: RowLead<'_>, degrees: &mut f32, range: RangeInclusive<f32>) -> Option<bool> {
        self.angle_shown(lead, degrees, range, false)
    }

    /// An angle added to others: a signed track over `range` with its zero tick, the dial, and the value with its sign ("+30°")
    pub(crate) fn relative_angle(&mut self, label: &str, degrees: &mut f32, range: RangeInclusive<f32>) -> Option<bool> {
        self.angle_shown(RowLead::label(label), degrees, range, true)
    }

    /// [`Self::angle_led`], the value shown with its sign when `plus`
    fn angle_shown(&mut self, lead: RowLead<'_>, degrees: &mut f32, range: RangeInclusive<f32>, plus: bool) -> Option<bool> {
        let (control, center_y) = self.row_with(lead);
        let id = self.next_id();
        let dial_x = self.right - self.used.value_width.max(super::VALUE_MIN_WIDTH) - DIAL_AIR - DIAL_RADIUS;
        let track = Track {
            start: control.left() + KNOB_RADIUS,
            end: dial_x - DIAL_RADIUS - DIAL_AIR,
            center_y,
            zero: (*range.start() < 0.0).then_some(0.0),
            ramp: false,
        };
        let name = lead.text();
        let dragged = self.track(id, name, &track, degrees, &range);

        self.paint_dial(pos2(dial_x, center_y), *degrees);

        let shown = if plus { format!("{:+.0}°", *degrees) } else { format!("{:.0}°", *degrees) };
        let typed = self.column_value_field(id, &name, center_y, &shown, "°", *degrees, &range, 0);
        Self::apply_typed(degrees, typed, dragged)
    }

    /// The small dial of an angle row centred on `center`, its hand at `degrees`
    pub(super) fn paint_dial(&self, center: egui::Pos2, degrees: f32) {
        let tokens = self.tokens;
        let painter = self.ui.painter();
        painter.circle_stroke(center, DIAL_RADIUS, Stroke::new(KNOB_STROKE, tokens.button_border));
        let turn = (degrees - DIAL_ZERO_DEGREES).to_radians();
        let hand = pos2(center.x + DIAL_RADIUS * turn.cos(), center.y + DIAL_RADIUS * turn.sin());
        painter.line_segment([center, hand], Stroke::new(TRACK_WIDTH, tokens.text));
    }

    /// A slider with a small dropdown chip before its value (Intensity's divider); returns the edit and the chip
    pub(crate) fn slider_with_chip(
        &mut self,
        label: &str,
        value: &mut f32,
        range: RangeInclusive<f32>,
        decimals: usize,
        chip: &str,
        widest_chip: &str,
    ) -> (Option<bool>, Response) {
        let (control, center_y) = self.row(label);
        let id = self.next_id();
        let chip_end = self.right - self.used.value_width.max(super::VALUE_MIN_WIDTH) - CHIP_TO_VALUE;
        let chip_width = self.text_width(widest_chip, FontId::proportional(TEXT_SIZE)) + CHIP_PADDING;
        let chip_rect = Rect::from_min_max(pos2(chip_end - chip_width, center_y - CHIP_HALF_HEIGHT), pos2(chip_end, center_y + CHIP_HALF_HEIGHT));
        let chip_response = self.ui.interact(chip_rect, id.with("chip"), Sense::click());
        chip_response.widget_info(|| WidgetInfo::labeled(egui::Role::ComboBox, true, chip));

        let tokens = self.tokens;
        let open = egui::Popup::is_id_open(self.ui.ctx(), egui::Popup::default_response_id(&chip_response));
        let painter = self.ui.painter();
        painter.rect_filled(chip_rect, CornerRadius::same(CONTROL_RADIUS), if chip_response.hovered() { tokens.row_hover } else { tokens.button });
        if open {
            painter.rect_stroke(chip_rect, CornerRadius::same(CONTROL_RADIUS), Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
        }
        painter.text(pos2(chip_end - CHIP_CARET_INSET, center_y), Align2::CENTER_CENTER, ic::CARET_DOWN, FontId::proportional(ICON_SIZE), tokens.text);
        painter.text(pos2(chip_end - CHIP_TEXT_INSET, center_y), Align2::RIGHT_CENTER, chip, semibold(SMALL_TEXT_SIZE), tokens.text);

        let track = Track { start: control.left() + KNOB_RADIUS, end: chip_rect.left() - DIAL_AIR - KNOB_RADIUS, center_y, zero: None, ramp: false };
        let dragged = self.track(id, label, &track, value, &range);
        let typed = self.value_field(id, label, center_y, *value, &range, decimals, "");
        (Self::apply_typed(value, typed, dragged), chip_response)
    }

    /// A typed value wins over a drag in the same frame and starts its own undo step
    pub(super) fn apply_typed(value: &mut f32, typed: Option<f32>, dragged: Option<bool>) -> Option<bool> {
        match typed {
            Some(typed) => {
                *value = typed;
                Some(false)
            }
            None => dragged,
        }
    }

    /// Paints a track with its knob and moves `value` while it is clicked or dragged
    pub(super) fn track(&mut self, id: Id, label: impl std::fmt::Display, track: &Track, value: &mut f32, range: &RangeInclusive<f32>) -> Option<bool> {
        let fill = self.tokens.accent;
        self.track_filled(id, label, track, value, range, fill)
    }

    /// [`Self::track`] whose fill is `fill` (an axis card's in its axis color)
    pub(super) fn track_filled(
        &mut self,
        id: Id,
        label: impl std::fmt::Display,
        track: &Track,
        value: &mut f32,
        range: &RangeInclusive<f32>,
        fill: Color32,
    ) -> Option<bool> {
        let (minimum, maximum) = (*range.start(), *range.end());
        let span = (maximum - minimum).max(f32::EPSILON);
        let length = (track.end - track.start).max(1.0);
        let hit = Rect::from_min_max(
            pos2(track.start - KNOB_RADIUS, track.center_y - ROW_HIT_HALF_HEIGHT),
            pos2(track.end + KNOB_RADIUS, track.center_y + ROW_HIT_HALF_HEIGHT),
        );
        let response = self.ui.interact(hit, id, Sense::click_and_drag());
        let enabled = self.ui.is_enabled();
        response.widget_info(|| WidgetInfo::slider(enabled, f64::from(*value), &label));

        // A press on the track drops what was typed in its value: the track's value is the one wanted
        if response.is_pointer_button_down_on() {
            self.drop_typing(id.with((self.id, "typed")));
        }

        let mut moved = false;
        if (response.dragged() || response.drag_stopped() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let picked = kept_in(minimum + (pointer.x - track.start) / length * span, range);
            if picked != *value {
                *value = picked;
                moved = true;
            }
        }

        // A press's first change starts an undo step, its later changes continue it
        let edit = crate::widgets::press_edit(self.ui, response.id, moved);

        let tokens = self.tokens;
        let at = |v: f32| track.start + ((v - minimum) / span).clamp(0.0, 1.0) * length;
        let knob_x = at(*value);
        let zero_x = track.zero.map_or(track.start, |zero| at(kept_in(zero, range)));
        let painter = self.ui.painter();
        if !track.ramp {
            painter.line_segment([pos2(track.start, track.center_y), pos2(track.end, track.center_y)], Stroke::new(TRACK_WIDTH, tokens.button_border));
            painter.line_segment([pos2(zero_x.min(knob_x), track.center_y), pos2(zero_x.max(knob_x), track.center_y)], Stroke::new(TRACK_WIDTH, fill));
        }
        if track.zero.is_some() {
            let tick = [pos2(zero_x, track.center_y - ZERO_TICK_HALF), pos2(zero_x, track.center_y + ZERO_TICK_HALF)];
            painter.line_segment(tick, Stroke::new(1.0, tokens.text_dim));
        }
        let outline = if response.hovered() || response.dragged() { tokens.accent } else { tokens.text };
        painter.circle(pos2(knob_x, track.center_y), KNOB_RADIUS, tokens.panel, Stroke::new(KNOB_STROKE, outline));
        self.paint_control_focus(&response);
        edit
    }

    /// The value at the row's end, answering as `label`'s value; a click turns it into a field, which returns the typed value when done
    #[allow(clippy::too_many_arguments)]
    pub(super) fn value_field(
        &mut self,
        id: Id,
        label: impl std::fmt::Display,
        center_y: f32,
        value: f32,
        range: &RangeInclusive<f32>,
        decimals: usize,
        suffix: &str,
    ) -> Option<f32> {
        let shown = format!("{}{suffix}", shown_number(value, decimals));
        self.column_value_field(id, &label, center_y, &shown, suffix, value, range, decimals)
    }

    /// [`Self::value_field`] reading `shown`: the panel's value column grows to fit it
    #[allow(clippy::too_many_arguments)]
    pub(super) fn column_value_field(
        &mut self,
        id: Id,
        label: &dyn std::fmt::Display,
        center_y: f32,
        shown: &str,
        suffix: &str,
        value: f32,
        range: &RangeInclusive<f32>,
        decimals: usize,
    ) -> Option<f32> {
        self.measure_value(shown);
        let width = self.used.value_width.max(super::VALUE_MIN_WIDTH);
        self.value_text_field(id, ValueText { label, right: self.right, width, center_y, shown, suffix }, value, range, decimals)
    }

    /// [`Self::value_field`] ending at `right`, `width` wide, outside the panel's value column
    #[allow(clippy::too_many_arguments)]
    pub(super) fn value_field_at(
        &mut self,
        id: Id,
        label: impl std::fmt::Display,
        right: f32,
        width: f32,
        center_y: f32,
        value: f32,
        range: &RangeInclusive<f32>,
        decimals: usize,
        suffix: &str,
    ) -> Option<f32> {
        let shown = format!("{}{suffix}", shown_number(value, decimals));
        self.value_text_field(id, ValueText { label: &label, right, width, center_y, shown: &shown, suffix }, value, range, decimals)
    }

    /// A value drawn as `text.shown` that turns into a field when clicked; returns the typed value (its `text.suffix` and a leading "×" or "x" allowed), kept in `range`
    pub(super) fn value_text_field(&mut self, id: Id, value_text: ValueText<'_>, value: f32, range: &RangeInclusive<f32>, decimals: usize) -> Option<f32> {
        let ValueText { label, right, width, center_y, shown, suffix } = value_text;
        let rect = Rect::from_min_max(pos2(right - width, center_y - ROW_HIT_HALF_HEIGHT), pos2(right, center_y + ROW_HIT_HALF_HEIGHT));
        let field_id = id.with((self.id, "typed"));
        let editing = self.typing_draft(field_id);
        let seeded = || shown_number(value, decimals);

        let tokens = self.tokens;
        let Some(mut draft) = editing else {
            self.skip_typing_edit();
            let response = self.ui.interact(rect, id.with("value"), Sense::click()).on_hover_cursor(egui::CursorIcon::Text);
            let enabled = self.ui.is_enabled();
            response.widget_info(|| WidgetInfo::labeled(egui::Role::Label, enabled, crate::i18n::trf("{label} value", &[("label", label)])));
            self.ui.painter().text(pos2(right, center_y), Align2::RIGHT_CENTER, shown, semibold(TEXT_SIZE), tokens.text);

            if response.hovered() {
                self.value_line(rect.left(), right, center_y, tokens.field_border);
            }
            self.paint_control_focus(&response);
            if response.clicked() {
                self.start_typing(field_id, seeded());
            }
            return None;
        };

        // Typing happens on the value's own line: no box, the line under it goes accent
        let response = self.typing_edit(rect.left(), right, center_y, field_id, &mut draft);
        self.value_line(rect.left(), right, center_y, tokens.accent);
        if !response.lost_focus() {
            self.keep_typing(field_id, draft);
            return None;
        }

        let untouched = self.end_typing(field_id, &draft);
        let typed_text = draft.text.trim();
        if self.escape_pressed() {
            return None;
        }

        // An unchanged draft must preserve full precision and any intervening edit
        if untouched || typed_text == seeded() {
            return None;
        }

        let typed: f32 = typed_text.trim_end_matches(suffix).trim_start_matches(['×', 'x']).trim().parse().ok()?;

        // A typed value that lands on the current one (a clamp, "0.750" for 0.75) is no edit either
        typed.is_finite().then(|| kept_in(typed, range)).filter(|kept| *kept != value)
    }
}

/// `value` kept in `range`; unlike `clamp`, a reversed or NaN range never panics
pub(super) fn kept_in(value: f32, range: &RangeInclusive<f32>) -> f32 {
    value.max(*range.start()).min(*range.end())
}
