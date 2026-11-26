use egui::{Align2, Color32, CornerRadius, FontId, Painter, Rect, Response, Role, Sense, Stroke, StrokeKind, WidgetInfo, pos2, vec2};

use super::{CONTROL_RADIUS, PropertySheet, ROW_HEIGHT, ROW_PICTURE_AIR, ROW_PICTURE_SIZE, RowLead, SMALL_TEXT_SIZE, TEXT_SIZE};
use crate::theme::semibold;

/// Draws a picture row's picture into its square
type RowPicture<'a> = &'a dyn Fn(&Painter, Rect);

/// Draws option `index`'s picture into its square
type SegmentPicture<'a> = &'a dyn Fn(&Painter, Rect, usize);

/// Side of a picture beside a switch's word, and the air between them
const SEGMENT_PICTURE_SIZE: f32 = 14.0;
const SEGMENT_PICTURE_AIR: f32 = 6.0;

/// Room a switch's word needs in its segment beyond its own width (6 px each side: Bevel Smooth's Out | In | In/Out fits beside the widest label column)
const SEGMENT_AIR: f32 = 12.0;

/// How far the chosen segment sits inside the switch's box
const SEGMENT_INSET: f32 = 2.0;

/// Corner radius of a switch's box
const SWITCH_RADIUS: u8 = 6;

/// Size of a toggle's pill
pub(super) const PILL_SIZE: egui::Vec2 = vec2(22.0, 13.0);

/// Corner radius of a toggle's pill: about half its height, so its ends are round
const PILL_RADIUS: u8 = 6;

/// Radius of a toggle's knob, and how far it sits from the pill's middle
const TOGGLE_KNOB_RADIUS: f32 = 4.0;
const TOGGLE_KNOB_TRAVEL: f32 = 4.5;

/// Air after a toggle's label, before the column the panel's toggles line up on
const TOGGLE_LABEL_AIR: f32 = 10.0;

/// From that column to the pill's centre: the pill starts on the column
const TOGGLE_CENTER_OFFSET: f32 = PILL_SIZE.x / 2.0;

/// From one of a pair's toggles to the next, and from its name to its pill's centre
const PAIR_PITCH: f32 = 76.0;
const PAIR_NAME_TO_PILL: f32 = 32.0;

/// Height of a channel pill, and the air between pills
const CHANNEL_PILL_HEIGHT: f32 = 22.0;
const CHANNEL_PILL_GAP: f32 = 4.0;

/// Corner radius of a channel pill: half its height, so its ends are round
const CHANNEL_PILL_RADIUS: u8 = 11;

/// Air under the channel strip
const CHANNEL_STRIP_AIR: f32 = 6.0;

/// The least air around a channel pill's name before the names shrink a point
const PILL_TEXT_AIR: f32 = 10.0;

/// Width of a focused control's outline in points
const FOCUS_OUTLINE_WIDTH: f32 = 2.0;

/// No tint for any segment: the chosen word keeps the text colour
pub(super) fn untinted(_index: usize) -> &'static [Color32] {
    &[]
}

pub(super) fn paint_focus_outline(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_stroke(rect, CornerRadius::same(CONTROL_RADIUS), Stroke::new(FOCUS_OUTLINE_WIDTH, color), StrokeKind::Inside);
}

impl PropertySheet<'_> {
    pub(super) fn paint_control_focus(&self, response: &Response) {
        if response.has_focus() {
            paint_focus_outline(self.ui.painter(), response.rect, self.tokens.text_strong);
        }
    }

    /// Two to four short choices as a boxed switch, or a dropdown when one doesn't fit; returns the picked index
    pub(crate) fn switch(&mut self, label: &str, options: &[&str], selected: usize) -> Option<usize> {
        self.switch_tinted(label, options, selected, &[])
    }

    /// [`Self::switch`] whose chosen word takes its colour from `tints` (axis colours)
    pub(crate) fn switch_tinted(&mut self, label: &str, options: &[&str], selected: usize, tints: &[Color32]) -> Option<usize> {
        let stacks = self.text_width(label, FontId::proportional(TEXT_SIZE)) > self.column() - self.left - super::LABEL_MIN_AIR;
        let left = if stacks { self.left } else { self.column() };
        if !self.switch_fits(options, left) {
            return self.switch_dropdown(label, options, selected);
        }

        let (control, _) = self.row(label);
        let runs = |index: usize| tints.get(index).map(std::slice::from_ref).unwrap_or_default();
        self.segments(control, options, selected, runs, None)
    }

    /// [`Self::switch`] with its label always above it, the switch across the whole panel (long choices in a set of such switches); returns the picked index
    pub(crate) fn stacked_switch(&mut self, label: &str, options: &[&str], selected: usize) -> Option<usize> {
        if !self.switch_fits(options, self.left) {
            return self.switch_dropdown(label, options, selected);
        }

        let (control, _) = self.row_with(RowLead::Label { text: label, stack: true });
        self.segments(control, options, selected, untinted, None)
    }

    /// Whether every option's word fits its share of a switch from `left` to the panel's right edge
    fn switch_fits(&self, options: &[&str], left: f32) -> bool {
        let segment = (self.right - left) / options.len().max(1) as f32;
        options.iter().all(|option| self.text_width(option, semibold(SMALL_TEXT_SIZE)) + SEGMENT_AIR <= segment)
    }

    /// The dropdown a switch whose words don't fit becomes; returns the picked index
    fn switch_dropdown(&mut self, label: &str, options: &[&str], selected: usize) -> Option<usize> {
        let name = |index: usize| options.get(index).copied().unwrap_or_default().to_owned();
        self.dropdown_built(label, selected, || 0..options.len(), name, None).0
    }

    /// A boxed switch across the whole panel; the chosen word takes its colours from `tints` (several colours split the word between them)
    pub(crate) fn wide_switch(&mut self, options: &[&str], selected: usize, tints: &[&[Color32]]) -> Option<usize> {
        let control = self.wide_row();
        self.segments(control, options, selected, |index| tints.get(index).copied().unwrap_or_default(), None)
    }

    /// A row's control box, `CONTROL_HEIGHT` tall, from the left padding to the right one
    pub(super) fn wide_row(&mut self) -> Rect {
        let row = self.take(ROW_HEIGHT);
        let center_y = row.center().y;
        self.drawn(center_y + super::CONTROL_HEIGHT / 2.0);
        Rect::from_x_y_ranges(self.left..=self.right, (center_y - super::CONTROL_HEIGHT / 2.0)..=(center_y + super::CONTROL_HEIGHT / 2.0))
    }

    /// A boxed switch across the whole panel (a mode switch at the top of a group), each word led by its picture; returns the picked index
    pub(crate) fn picture_switch(&mut self, options: &[&str], selected: usize, picture: SegmentPicture<'_>) -> Option<usize> {
        let control = self.wide_row();
        self.segments(control, options, selected, untinted, Some(picture))
    }

    /// A boxed switch's box and segments in `control`, the chosen word in the colours `tints` gives its index (none: the text colour); returns the segment clicked when it was not the chosen one
    pub(super) fn segments<'t>(
        &mut self,
        control: Rect,
        options: &[&str],
        selected: usize,
        tints: impl Fn(usize) -> &'t [Color32],
        picture: Option<SegmentPicture<'_>>,
    ) -> Option<usize> {
        let segment = control.width() / options.len().max(1) as f32;
        let tokens = self.tokens;
        self.ui.painter().rect(control, CornerRadius::same(SWITCH_RADIUS), tokens.field, Stroke::new(1.0, tokens.field_border), StrokeKind::Inside);
        let mut picked = None;
        for (index, option) in options.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(control.left() + index as f32 * segment, control.top()), vec2(segment, control.height()));
            let id = self.next_id();
            let response = self.ui.interact(rect, id, Sense::click());
            let on = index == selected;
            response.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, on, *option));
            if response.clicked() && !on {
                picked = Some(index);
            }

            let painter = self.ui.painter();
            if on {
                painter.rect_filled(rect.shrink(SEGMENT_INSET), CornerRadius::same(CONTROL_RADIUS), tokens.button);
            } else if response.hovered() {
                painter.rect_filled(rect.shrink(SEGMENT_INSET), CornerRadius::same(CONTROL_RADIUS), tokens.row_hover);
            }

            self.paint_control_focus(&response);
            let runs = tints(index);
            let tint = runs.first().copied().unwrap_or(tokens.text);
            let (font, color) = if on { (semibold(SMALL_TEXT_SIZE), tint) } else { (FontId::proportional(SMALL_TEXT_SIZE), tokens.text_dim) };
            let Some(draw) = picture else {
                let word =
                    if on && runs.len() > 1 { split_word(painter, option, runs, &font) } else { painter.layout_no_wrap((*option).to_owned(), font, color) };
                painter.galley(rect.center() - word.size() / 2.0, word, color);
                continue;
            };

            // The picture and the word centred together in the segment
            let word = painter.layout_no_wrap((*option).to_owned(), font, color);
            let left = rect.center().x - (SEGMENT_PICTURE_SIZE + SEGMENT_PICTURE_AIR + word.size().x) / 2.0;
            draw(
                painter,
                Rect::from_min_size(pos2(left, rect.center().y - SEGMENT_PICTURE_SIZE / 2.0), vec2(SEGMENT_PICTURE_SIZE, SEGMENT_PICTURE_SIZE)),
                index,
            );
            let word_left = left + SEGMENT_PICTURE_SIZE + SEGMENT_PICTURE_AIR;
            painter.galley(pos2(word_left, rect.center().y - word.size().y / 2.0), word, color);
        }
        picked
    }

    /// An on/off beside its label; every toggle in a panel lines up past the longest toggle label
    pub(crate) fn toggle(&mut self, label: &str, on: &mut bool) -> bool {
        let rect = self.take(ROW_HEIGHT);
        let center_y = rect.center().y;
        let label_x = self.left + self.lead_shift();
        self.paint_info(center_y);
        let font = FontId::proportional(TEXT_SIZE);
        let label_end = label_x - self.left + self.text_width(label, font.clone()) + TOGGLE_LABEL_AIR;
        self.seen.toggle_label_end = self.seen.toggle_label_end.max(label_end);

        let column = (self.column() - self.left).max(self.used.toggle_label_end).max(label_end);
        let pill = Rect::from_center_size(pos2(self.left + column + TOGGLE_CENTER_OFFSET, center_y), PILL_SIZE);
        let clicked = self.toggle_hit(Rect::from_min_max(pos2(label_x, rect.top()), pos2(pill.right(), rect.bottom())), label, on);

        let text = self.tokens.text;
        self.ui.painter().text(pos2(label_x, center_y), Align2::LEFT_CENTER, label, font, text);
        self.pill(pill.center(), *on, true);
        self.drawn(center_y + PILL_SIZE.y / 2.0);
        clicked
    }

    /// A culling row on the panel's grid: its name on the label column, its picture where the controls start with the switch after it, a pending i at the row's right end
    pub(crate) fn picture_toggle(&mut self, picture: RowPicture<'_>, label: &str, on: &mut bool) -> bool {
        let rect = self.take(ROW_HEIGHT);
        let center_y = rect.center().y;
        self.paint_info_at_end(center_y);
        let font = FontId::proportional(TEXT_SIZE);

        // A name too long for the label column pushes every picture row's picture right instead of running under it
        let label_end = self.text_width(label, font.clone()) + super::LABEL_MIN_AIR;
        self.seen.picture_label_end = self.seen.picture_label_end.max(label_end);
        let picture_x = self.left + (self.column() - self.left).max(self.used.picture_label_end).max(label_end);
        let square = Rect::from_min_size(pos2(picture_x, center_y - ROW_PICTURE_SIZE / 2.0), vec2(ROW_PICTURE_SIZE, ROW_PICTURE_SIZE));
        let pill = Rect::from_min_size(pos2(square.right() + ROW_PICTURE_AIR, center_y - PILL_SIZE.y / 2.0), PILL_SIZE);
        let clicked = self.toggle_hit(Rect::from_min_max(pos2(self.left, rect.top()), pos2(pill.right(), rect.bottom())), label, on);

        let text = self.tokens.text;
        picture(self.ui.painter(), square);
        self.ui.painter().text(pos2(self.left, center_y), Align2::LEFT_CENTER, label, font, text);
        self.pill(pill.center(), *on, true);
        self.drawn(square.bottom());
        clicked
    }

    /// A toggle's clickable area: a click flips `on`; returns whether it was clicked
    pub(super) fn toggle_hit(&mut self, hit: Rect, label: &str, on: &mut bool) -> bool {
        let id = self.next_id();
        let response = self.ui.interact(hit, id, Sense::click());
        if response.clicked() {
            *on = !*on;
        }

        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, *on, label));
        self.paint_control_focus(&response);
        response.clicked()
    }

    /// Two named toggles side by side after one label (Flip copy: U, V); returns the index clicked
    pub(crate) fn toggle_pair(&mut self, label: &str, names: [&str; 2], on: &mut [bool; 2]) -> Option<usize> {
        let (control, center_y) = self.row(label);
        let font = FontId::proportional(TEXT_SIZE);
        let mut clicked = None;
        for (index, (name, state)) in names.into_iter().zip(on.iter_mut()).enumerate() {
            let x = control.left() + index as f32 * PAIR_PITCH;
            let pill = pos2(x + PAIR_NAME_TO_PILL, center_y);
            let hit = Rect::from_min_max(pos2(x, control.top()), pos2(pill.x + PILL_SIZE.x / 2.0, control.bottom()));
            let id = self.next_id();
            let response = self.ui.interact(hit, id, Sense::click());
            if response.clicked() {
                *state = !*state;
                clicked = Some(index);
            }

            response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, *state, format!("{label} {name}")));
            let tokens = self.tokens;
            self.ui.painter().text(pos2(x, center_y), Align2::LEFT_CENTER, name, font.clone(), if *state { tokens.text } else { tokens.text_dim });
            self.pill(pill, *state, true);
            self.paint_control_focus(&response);
        }
        clicked
    }

    /// A toggle's pill centred on `center`: on (accent), off, or greyed out when not `enabled`
    pub(super) fn pill(&self, center: egui::Pos2, on: bool, enabled: bool) {
        let tokens = self.tokens;
        let (fill, border, knob, knob_x) = match (enabled, on) {
            (false, _) => (tokens.field, tokens.separator, tokens.separator, center.x - TOGGLE_KNOB_TRAVEL),
            (true, true) => (tokens.accent_dim, tokens.accent, tokens.text, center.x + TOGGLE_KNOB_TRAVEL),
            (true, false) => (tokens.button, tokens.button_border, tokens.text_dim, center.x - TOGGLE_KNOB_TRAVEL),
        };
        let painter = self.ui.painter();
        painter.rect(Rect::from_center_size(center, PILL_SIZE), CornerRadius::same(PILL_RADIUS), fill, Stroke::new(1.0, border), StrokeKind::Inside);
        painter.circle_filled(pos2(knob_x, center.y), TOGGLE_KNOB_RADIUS, knob);
    }

    /// The channel strip: one pill per channel; returns the clicked index and whether Alt (solo) was held
    pub(crate) fn channel_pills(&mut self, names: &[&str], on: &[bool]) -> Option<(usize, bool)> {
        let rect = self.take(CHANNEL_PILL_HEIGHT);
        let count = names.len().max(1) as f32;
        let width = (self.right - self.left - (count - 1.0) * CHANNEL_PILL_GAP) / count;
        let size = if names.iter().all(|name| self.text_width(name, semibold(SMALL_TEXT_SIZE)) + PILL_TEXT_AIR <= width) {
            SMALL_TEXT_SIZE
        } else {
            SMALL_TEXT_SIZE - 1.0
        };

        let tokens = self.tokens;
        let mut clicked = None;
        for (index, name) in names.iter().enumerate() {
            let pill = Rect::from_min_size(pos2(self.left + index as f32 * (width + CHANNEL_PILL_GAP), rect.top()), vec2(width, CHANNEL_PILL_HEIGHT));
            let id = self.next_id();
            let response = self.ui.interact(pill, id, Sense::click());
            if response.clicked() {
                clicked = Some((index, self.ui.input(|input| input.modifiers.alt)));
            }

            let enabled = on.get(index).copied().unwrap_or(false);
            response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, enabled, *name));
            let (fill, border, color, font) = if enabled {
                (tokens.accent_dim, tokens.accent, tokens.text, semibold(size))
            } else {
                (tokens.field, tokens.field_border, tokens.text_dim, FontId::proportional(size))
            };
            let painter = self.ui.painter();
            painter.rect(
                pill,
                CornerRadius::same(CHANNEL_PILL_RADIUS),
                if response.hovered() && !enabled { tokens.row_hover } else { fill },
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
            painter.text(pill.center(), Align2::CENTER_CENTER, *name, font, color);
            self.paint_control_focus(&response);
        }
        self.drawn(rect.bottom());
        self.space(CHANNEL_STRIP_AIR);
        self.air_at = Some(self.cursor_y());
        clicked
    }
}

/// `word` laid out with its letters split as evenly as they go between the colours of `runs`
fn split_word(painter: &Painter, word: &str, runs: &[Color32], font: &FontId) -> std::sync::Arc<egui::Galley> {
    let letters: Vec<char> = word.chars().collect();
    let mut job = egui::text::LayoutJob::default();
    let parts = runs.len().max(1);
    for (part, color) in runs.iter().enumerate() {
        let (start, end) = (letters.len() * part / parts, letters.len() * (part + 1) / parts);
        let text: String = letters.get(start..end).unwrap_or_default().iter().collect();
        job.append(&text, 0.0, egui::TextFormat::simple(font.clone(), *color));
    }
    painter.layout_job(job)
}
