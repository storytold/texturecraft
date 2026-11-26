//! Rows a symmetry needs: a section caption with its own switch, sliders lettered by axis, and a picture beside a stack of sliders

use std::ops::RangeInclusive;

use egui::{Align2, Color32, FontId, Painter, Rect, Response, Role, Sense, Stroke, StrokeKind, WidgetInfo, pos2, vec2};

use super::row::SECTION_CAPTION_CENTER;
use super::slider::{DIAL_AIR, DIAL_RADIUS, KNOB_RADIUS};
use super::switch_control::PILL_SIZE;
use super::track::Track;
use super::{BLOCK_MARGIN, PropertySheet, SMALL_TEXT_SIZE, StackSlider, TEXT_SIZE};
use crate::theme::semibold;

/// From the right edge to the centre of a caption's switch: the pill ends on the edge
const CAPTION_SWITCH_INSET: f32 = PILL_SIZE.x / 2.0;

/// The click area around a caption's switch
const CAPTION_SWITCH_HIT: egui::Vec2 = vec2(30.0, 20.0);

/// Least air between a caption and the reason standing in for its switch
const UNAVAILABLE_AIR: f32 = 12.0;

/// From the controls column to where a lettered slider's track starts, past its axis letter
const LETTER_TO_TRACK: f32 = 16.0;

/// From one part of a lettered value to the next
const PART_LINE: f32 = 22.0;

/// A stacked slider's caption line, its track's line, and the air between stacked sliders
const STACK_CAPTION_LINE: f32 = 18.0;
const STACK_PART_LINE: f32 = 22.0;
const STACK_GAP: f32 = 6.0;

/// An axis card's air above its name row, the edge of its letter chip, the air from the chip to the name, and its full-width track's line
const AXIS_CARD_TOP: f32 = 4.0;
const AXIS_CHIP_SIZE: f32 = 18.0;
const AXIS_CHIP_TO_NAME: f32 = 8.0;
const AXIS_TRACK_LINE: f32 = 20.0;

/// The letter on an axis chip: dark on every axis color, in every theme
const AXIS_CHIP_LETTER: Color32 = Color32::from_rgb(20, 20, 24);

/// The picture's top below the block's (the first caption's letters), how much shorter it is than the stack, and its air before the controls column
const PICTURE_TOP: f32 = 4.0;
const PICTURE_SHORTER: f32 = 10.0;
const PICTURE_TO_COLUMN: f32 = 6.0;

impl PropertySheet<'_> {
    /// A section caption with its on/off switch at the right edge
    pub(crate) fn section_switch(&mut self, title: &str, on: bool) -> Response {
        self.section_in(title, false, self.tokens.text);
        let top = self.sections.last().map_or(self.cursor_y(), |(_, top)| *top);
        let center = pos2(self.right - CAPTION_SWITCH_INSET, top + SECTION_CAPTION_CENTER);
        let id = self.next_id();
        let response = self.ui.interact(Rect::from_center_size(center, CAPTION_SWITCH_HIT), id, Sense::click());
        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, on, title));
        self.pill(center, on, true);
        self.paint_control_focus(&response);
        response
    }

    /// A dim section caption whose switch can't be used: `reason` stands in its place at the right edge, cut with "…" to fit (then its tooltip reads it whole)
    pub(crate) fn section_unavailable(&mut self, title: &str, reason: &str) {
        let tokens = self.tokens;
        self.section_in(title, false, tokens.text_dim);
        let top = self.sections.last().map_or(self.cursor_y(), |(_, top)| *top);
        let center_y = top + SECTION_CAPTION_CENTER;
        let painter = self.ui.painter().clone();
        let caption = painter.layout_no_wrap(title.to_uppercase(), semibold(SMALL_TEXT_SIZE), tokens.text_dim).size().x;
        let room = self.right - self.left - caption - UNAVAILABLE_AIR;
        let galley = crate::widgets::elided(&painter, reason, SMALL_TEXT_SIZE, tokens.text_dim, room);
        let cut = galley.elided;
        let rect = Rect::from_min_size(pos2(self.right - galley.size().x, center_y - galley.size().y / 2.0), galley.size());
        painter.galley(rect.min, galley, tokens.text_dim);
        let id = self.next_id();
        let response = self.ui.interact(rect, id, Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(Role::Label, false, reason));
        if cut {
            self.tip(response, reason);
        }
    }

    /// Draw lettered sliders and report whether their combined edit continues the previous undo step.
    pub(crate) fn lettered_sliders(&mut self, label: &str, parts: &mut [(&str, Color32, f32)], range: RangeInclusive<f32>, decimals: usize) -> Option<bool> {
        let (control, first) = self.row(label);
        let extra = parts.len().saturating_sub(1) as f32 * PART_LINE;
        if extra > 0.0 {
            self.take(extra);
        }

        let mut edit = None;
        for (index, (letter, color, value)) in parts.iter_mut().enumerate() {
            let center_y = first + index as f32 * PART_LINE;
            self.ui.painter().text(pos2(control.left(), center_y), Align2::LEFT_CENTER, *letter, semibold(TEXT_SIZE), *color);
            let id = self.next_id();
            let track = Track { start: control.left() + LETTER_TO_TRACK + KNOB_RADIUS, end: self.track_end(), center_y, zero: Some(0.0), ramp: false };
            let name = format_args!("{label} {letter}");
            let dragged = self.track(id, name, &track, value, &range);
            let typed = self.value_field(id, name, center_y, *value, &range, decimals, "");
            if let Some(merge) = Self::apply_typed(value, typed, dragged) {
                edit = Some(edit.unwrap_or(true) && merge);
            }
            self.drawn(center_y + super::CONTROL_HEIGHT / 2.0);
        }

        edit
    }

    /// A card for one axis: its letter on a chip in `tint`, the bold name and the value, then a full-width slider filled in `tint`; the axis's other rows follow, then [`Self::close_card`]
    pub(crate) fn axis_card(
        &mut self,
        (letter, tint): (&str, Color32),
        name: &str,
        value: &mut f32,
        range: RangeInclusive<f32>,
        decimals: usize,
    ) -> Option<bool> {
        self.open_card();
        self.scope = self.named_id("axis_card", name);
        self.space(AXIS_CARD_TOP);
        let head = self.take(super::ROW_HEIGHT);
        let center_y = head.center().y;
        let chip = Rect::from_center_size(pos2(self.left + AXIS_CHIP_SIZE / 2.0, center_y), vec2(AXIS_CHIP_SIZE, AXIS_CHIP_SIZE));
        let painter = self.ui.painter();
        painter.rect_filled(chip, egui::CornerRadius::same(super::CONTROL_RADIUS), tint);
        painter.text(chip.center(), Align2::CENTER_CENTER, letter, semibold(SMALL_TEXT_SIZE), AXIS_CHIP_LETTER);
        painter.text(pos2(chip.right() + AXIS_CHIP_TO_NAME, center_y), Align2::LEFT_CENTER, name, semibold(TEXT_SIZE), self.tokens.text);

        let id = self.next_id();
        let typed = self.value_field(id, name, center_y, *value, &range, decimals, "");
        let track_y = self.take(AXIS_TRACK_LINE).center().y;
        let track = Track { start: self.left + KNOB_RADIUS, end: self.right - KNOB_RADIUS, center_y: track_y, zero: None, ramp: false };
        let dragged = self.track_filled(id, name, &track, value, &range, tint);
        self.drawn(track_y + KNOB_RADIUS);
        Self::apply_typed(value, typed, dragged)
    }

    /// Draw a picture beside captioned sliders and report the edit on each slider.
    pub(crate) fn picture_sliders<const N: usize>(&mut self, picture: &dyn Fn(&Painter, Rect), sliders: &mut [StackSlider<'_>; N]) -> [Option<bool>; N] {
        let count = sliders.len().max(1) as f32;
        let height = count * (STACK_CAPTION_LINE + STACK_PART_LINE) + (count - 1.0) * STACK_GAP;
        self.space(BLOCK_MARGIN);
        let block = self.take(height);
        let x0 = self.column();
        let side = (height - PICTURE_SHORTER).min(x0 - self.left - PICTURE_TO_COLUMN).max(1.0);
        let square = Rect::from_min_size(pos2(self.left, block.top() + PICTURE_TOP), vec2(side, side));
        picture(self.ui.painter(), square);
        self.ui.painter().rect_stroke(square, 0, Stroke::new(1.0, self.tokens.field_border), StrokeKind::Inside);

        let mut edits = [None; N];
        let mut y = block.top();
        for (edit, slider) in edits.iter_mut().zip(sliders) {
            let dim = self.tokens.text_dim;
            let caption = pos2(x0, y + STACK_CAPTION_LINE / 2.0);
            self.ui.painter().text(caption, Align2::LEFT_CENTER, slider.label, FontId::proportional(SMALL_TEXT_SIZE), dim);
            let center_y = y + STACK_CAPTION_LINE + STACK_PART_LINE / 2.0;
            let id = self.next_id();
            let dial_x = self.right - self.used.value_width.max(super::VALUE_MIN_WIDTH) - DIAL_AIR - DIAL_RADIUS;
            let end = if slider.dial { dial_x - DIAL_RADIUS - DIAL_AIR } else { self.track_end() };
            let track = Track { start: x0 + KNOB_RADIUS, end, center_y, zero: None, ramp: false };
            if slider.dial {
                self.paint_dial(pos2(dial_x, center_y), *slider.value);
            }
            let dragged = self.track(id, slider.label, &track, slider.value, &slider.range);
            let typed = self.value_field(id, slider.label, center_y, *slider.value, &slider.typed, slider.decimals, slider.suffix);
            *edit = Self::apply_typed(slider.value, typed, dragged);
            y += STACK_CAPTION_LINE + STACK_PART_LINE + STACK_GAP;
        }

        self.drawn(square.bottom().max(block.bottom() - STACK_PART_LINE / 2.0 + KNOB_RADIUS));
        self.space(BLOCK_MARGIN);
        edits
    }
}
