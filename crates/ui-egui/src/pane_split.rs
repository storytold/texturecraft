//! Two panes split by a focusable divider: the geometry of one split and its resize handle (pointer drag, arrow keys
//! and accessibility). The shelf's saved searches and path filter (`shelf_panes.rs`) use it.

use std::ops::RangeInclusive;

use craft_behavior::dock::SplitSize;
use craft_behavior::flow::Axis;
use egui::{Color32, EventFilter, Id, Key, Modifiers, Rect, Response, Sense, Ui, Vec2, accesskit};

/// Contained, non-overlapping rectangles of a single split, including its divider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SplitLayout {
    pub(crate) first: Rect,
    pub(crate) divider: Rect,
    pub(crate) second: Rect,
    first_size: f32,
    available: f32,
}

impl SplitLayout {
    /// The first pane's size along the split axis, in points.
    pub(crate) fn first_size(self) -> f32 {
        self.first_size
    }

    /// Space for both panes, excluding the divider, in points.
    pub(crate) fn available(self) -> f32 {
        self.available
    }

    /// The size after moving the divider by `delta` points, in the same sizing mode. When both minima cannot fit,
    /// each pane gets half the available space. Invalid inputs and zero available space give `None`, so the caller
    /// keeps its saved value.
    pub(crate) fn resized(self, size: SplitSize, delta: f32, minimum: f32) -> Option<SplitSize> {
        if !value(size).is_finite() || !delta.is_finite() || !valid_nonnegative(minimum) || self.available <= 0.0 {
            return None;
        }
        let available = f64::from(self.available);
        let first = clamp_first(f64::from(self.first_size) + f64::from(delta), available, minimum);
        Some(match size {
            SplitSize::FixedFirst(_) => SplitSize::FixedFirst(first as f32),
            SplitSize::FixedSecond(_) => SplitSize::FixedSecond((available - first) as f32),
            _ => SplitSize::Ratio((first / available) as f32),
        })
    }
}

fn value(size: SplitSize) -> f32 {
    match size {
        SplitSize::Ratio(v) | SplitSize::FixedFirst(v) | SplitSize::FixedSecond(v) => v,
        #[allow(unreachable_patterns)]
        _ => f32::NAN,
    }
}

/// Divides `rect` into two panes and a divider `gap` points thick. Equal pane minima relax to half the remaining
/// span in tiny areas; finite sizes outside the available range are clamped. Inverted or non-finite rectangles, a
/// negative or non-finite gap or minimum and a non-finite size give `None`.
pub(crate) fn split_rect(rect: Rect, axis: Axis, size: SplitSize, gap: f32, minimum: f32) -> Option<SplitLayout> {
    if !valid_rect(rect) || !valid_nonnegative(gap) || !valid_nonnegative(minimum) || !value(size).is_finite() {
        return None;
    }
    let total = if axis == Axis::X { rect.width() } else { rect.height() };
    let gap = gap.min(total);
    let available = total - gap;
    let space = f64::from(available);
    // f64: two finite f32 inputs can overflow when multiplied.
    let wanted = match size {
        SplitSize::FixedFirst(points) => f64::from(points),
        SplitSize::FixedSecond(points) => space - f64::from(points),
        other => space * f64::from(value(other)),
    };
    let first_size = clamp_first(wanted, space, minimum) as f32;
    let (mut first, mut divider, mut second) = (rect, rect, rect);
    if axis == Axis::X {
        first.max.x = (rect.min.x + first_size).min(rect.max.x);
        divider.min.x = first.max.x;
        divider.max.x = (divider.min.x + gap).min(rect.max.x);
        second.min.x = divider.max.x;
    } else {
        first.max.y = (rect.min.y + first_size).min(rect.max.y);
        divider.min.y = first.max.y;
        divider.max.y = (divider.min.y + gap).min(rect.max.y);
        second.min.y = divider.max.y;
    }
    Some(SplitLayout { first, divider, second, first_size, available })
}

fn valid_rect(rect: Rect) -> bool {
    rect.min.is_finite()
        && rect.max.is_finite()
        && rect.width().is_finite()
        && rect.height().is_finite()
        && rect.min.x <= rect.max.x
        && rect.min.y <= rect.max.y
}

fn valid_nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

fn clamp_first(wanted: f64, available: f64, minimum: f32) -> f64 {
    let low = f64::from(minimum).min(available * 0.5);
    wanted.clamp(low, available - low)
}

/// Colours of a resize handle. The handle at rest paints no fill.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResizeHandleStyle {
    pub(crate) hovered: Color32,
    pub(crate) dragged: Color32,
    pub(crate) focused: Color32,
}

/// A focusable divider moved by a pointer drag, the arrows along its axis while it has focus, and accessibility's
/// Increment, Decrement and SetValue. It never stores a size: [`show`](ResizeHandle::show) returns the move along the
/// axis for the caller to apply.
pub(crate) struct ResizeHandle<'a> {
    id: Id,
    axis: Axis,
    rect: Rect,
    label: &'a str,
    style: ResizeHandleStyle,
    hit_margin: f32,
    value: Option<(f32, RangeInclusive<f32>)>,
}

/// Keyboard and accessibility step, in points.
const STEP: f32 = 8.0;

impl<'a> ResizeHandle<'a> {
    pub(crate) fn new(id: Id, axis: Axis, rect: Rect, label: &'a str, style: ResizeHandleStyle) -> Self {
        Self { id, axis, rect, label, style, hit_margin: 3.0, value: None }
    }

    /// Widens the hit rectangle along the movement axis, in points. Default: 3.
    pub(crate) fn hit_margin(mut self, margin: f32) -> Self {
        self.hit_margin = margin;
        self
    }

    /// The current size and its bounds, for accessibility and its SetValue action.
    pub(crate) fn value(mut self, value: f32, range: RangeInclusive<f32>) -> Self {
        self.value = Some((value, range));
        self
    }

    /// Shows the handle and returns the requested move along the axis this pass, in points (0 for none).
    pub(crate) fn show(self, ui: &mut Ui) -> f32 {
        let horizontal = self.axis == Axis::X;
        let hit = self.rect.expand2(if horizontal { Vec2::new(self.hit_margin, 0.0) } else { Vec2::new(0.0, self.hit_margin) });
        let valid = valid_rect(self.rect)
            && self.rect.width() > 0.0
            && self.rect.height() > 0.0
            && valid_rect(hit)
            && (0.0..=4096.0).contains(&self.hit_margin)
            && self
                .value
                .as_ref()
                .is_none_or(|(value, range)| value.is_finite() && range.start().is_finite() && range.end().is_finite() && range.start() <= range.end());
        let inert = Rect::from_min_size(ui.min_rect().min, Vec2::ZERO);
        let mut response = ui.interact(if valid { hit } else { inert }, self.id, if valid { Sense::drag() } else { Sense::hover() });
        let enabled = valid && response.enabled();
        let mut delta = 0.0;
        if enabled {
            drag_focus(ui, &response);
            if response.dragged() {
                let drag = response.drag_delta();
                delta = if horizontal { drag.x } else { drag.y };
            }
            if response.has_focus() {
                let filter = EventFilter { horizontal_arrows: horizontal, vertical_arrows: !horizontal, ..Default::default() };
                ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, filter));
                let (back, forward) = if horizontal { (Key::ArrowLeft, Key::ArrowRight) } else { (Key::ArrowUp, Key::ArrowDown) };
                ui.input_mut(|input| {
                    if input.consume_key(Modifiers::NONE, back) {
                        delta -= STEP;
                    }
                    if input.consume_key(Modifiers::NONE, forward) {
                        delta += STEP;
                    }
                });
            }
            let mut change = 0.0;
            ui.input_mut(|input| {
                input.consume_accesskit_action_requests(response.id, |request| match request.action {
                    accesskit::Action::Increment => {
                        change += STEP;
                        true
                    }
                    accesskit::Action::Decrement => {
                        change -= STEP;
                        true
                    }
                    accesskit::Action::SetValue => {
                        if let (Some((value, range)), Some(accesskit::ActionData::NumericValue(new_value))) = (&self.value, &request.data)
                            && new_value.is_finite()
                        {
                            change = (new_value.clamp(f64::from(*range.start()), f64::from(*range.end())) - f64::from(*value)) as f32;
                            delta = 0.0;
                        }
                        true
                    }
                    _ => false,
                });
            });
            delta += change;
        }
        if !delta.is_finite() {
            delta = 0.0;
        }
        if delta != 0.0 {
            response.mark_changed();
        }
        response.widget_info(|| match &self.value {
            Some((value, _)) if valid => egui::WidgetInfo::slider(enabled, f64::from(*value), self.label),
            _ => egui::WidgetInfo::labeled(egui::Role::Unknown, enabled, self.label),
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_role(accesskit::Role::Splitter);
            // Orientation describes the divider itself, across its movement axis.
            node.set_orientation(if horizontal { accesskit::Orientation::Vertical } else { accesskit::Orientation::Horizontal });
            if enabled {
                node.add_action(accesskit::Action::Increment);
                node.add_action(accesskit::Action::Decrement);
                if let Some((value, range)) = &self.value {
                    node.set_numeric_value(f64::from(*value));
                    node.set_min_numeric_value(f64::from(*range.start()));
                    node.set_max_numeric_value(f64::from(*range.end()));
                    node.set_numeric_value_step(f64::from(STEP));
                    node.add_action(accesskit::Action::SetValue);
                }
            }
        });
        if enabled {
            let fill = if response.dragged() {
                Some(self.style.dragged)
            } else if response.has_focus() {
                Some(self.style.focused)
            } else if response.hovered() {
                Some(self.style.hovered)
            } else {
                None
            };
            if let Some(fill) = fill {
                ui.painter().rect_filled(self.rect, 0.0, fill);
            }
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(if horizontal { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
            }
        }
        delta
    }
}

/// A pointer drag focuses its handle for the gesture only and gives the focus up on release, so the app's shortcuts
/// and its canvas arrow keys work again. A handle focused from the keyboard before the drag keeps its focus.
fn drag_focus(ui: &Ui, response: &Response) {
    let key = response.id.with("pointer-focus");
    if response.drag_started() {
        if !response.has_focus() {
            ui.ctx().data_mut(|d| d.insert_temp(key, ()));
        }
        response.request_focus();
    }
    if response.drag_stopped() && ui.ctx().data_mut(|d| d.remove_temp::<()>(key)).is_some() {
        response.surrender_focus();
    }
}
