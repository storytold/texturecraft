use egui::{Align2, FontId, Painter, Rect, Stroke, StrokeKind, pos2, vec2};
use egui_phosphor::regular as ic;

use super::number_field::LINK_SIZE;
use super::preview_edit::PreviewEdit;
use super::preview_lower::PreviewLower;
use super::preview_size::PreviewSize;
use super::row::CAPTION_LINE;
use super::track::Track;
use super::{BLOCK_MARGIN, ICON_SIZE, PropertySheet, SMALL_TEXT_SIZE};

/// One slider line beside the preview square
const PART_LINE: f32 = 22.0;

/// Air between the size and what follows it
const PART_GAP: f32 = 6.0;

/// From the block's top to the square's top: the first caption's letters
const SQUARE_TOP: f32 = 4.0;

/// The square stops this far above the block's bottom: the last knob's bottom
const SQUARE_SHORTER: f32 = 10.0;

/// Air between the square and the controls beside it
const SQUARE_TO_SIDE: f32 = 12.0;

/// From the side's left edge to a direction glyph's centre, and to the track after it
const GLYPH_CENTER: f32 = 5.0;
const GLYPH_TRACK: f32 = 18.0;

/// Room kept at the right edge for the link, on every line, so the tracks end together
pub(super) const LINK_ROOM: f32 = 24.0;

/// The value column's width beside the square: every track ends this far left of the value's end
const SIDE_VALUE_WIDTH: f32 = 46.0;

/// The air kept between a side slider's track and its value
const SIDE_VALUE_AIR: f32 = 8.0;

impl PropertySheet<'_> {
    /// A preview square beside its stack (the size, then Offset or one setting), exactly as tall as the stack
    pub(crate) fn preview_block(&mut self, picture: &dyn Fn(&Painter, Rect), size: PreviewSize<'_>, lower: PreviewLower<'_>) -> PreviewEdit {
        let size_rows = if *size.linked { 1 } else { 2 };
        let lower_rows = match lower {
            PreviewLower::Offset { .. } => 2,
            PreviewLower::Slider { .. } => 1,
        };
        let height = CAPTION_LINE + PART_LINE * size_rows as f32 + PART_GAP + CAPTION_LINE + PART_LINE * lower_rows as f32;
        self.space(BLOCK_MARGIN);
        let block = self.take(height);
        let top = block.top();

        let side = (height - SQUARE_SHORTER).max(1.0);
        let square = Rect::from_min_size(pos2(self.left, top + SQUARE_TOP), vec2(side, side));
        picture(self.ui.painter(), square);
        self.ui.painter().rect_stroke(square, 0, Stroke::new(1.0, self.tokens.field_border), StrokeKind::Inside);

        let x0 = square.right() + SQUARE_TO_SIDE;
        let value_right = self.right - LINK_ROOM;
        let track_end = value_right - SIDE_VALUE_WIDTH;
        let mut edit = PreviewEdit { square, size: None, lower: None };
        let mut y = top;

        self.side_caption(x0, y, size.label);
        y += CAPTION_LINE;
        let linked = *size.linked;
        for part in 0..size_rows {
            let center_y = y + PART_LINE / 2.0;
            let start = if linked { x0 } else { self.direction_glyph(x0, center_y, part) };
            let mut value = size.values[part];
            let label = format_args!("{}{}", size.label, part_suffix(part));
            if let Some(merge) = self.side_slider(label, start, track_end, value_right, center_y, &mut value, (&size.range, &size.typed), size.decimals, false)
            {
                size.values[part] = value;
                if linked {
                    size.values[1] = value;
                }
                edit.size = Some(edit.size.unwrap_or(true) && merge);
            }
            y += PART_LINE;
        }
        let link_center = pos2(self.right - LINK_SIZE / 2.0, y - PART_LINE / 2.0);
        if self.link_button(link_center, size.linked) && *size.linked && size.values[1] != size.values[0] {
            size.values[1] = size.values[0];
            edit.size = Some(false);
        }
        y += PART_GAP;

        match lower {
            PreviewLower::Offset { values, range, typed } => {
                let offset = crate::i18n::tr("Offset");
                self.side_caption(x0, y, offset);
                y += CAPTION_LINE;
                for part in 0..2 {
                    let center_y = y + PART_LINE / 2.0;
                    let start = self.direction_glyph(x0, center_y, part);
                    let label = format_args!("{offset}{}", part_suffix(part));
                    if let Some(merge) = self.side_slider(label, start, track_end, value_right, center_y, &mut values[part], (&range, &typed), 2, true) {
                        edit.lower = Some(edit.lower.unwrap_or(true) && merge);
                    }
                    y += PART_LINE;
                }
            }
            PreviewLower::Slider { label, value, range, decimals } => {
                self.side_caption(x0, y, label);
                y += CAPTION_LINE;
                if let Some(merge) = self.side_slider(label, x0, track_end, value_right, y + PART_LINE / 2.0, value, (&range, &range), decimals, false) {
                    edit.lower = Some(merge);
                }
            }
        }
        self.drawn(square.bottom());
        self.space(BLOCK_MARGIN);
        edit
    }

    fn side_caption(&self, x: f32, top: f32, text: &str) {
        let dim = self.tokens.text_dim;
        self.ui.painter().text(pos2(x, top + CAPTION_LINE / 2.0), Align2::LEFT_CENTER, text, FontId::proportional(SMALL_TEXT_SIZE), dim);
    }

    /// The ⇄ (across) or ⇅ (down) glyph before a part's track; returns where the track starts
    fn direction_glyph(&self, x0: f32, center_y: f32, part: usize) -> f32 {
        let glyph = if part == 0 { ic::ARROWS_LEFT_RIGHT } else { ic::ARROWS_DOWN_UP };
        let dim = self.tokens.text_dim;
        self.ui.painter().text(pos2(x0 + GLYPH_CENTER, center_y), Align2::CENTER_CENTER, glyph, FontId::proportional(ICON_SIZE), dim);
        x0 + GLYPH_TRACK
    }

    #[allow(clippy::too_many_arguments)]
    fn side_slider(
        &mut self,
        label: impl std::fmt::Display,
        start: f32,
        end: f32,
        value_right: f32,
        center_y: f32,
        value: &mut f32,
        (range, typed_range): (&std::ops::RangeInclusive<f32>, &std::ops::RangeInclusive<f32>),
        decimals: usize,
        signed: bool,
    ) -> Option<bool> {
        let id = self.next_id();
        let dragged = self.track(id, &label, &Track { start, end, center_y, zero: signed.then_some(0.0), ramp: false }, value, range);
        let typed = self.value_field_at(id, &label, value_right, SIDE_VALUE_WIDTH - SIDE_VALUE_AIR, center_y, *value, typed_range, decimals, "");
        Self::apply_typed(value, typed, dragged)
    }
}

/// What a part's name adds to its caption: nothing for the first, " 2" for the second
fn part_suffix(part: usize) -> &'static str {
    if part == 0 { "" } else { " 2" }
}
