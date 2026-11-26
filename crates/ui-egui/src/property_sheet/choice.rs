use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Margin, Painter, Rect, RectAlign, Response, Role, Sense, Shadow, Stroke, StrokeKind, WidgetInfo, pos2, vec2,
};
use egui_phosphor::regular as ic;

use super::{CONTROL_HEIGHT, CONTROL_RADIUS, ICON_SIZE, PropertySheet, RowLead, SMALL_TEXT_SIZE, TEXT_SIZE};
use crate::theme::{HARD_SHADOW, semibold};

/// Draws an option's picture into its square
pub(super) type ChoicePicture<'a, T> = &'a dyn Fn(&Painter, Rect, T);

/// Size of a picture in a dropdown button and in its list
const PICTURE_SIZE: f32 = 18.0;

/// From a picture dropdown's left edge to its picture
const PICTURE_INSET: f32 = 6.0;

/// From a plain dropdown's left edge to its value
pub(super) const VALUE_INSET: f32 = 8.0;

/// Air between a picture and the words after it
const PICTURE_TO_TEXT: f32 = 8.0;

/// From a dropdown's right edge to its caret's centre
pub(super) const CARET_INSET: f32 = 10.0;

/// Room a dropdown keeps for its caret: its value is cut with "…" before it
pub(super) const CARET_ROOM: f32 = 24.0;

/// The open list's border, drawn outside its options: the list's outer edge is its button's
pub(super) const LIST_BORDER: f32 = 1.0;

/// The open list's corner radius and the air above its first option and below its last
const LIST_RADIUS: u8 = 6;
const LIST_PADDING: i8 = 4;

/// The open list's hard shadow, offset down and right like the other popups'
const LIST_SHADOW: Shadow = Shadow { offset: [2, 3], blur: 0, spread: 0, color: HARD_SHADOW };

/// Height of one option in an open list
const OPTION_HEIGHT: f32 = 26.0;

/// Width of an option's keyboard focus outline in points
const OPTION_FOCUS_WIDTH: f32 = 2.0;

/// How far an option's words sit above its centre, to look centred beside their capitals
const OPTION_TEXT_LIFT: f32 = 1.0;

/// Gap under an option's hover fill, so neighbouring fills never touch
const HOVER_BOTTOM_GAP: f32 = 2.0;

/// How far an option's hover fill sits inside the list's edges
const HOVER_INSET: f32 = 3.0;

/// From the list's right edge to the hovered option's preview eye
const EYE_INSET: f32 = 16.0;

/// Lists narrower than this have no room for the preview eye
const EYE_MIN_LIST_WIDTH: f32 = 80.0;

/// The lettering of a set-once choice, cut with "…" to fit `room`
pub(super) fn truncated(painter: &Painter, text: &str, room: f32, color: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), semibold(TEXT_SIZE), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(0.0));
    painter.layout_job(job)
}

/// The chosen option's accent bar, inset from the option's left, top and bottom
const CHOSEN_BAR_LEFT: f32 = 2.0;
const CHOSEN_BAR_RIGHT: f32 = 4.0;
const CHOSEN_BAR_TOP: f32 = 5.0;
const CHOSEN_BAR_BOTTOM: f32 = 7.0;

/// The chosen option's accent bar is 2 px wide: a 1 px radius rounds its ends fully
const CHOSEN_BAR_RADIUS: u8 = 1;

/// The hovered option's eye glyph size
const EYE_SIZE: f32 = 13.0;

impl PropertySheet<'_> {
    /// A set-once choice whose list hangs from its button; returns the picked and the hovered option
    pub(crate) fn dropdown<T: Copy + PartialEq>(
        &mut self,
        label: &str,
        current: T,
        options: &[T],
        name: impl Fn(T) -> String,
        picture: Option<ChoicePicture<'_, T>>,
    ) -> (Option<T>, Option<T>) {
        self.dropdown_led(RowLead::label(label), current, options, name, picture)
    }

    /// [`Self::dropdown`] led by `lead` (a label stacked above it, when its values differ only at their end)
    pub(crate) fn dropdown_led<T: Copy + PartialEq>(
        &mut self,
        lead: RowLead<'_>,
        current: T,
        options: &[T],
        name: impl Fn(T) -> String,
        picture: Option<ChoicePicture<'_, T>>,
    ) -> (Option<T>, Option<T>) {
        let response = self.dropdown_row(lead, current, &name, picture);
        self.hanging_list(&response, current, options, &name, picture)
    }

    /// [`Self::dropdown`] whose `options` are built only while its list is open
    pub(crate) fn dropdown_built<T: Copy + PartialEq, Options: IntoIterator<Item = T>>(
        &mut self,
        label: &str,
        current: T,
        options: impl FnOnce() -> Options,
        name: impl Fn(T) -> String,
        picture: Option<ChoicePicture<'_, T>>,
    ) -> (Option<T>, Option<T>) {
        let response = self.dropdown_row(RowLead::label(label), current, &name, picture);
        self.hanging_groups(&response, current, || [(None, options())], &name, picture)
    }

    /// A dropdown's row led by `lead`, its button showing `current` and described by the row's name; returns the button's response
    fn dropdown_row<T: Copy>(&mut self, lead: RowLead<'_>, current: T, name: &impl Fn(T) -> String, picture: Option<ChoicePicture<'_, T>>) -> Response {
        let x0 = self.lead_with(lead);
        let row = self.take(super::ROW_HEIGHT);
        let center_y = row.center().y;
        self.drawn(center_y + CONTROL_HEIGHT / 2.0);
        let button = self.control_box(x0, center_y);
        let response = self.dropdown_button(button, &name(current), picture.map(|draw| move |painter: &Painter, rect: Rect| draw(painter, rect, current)));
        self.ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(lead.text()));
        response
    }

    /// [`Self::dropdown`] whose list holds its options in groups, each under its caption, built only while the list is open; returns the picked option
    pub(crate) fn grouped_dropdown<'g, T: Copy + PartialEq, Groups: IntoIterator<Item = (&'g str, Vec<T>)>>(
        &mut self,
        label: &str,
        current: T,
        groups: impl FnOnce() -> Groups,
        name: impl Fn(T) -> String,
        picture: ChoicePicture<'_, T>,
    ) -> Option<T> {
        let response = self.dropdown_row(RowLead::label(label), current, &name, Some(picture));
        let captioned = || groups().into_iter().map(|(caption, options)| (Some(caption), options));
        self.hanging_groups(&response, current, captioned, &name, Some(picture)).0
    }

    /// Paints a dropdown's button in `rect` and returns its response
    pub(super) fn dropdown_button(&mut self, rect: Rect, value: &str, picture: Option<impl Fn(&Painter, Rect)>) -> Response {
        self.dropdown_button_led(rect, value, |painter| match picture {
            Some(draw) => {
                let picture_rect =
                    Rect::from_min_size(pos2(rect.left() + PICTURE_INSET, rect.center().y - PICTURE_SIZE / 2.0), vec2(PICTURE_SIZE, PICTURE_SIZE));
                draw(painter, picture_rect);
                picture_rect.right() + PICTURE_TO_TEXT
            }
            None => rect.left() + VALUE_INSET,
        })
    }

    /// [`Self::dropdown_button`] whose left end `lead` paints on the button's face, returning where the value starts
    pub(super) fn dropdown_button_led(&mut self, rect: Rect, value: &str, lead: impl FnOnce(&Painter) -> f32) -> Response {
        let id = self.next_id();
        let response = self.ui.interact(rect, id, Sense::click());
        let enabled = self.ui.is_enabled();
        response.widget_info(|| WidgetInfo::labeled(Role::ComboBox, enabled, value));
        let tokens = self.tokens;
        let painter = self.ui.painter();
        painter.rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), if response.hovered() { tokens.row_hover } else { tokens.button });

        let text_x = lead(painter);
        let galley = truncated(painter, value, rect.right() - CARET_ROOM - text_x, tokens.text);
        painter.galley(pos2(text_x, rect.center().y - galley.size().y / 2.0), galley, tokens.text);
        painter.text(pos2(rect.right() - CARET_INSET, rect.center().y), Align2::CENTER_CENTER, ic::CARET_DOWN, FontId::proportional(ICON_SIZE), tokens.text);
        self.paint_control_focus(&response);
        response
    }

    /// The open list of `button`: 1 px below it, its width, over the rows below; returns the picked and the hovered option
    pub(crate) fn hanging_list<T: Copy + PartialEq>(
        &mut self,
        button: &Response,
        current: T,
        options: &[T],
        name: &impl Fn(T) -> String,
        picture: Option<ChoicePicture<'_, T>>,
    ) -> (Option<T>, Option<T>) {
        self.hanging_groups(button, current, || [(None, options.iter().copied())], name, picture)
    }

    /// [`Self::hanging_list`] of options in groups, each under its caption (a dim line that can't be picked) when it has one; `groups` runs only while the list is open
    fn hanging_groups<'g, T: Copy + PartialEq, Groups: IntoIterator<Item = (Option<&'g str>, Options)>, Options: IntoIterator<Item = T>>(
        &mut self,
        button: &Response,
        current: T,
        groups: impl FnOnce() -> Groups,
        name: &impl Fn(T) -> String,
        picture: Option<ChoicePicture<'_, T>>,
    ) -> (Option<T>, Option<T>) {
        let tokens = self.tokens;
        let width = button.rect.width() - 2.0 * LIST_BORDER;
        let frame = Frame::NONE
            .fill(tokens.chrome)
            .stroke(Stroke::new(LIST_BORDER, tokens.field_border))
            .corner_radius(CornerRadius::same(LIST_RADIUS))
            .inner_margin(Margin::symmetric(0, LIST_PADDING))
            .shadow(LIST_SHADOW);
        let shown = egui::Popup::from_toggle_button_response(button).align(RectAlign::BOTTOM_START).gap(1.0).width(width).frame(frame).show(|ui| {
            let mut picked = None;
            let mut hovered = None;
            for (caption, options) in groups() {
                if let Some(caption) = caption {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, OPTION_HEIGHT), Sense::hover());
                    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, caption));
                    let center_y = rect.top() + OPTION_HEIGHT / 2.0 - OPTION_TEXT_LIFT;
                    ui.painter().text(
                        pos2(rect.left() - LIST_BORDER + VALUE_INSET, center_y),
                        Align2::LEFT_CENTER,
                        caption,
                        FontId::proportional(SMALL_TEXT_SIZE),
                        tokens.text_dim,
                    );
                }
                for option in options {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, OPTION_HEIGHT), Sense::click());
                    let painter = ui.painter();
                    let chosen = option == current;
                    response.widget_info(|| WidgetInfo::selected(Role::ListBoxOption, true, chosen, name(option)));
                    if response.hovered() {
                        hovered = Some(option);
                        let fill =
                            Rect::from_min_max(pos2(rect.left() + HOVER_INSET, rect.top()), pos2(rect.right() - HOVER_INSET, rect.bottom() - HOVER_BOTTOM_GAP));
                        painter.rect_filled(fill, CornerRadius::same(CONTROL_RADIUS), tokens.button);
                    }
                    if chosen {
                        let bar = Rect::from_min_max(
                            pos2(rect.left() + CHOSEN_BAR_LEFT, rect.top() + CHOSEN_BAR_TOP),
                            pos2(rect.left() + CHOSEN_BAR_RIGHT, rect.bottom() - CHOSEN_BAR_BOTTOM),
                        );
                        painter.rect_filled(bar, CornerRadius::same(CHOSEN_BAR_RADIUS), tokens.accent);
                    }
                    if response.has_focus() {
                        painter.rect_stroke(rect, CornerRadius::same(CONTROL_RADIUS), Stroke::new(OPTION_FOCUS_WIDTH, tokens.text_strong), StrokeKind::Inside);
                    }

                    // Align the option text and picture with the button despite the list border
                    let button_left = rect.left() - LIST_BORDER;
                    let center_y = rect.top() + OPTION_HEIGHT / 2.0 - OPTION_TEXT_LIFT;
                    let mut text_x = button_left + VALUE_INSET;
                    if let Some(draw) = picture {
                        draw(
                            painter,
                            Rect::from_min_size(pos2(button_left + PICTURE_INSET, center_y - PICTURE_SIZE / 2.0), vec2(PICTURE_SIZE, PICTURE_SIZE)),
                            option,
                        );
                        text_x = button_left + PICTURE_INSET + PICTURE_SIZE + PICTURE_TO_TEXT;
                    }
                    let font = if chosen { semibold(TEXT_SIZE) } else { FontId::proportional(TEXT_SIZE) };
                    painter.text(pos2(text_x, center_y), Align2::LEFT_CENTER, name(option), font, tokens.text);
                    if response.hovered() && width >= EYE_MIN_LIST_WIDTH {
                        painter.text(pos2(rect.right() - EYE_INSET, center_y), Align2::CENTER_CENTER, ic::EYE, FontId::proportional(EYE_SIZE), tokens.accent);
                    }
                    if response.clicked() {
                        if !chosen {
                            picked = Some(option);
                        }

                        button.request_focus();
                        ui.close();
                    }
                }
            }
            (picked, hovered)
        });
        shown.map(|inner| inner.inner).unwrap_or((None, None))
    }
}
