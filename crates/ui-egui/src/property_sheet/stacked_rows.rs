//! Rows with their label above a full-width control, as the bake panels lay them out: folding sections, bold sub-headers, checks, and controls that grey out with a reason

use std::ops::RangeInclusive;

use egui::{Align2, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, UiBuilder, WidgetInfo, pos2, vec2};
use egui_phosphor::regular as ic;

use super::choice::ChoicePicture;
use super::number_field::shown_number;
use super::slider::KNOB_RADIUS;
use super::track::Track;
use super::value_text::ValueText;
use super::{CONTROL_HEIGHT, CONTROL_RADIUS, FoldSection, ICON_SIZE, PropertySheet, SMALL_TEXT_SIZE, TEXT_SIZE};
use crate::theme::semibold;

/// Height of a label's line above its control, and its text's centre in it
pub(super) const LABEL_LINE: f32 = 18.0;
const LABEL_CENTER: f32 = 9.0;

/// Air under a control before the next setting; full-width buttons and lists take it above and twice below, so two filled boxes never touch
pub(super) const STACK_ROW_AIR: f32 = 6.0;

/// A slider's track line under its label, and its centre in it
const TRACK_LINE: f32 = 16.0;
const TRACK_CENTER: f32 = 8.0;

/// A check's line, its box and the air from the box to its label
const CHECK_LINE: f32 = 20.0;
pub(crate) const CHECK_SIZE: f32 = 13.0;
const CHECK_RADIUS: u8 = 2;
const CHECK_TO_LABEL: f32 = 9.0;

/// The tick's three points across a check's box, as fractions of its side (from the reference's tick)
const TICK_POINTS: [(f32, f32); 3] = [(0.25, 0.52), (0.43, 0.7), (0.75, 0.3)];
const TICK_WIDTH: f32 = 1.7;

/// Half the length of a mixed check's dash, and its thickness
const DASH_HALF: f32 = 3.0;
const DASH_WIDTH: f32 = 2.0;

/// Air above a later section's hairline and from it to the caption
const FOLD_AIR_ABOVE: f32 = 8.0;
const FOLD_RULE_TO_CAPTION: f32 = 10.0;

/// Air above the first section's caption
const FOLD_FIRST_AIR: f32 = 10.0;

/// A section caption's line, the air under it, and where its chevron and caption sit
const FOLD_CAPTION_LINE: f32 = 20.0;
const FOLD_CAPTION_AIR: f32 = 10.0;
const FOLD_CHEVRON_CENTER: f32 = 5.0;
const FOLD_CAPTION_LEFT: f32 = 18.0;

/// The reset glyph's size and its centre's distance from the right edge
const FOLD_RESET_SIZE: f32 = 14.0;
const FOLD_RESET_INSET: f32 = 6.0;
const FOLD_RESET_BOX: f32 = 20.0;

/// Air above a later sub-header, its line, and the air under it
const SUB_HEADER_AIR: f32 = 6.0;
const SUB_HEADER_LINE: f32 = 18.0;
const SUB_HEADER_GAP: f32 = 4.0;

/// How far a sub-header's rows, and settings that belong to the one above, sit in from the left
const STACK_INDENT: f32 = 12.0;

/// Opacity of a greyed setting, on top of egui's disabled fade: 0.4 in all, the locked bake mock's 60 % blend toward the panel
const GREYED_EXTRA_OPACITY: f32 = 0.8;

/// A full-width button's glyph size and the air between it and its words
const WIDE_BUTTON_ICON_SIZE: f32 = 13.0;
const WIDE_BUTTON_ICON_GAP: f32 = 8.0;

/// Width of the link between two paired dropdowns, and the air on its sides
const PAIR_LINK_WIDTH: f32 = 26.0;
const PAIR_LINK_AIR: f32 = 4.0;

/// Values with more places than this drop their trailing zeros
const SHORT_DECIMALS: usize = 2;

/// Air before a field's text
const FIELD_TEXT_INSET: i8 = 8;

impl PropertySheet<'_> {
    /// A section that folds: chevron and CAPS caption left, its reset glyph right, a hairline above a later one; the click on the caption line folds it
    pub(crate) fn fold_section(&mut self, title: &str, default_open: bool) -> FoldSection {
        self.close_card();
        if self.empty {
            self.space(FOLD_FIRST_AIR);
        } else {
            self.space(FOLD_AIR_ABOVE);
            let y = self.cursor_y();
            self.ui.painter().line_segment([pos2(self.left, y), pos2(self.right, y)], Stroke::new(1.0, self.tokens.separator));
            self.space(FOLD_RULE_TO_CAPTION);
        }

        let section = self.id.with(("fold_section", title));
        self.scope = section;
        self.band_scope = section;
        let line = self.take(FOLD_CAPTION_LINE);
        let center_y = line.center().y;
        let reset_rect = Rect::from_center_size(pos2(self.right - FOLD_RESET_INSET, center_y), vec2(FOLD_RESET_BOX, FOLD_RESET_BOX));
        let header = Rect::from_min_max(pos2(self.left, line.top()), pos2(reset_rect.left(), line.bottom()));

        let mut open = self.ui.data(|data| data.get_temp::<bool>(section)).unwrap_or(default_open);
        let fold = self.ui.interact(header, section.with("fold"), Sense::click());
        if fold.clicked() {
            open = !open;
            self.ui.data_mut(|data| data.insert_temp(section, open));
        }
        let enabled = self.ui.is_enabled();
        fold.widget_info(|| WidgetInfo::selected(egui::Role::Button, enabled, open, title));
        self.ui.ctx().accesskit_node_builder(fold.id, |node| node.set_expanded(open));

        // Added after the caption's response, so the glyph takes the clicks over it
        let reset = self.ui.interact(reset_rect, section.with("reset"), Sense::click());
        let reset_label = crate::i18n::trf("Reset {section}", &[("section", &title)]);
        reset.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, &reset_label));
        let reset = self.tip(reset, &reset_label);

        let tokens = self.tokens;
        let painter = self.ui.painter();
        let chevron = if open { ic::CARET_DOWN } else { ic::CARET_RIGHT };
        painter.text(pos2(self.left + FOLD_CHEVRON_CENTER, center_y), Align2::CENTER_CENTER, chevron, FontId::proportional(ICON_SIZE), tokens.text);
        painter.text(pos2(self.left + FOLD_CAPTION_LEFT, center_y), Align2::LEFT_CENTER, title.to_uppercase(), semibold(SMALL_TEXT_SIZE), tokens.text);
        let reset_color = if reset.hovered() { tokens.text_strong } else { tokens.text_dim };
        painter.text(reset_rect.center(), Align2::CENTER_CENTER, ic::CLOCK_COUNTER_CLOCKWISE, FontId::proportional(FOLD_RESET_SIZE), reset_color);
        self.paint_control_focus(&fold);
        self.paint_control_focus(&reset);
        self.drawn(line.bottom());
        if open {
            self.space(FOLD_CAPTION_AIR);
        }
        self.sub_headers = 0;
        FoldSection { open, reset }
    }

    /// A bold sub-header; its rows follow one [`STACK_INDENT`] in, inside `body`
    pub(crate) fn sub_header<R>(&mut self, title: &str, body: impl FnOnce(&mut PropertySheet<'_>) -> R) -> R {
        if self.sub_headers > 0 {
            self.space(SUB_HEADER_AIR);
        }
        self.sub_headers += 1;
        let line = self.take(SUB_HEADER_LINE);
        let text = self.tokens.text_strong;
        self.ui.painter().text(pos2(self.left, line.top() + LABEL_CENTER), Align2::LEFT_CENTER, title, semibold(TEXT_SIZE), text);
        self.drawn(line.bottom());
        self.space(SUB_HEADER_GAP);
        self.indented(body)
    }

    /// `body`'s rows one [`STACK_INDENT`] in from the left
    pub(crate) fn indented<R>(&mut self, body: impl FnOnce(&mut PropertySheet<'_>) -> R) -> R {
        let left = self.left;
        self.left += STACK_INDENT;
        let result = body(self);
        self.left = left;
        result
    }

    /// `body`'s rows, greyed and inert unless `enabled`; hovering them shows `reason` (which also names them for assistive tech). Inside a greyed part they grey no further
    pub(crate) fn greyed<R>(&mut self, enabled: bool, reason: Option<&str>, body: impl FnOnce(&mut PropertySheet<'_>) -> R) -> R {
        let already_greyed = !self.ui.is_enabled();
        let top = self.cursor_y();
        let builder = UiBuilder::new().max_rect(self.ui.available_rect_before_wrap()).layout(*self.ui.layout());
        let mut child = self.ui.new_child(builder);
        if !enabled && !already_greyed {
            child.disable();
            child.multiply_opacity(GREYED_EXTRA_OPACITY);
        }

        let mut sheet = PropertySheet {
            tokens: self.tokens,
            id: self.id,
            scope: self.scope,
            band_scope: self.band_scope,
            named: std::mem::take(&mut self.named),
            left: self.left,
            right: self.right,
            used: self.used,
            seen: self.seen,
            card: self.card.take(),
            air_at: self.air_at,
            drawn_bottom: self.drawn_bottom,
            empty: self.empty,
            sections: std::mem::take(&mut self.sections),
            info: None,
            held: self.held,
            sub_headers: self.sub_headers,
            ui: &mut child,
        };
        let result = body(&mut sheet);
        let PropertySheet { named, seen, card, air_at, drawn_bottom, empty, sections, held, sub_headers, scope, band_scope, .. } = sheet;
        (self.named, self.seen, self.card, self.air_at, self.drawn_bottom, self.empty, self.sections) =
            (named, seen, card, air_at, drawn_bottom, empty, sections);
        (self.held, self.sub_headers, self.scope, self.band_scope) = (held, sub_headers, scope, band_scope);

        let bottom = child.min_rect().bottom().max(top);
        let region = Rect::from_min_max(pos2(self.left, top), pos2(self.right, bottom));
        self.ui.advance_cursor_after_rect(Rect::from_min_max(pos2(child.min_rect().left(), top), pos2(child.min_rect().right(), bottom)));
        if let (false, false, Some(reason)) = (enabled, already_greyed, reason) {
            let response = self.ui.interact(region, self.id.with(("greyed", top as i32)), Sense::hover());
            response.widget_info(|| WidgetInfo::labeled(egui::Role::Label, true, reason));

            // At the pointer: the region is tall, so a tip at its edge would land far from what was hovered
            let mut tooltip = egui::Tooltip::for_enabled(&response);
            tooltip.popup = tooltip.popup.anchor(egui::PopupAnchor::Pointer);
            tooltip.show(|ui| ui.label(reason));
        }
        result
    }

    /// A label on its own line above its control
    fn label_line(&mut self, label: &str) -> Rect {
        let line = self.take(LABEL_LINE);
        let text = self.tokens.text;
        self.ui.painter().text(pos2(self.left, line.top() + LABEL_CENTER), Align2::LEFT_CENTER, label, FontId::proportional(TEXT_SIZE), text);
        line
    }

    /// A full-width control's box under its label, and the air after it
    fn control_line(&mut self) -> Rect {
        let line = self.take(CONTROL_HEIGHT);
        self.drawn(line.bottom());
        let rect = Rect::from_min_max(pos2(self.left, line.top()), pos2(self.right, line.bottom()));
        self.space(STACK_ROW_AIR);
        rect
    }

    /// A dropdown under its label; returns the picked option
    pub(crate) fn dropdown_above<T: Copy + PartialEq>(&mut self, label: &str, current: T, options: &[T], name: impl Fn(T) -> String) -> Option<T> {
        self.label_line(label);
        let button = self.control_line();
        let response = self.dropdown_button(button, &name(current), None::<fn(&egui::Painter, Rect)>);
        named_choice(self.ui, &response, label, &name(current));
        self.hanging_list(&response, current, options, &name, None::<ChoicePicture<'_, T>>).0
    }

    /// Two dropdowns under one label with a link between them; while `linked` the second follows the first and shows greyed. Returns which one picked what
    pub(crate) fn dropdown_pair_above<T: Copy + PartialEq>(
        &mut self,
        label: &str,
        values: [T; 2],
        options: &[T],
        name: impl Fn(T) -> String,
        linked: &mut bool,
        link_label: &str,
    ) -> Option<(usize, T)> {
        self.label_line(label);
        let line = self.control_line();
        let half = (line.width() - PAIR_LINK_WIDTH - 2.0 * PAIR_LINK_AIR) / 2.0;
        let first = Rect::from_min_size(line.min, vec2(half, CONTROL_HEIGHT));
        let link = Rect::from_min_size(pos2(first.right() + PAIR_LINK_AIR, line.top()), vec2(PAIR_LINK_WIDTH, CONTROL_HEIGHT));
        let second = Rect::from_min_max(pos2(link.right() + PAIR_LINK_AIR, line.top()), line.max);

        let first_response = self.dropdown_button(first, &name(values[0]), None::<fn(&egui::Painter, Rect)>);
        named_choice(self.ui, &first_response, label, &name(values[0]));
        let mut picked = self.hanging_list(&first_response, values[0], options, &name, None::<ChoicePicture<'_, T>>).0.map(|value| (0, value));

        let link_id = self.next_id();
        let link_response = self.ui.interact(link, link_id, Sense::click());
        let enabled = self.ui.is_enabled();
        link_response.widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, enabled, *linked, link_label));
        let link_response = self.tip(link_response, link_label);
        if link_response.clicked() {
            *linked = !*linked;
        }
        let tokens = self.tokens;

        // Held down: the hairline grey, neutral in every accent (tool_active takes the accent's tint)
        let fill = if *linked {
            tokens.separator
        } else if link_response.hovered() {
            tokens.row_hover
        } else {
            tokens.button
        };
        let painter = self.ui.painter();
        painter.rect(link, CornerRadius::same(CONTROL_RADIUS), fill, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
        let glyph = if *linked { ic::LINK_SIMPLE_HORIZONTAL } else { ic::LINK_SIMPLE_HORIZONTAL_BREAK };
        painter.text(link.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(ICON_SIZE), tokens.text);
        self.paint_control_focus(&link_response);

        let follows = *linked;
        let second_value = if follows { values[0] } else { values[1] };
        let mut second_ui = self.ui.new_child(UiBuilder::new().max_rect(second));
        if follows {
            second_ui.disable();
        }
        let mut second_sheet = PropertySheet {
            tokens,
            id: self.id,
            scope: self.scope,
            band_scope: self.band_scope,
            named: Vec::new(),
            left: second.left(),
            right: second.right(),
            used: self.used,
            seen: self.seen,
            card: None,
            air_at: None,
            drawn_bottom: self.drawn_bottom,
            empty: false,
            sections: Vec::new(),
            info: None,
            held: false,
            sub_headers: 0,
            ui: &mut second_ui,
        };
        let second_response = second_sheet.dropdown_button(second, &name(second_value), None::<fn(&egui::Painter, Rect)>);
        named_choice(second_sheet.ui, &second_response, label, &name(second_value));
        if let Some(value) = second_sheet.hanging_list(&second_response, second_value, options, &name, None::<ChoicePicture<'_, T>>).0 {
            picked = Some((1, value));
        }
        picked
    }

    /// A slider under its label, its value at the label line's right end; `Some(merge)` when `value` changed, `merge` true for a drag's later steps
    pub(crate) fn slider_above(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>, decimals: usize) -> Option<bool> {
        let line = self.label_line(label);
        let id = self.named_id("slider_above", label);
        let shown = shown_number(*value, decimals);

        // Trim trailing zeros only on high-precision values
        let shown = if decimals > SHORT_DECIMALS && shown.contains('.') { shown.trim_end_matches('0').trim_end_matches('.').to_owned() } else { shown };
        let width = self.text_width(&shown, semibold(TEXT_SIZE)).max(super::VALUE_MIN_WIDTH);
        let value_text = ValueText { label: &label, right: self.right, width, center_y: line.top() + LABEL_CENTER, shown: &shown, suffix: "" };
        let typed = self.value_text_field(id, value_text, *value, &range, decimals);

        let track_line = self.take(TRACK_LINE);
        let center_y = track_line.top() + TRACK_CENTER;
        let track = Track { start: self.left + KNOB_RADIUS, end: self.right - KNOB_RADIUS, center_y, zero: (*range.start() < 0.0).then_some(0.0), ramp: false };
        let dragged = self.track(id, label, &track, value, &range);
        self.drawn(center_y + KNOB_RADIUS);
        self.space(STACK_ROW_AIR);
        Self::apply_typed(value, typed, dragged)
    }

    /// A check with its label after it; returns whether it was clicked (`on` already flipped)
    pub(crate) fn check(&mut self, label: &str, on: &mut bool) -> bool {
        let clicked = self.check_mixed(label, Some(*on)).clicked();
        if clicked {
            *on = !*on;
        }
        clicked
    }

    /// A check that may show mixed (`None`, a dash) for several states at once; the caller flips what it stands for on a click
    pub(crate) fn check_mixed(&mut self, label: &str, state: Option<bool>) -> Response {
        self.check_noted(label, state, None)
    }

    /// `check_mixed` with a dim `note` at the row's right end, such as the values a set would give up
    pub(crate) fn check_noted(&mut self, label: &str, state: Option<bool>, note: Option<&str>) -> Response {
        let line = self.take(CHECK_LINE);
        let center_y = line.center().y;
        let text = self.ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(TEXT_SIZE), self.tokens.text);
        let hit = Rect::from_min_max(pos2(self.left, line.top()), pos2(self.left + CHECK_SIZE + CHECK_TO_LABEL + text.size().x, line.bottom()));
        let id = self.next_id();
        let response = self.ui.interact(hit, id, Sense::click());
        let enabled = self.ui.is_enabled();
        response.widget_info(|| match state {
            Some(on) => WidgetInfo::selected(egui::Role::CheckBox, enabled, on, label),
            None => WidgetInfo::labeled(egui::Role::CheckBox, enabled, label),
        });

        let check = Rect::from_min_size(pos2(self.left, center_y - CHECK_SIZE / 2.0), vec2(CHECK_SIZE, CHECK_SIZE));
        paint_check(self.ui.painter(), &self.tokens, check, state, response.hovered());
        self.ui.painter().galley(pos2(check.right() + CHECK_TO_LABEL, center_y - text.size().y / 2.0), text, self.tokens.text);
        if let Some(note) = note {
            self.ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(note));
            self.ui.painter().text(pos2(self.right, center_y), Align2::RIGHT_CENTER, note, FontId::proportional(SMALL_TEXT_SIZE), self.tokens.text_dim);
        }
        self.paint_control_focus(&response);
        self.drawn(line.bottom());
        self.space(STACK_ROW_AIR);
        response
    }

    /// A one-line text field under its label; returns whether `text` changed
    pub(crate) fn text_field_above(&mut self, label: &str, text: &mut String) -> bool {
        self.label_line(label);
        let line = self.take(CONTROL_HEIGHT);
        let field = Rect::from_min_max(pos2(self.left, line.top()), pos2(self.right, line.bottom()));
        let id = self.named_id("text_field_above", label);
        let typing = self.ui.memory(|memory| memory.has_focus(id));
        let tokens = self.tokens;
        let outline = Stroke::new(1.0, if typing { tokens.accent } else { tokens.field_border });
        self.ui.painter().rect(field, CornerRadius::same(CONTROL_RADIUS), tokens.field, outline, StrokeKind::Inside);
        let edit = egui::TextEdit::singleline(text)
            .id(id)
            .font(FontId::proportional(TEXT_SIZE))
            .text_color(tokens.text)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(FIELD_TEXT_INSET, 0)))
            .align(Align2::LEFT_CENTER);
        let response = self.ui.put(field.shrink(1.0), edit);
        let enabled = self.ui.is_enabled();
        response.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, enabled, label));
        self.drawn(field.bottom());
        self.space(STACK_ROW_AIR);
        response.changed()
    }

    /// A full-width button with air above and twice below; `pressed` shows it held down (a toggle that is on)
    pub(crate) fn wide_button(&mut self, glyph: &str, text: &str, pressed: bool) -> Response {
        self.space(STACK_ROW_AIR);
        let line = self.take(CONTROL_HEIGHT);
        let button = Rect::from_min_max(pos2(self.left, line.top()), pos2(self.right, line.bottom()));
        let id = self.next_id();
        let response = self.ui.interact(button, id, Sense::click());
        let enabled = self.ui.is_enabled();
        response.widget_info(|| WidgetInfo::selected(egui::Role::Button, enabled, pressed, text));

        let tokens = self.tokens;

        // Held down: the hairline grey, as the paired dropdowns' link
        let fill = match (pressed, response.hovered()) {
            (true, _) => tokens.separator,
            (false, true) => tokens.row_hover,
            (false, false) => tokens.button,
        };
        let painter = self.ui.painter();
        painter.rect(button, CornerRadius::same(CONTROL_RADIUS), fill, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
        let icon = painter.layout_no_wrap(glyph.to_owned(), FontId::proportional(WIDE_BUTTON_ICON_SIZE), tokens.text);
        let words = painter.layout_no_wrap(text.to_owned(), FontId::proportional(TEXT_SIZE), tokens.text);
        let x = button.center().x - (icon.size().x + WIDE_BUTTON_ICON_GAP + words.size().x) / 2.0;
        let center_y = button.center().y;
        painter.galley(pos2(x, center_y - icon.size().y / 2.0), icon.clone(), tokens.text);
        painter.galley(pos2(x + icon.size().x + WIDE_BUTTON_ICON_GAP, center_y - words.size().y / 2.0), words, tokens.text);
        self.paint_control_focus(&response);
        self.drawn(button.bottom());
        self.space(2.0 * STACK_ROW_AIR);
        response
    }

    /// A dim line of its own, such as a map's "No Parameters"
    pub(crate) fn dim_line(&mut self, text: &str) {
        let line = self.take(LABEL_LINE);
        let dim = self.tokens.text_dim;
        let id = self.next_id();
        let response = self.ui.interact(line, id, Sense::hover());
        let enabled = self.ui.is_enabled();
        response.widget_info(|| WidgetInfo::labeled(egui::Role::Label, enabled, text));
        self.ui.painter().text(pos2(self.left, line.top() + LABEL_CENTER), Align2::LEFT_CENTER, text, FontId::proportional(TEXT_SIZE), dim);
        self.drawn(line.bottom());
    }
}

/// A check's box in `rect`: on = dim fill and a tick in the panel's colour, mixed (`None`) = a dash, off = the row colour with a field's border
pub(crate) fn paint_check(painter: &egui::Painter, tokens: &crate::theme::Tokens, rect: Rect, state: Option<bool>, hovered: bool) {
    if state == Some(false) {
        let border = if hovered { tokens.text } else { tokens.text_dim };
        painter.rect(rect, CornerRadius::same(CHECK_RADIUS), tokens.row, Stroke::new(1.0, border), StrokeKind::Inside);
        return;
    }

    let fill = if hovered { tokens.text } else { tokens.text_dim };
    painter.rect_filled(rect, CornerRadius::same(CHECK_RADIUS), fill);
    if state.is_none() {
        let y = rect.center().y;
        painter.line_segment([pos2(rect.center().x - DASH_HALF, y), pos2(rect.center().x + DASH_HALF, y)], Stroke::new(DASH_WIDTH, tokens.panel));
        return;
    }

    let points = TICK_POINTS.map(|(x, y)| pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()));
    painter.line(points.to_vec(), Stroke::new(TICK_WIDTH, tokens.panel));
}

/// Names a dropdown for assistive tech by its setting (`label`), its value the choice it shows
fn named_choice(ui: &egui::Ui, response: &Response, label: &str, value: &str) {
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label(label);
        node.set_value(value);
    });
}
