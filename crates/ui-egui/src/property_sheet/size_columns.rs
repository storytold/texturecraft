//! A row of vertical sliders across the panel, one per size, each with its value above and its name below (Curvature's detail sizes)

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, WidgetInfo, pos2};

use super::slider::{KNOB_RADIUS, KNOB_STROKE};
use super::{PropertySheet, SMALL_TEXT_SIZE, SizeColumnsResponse};
use crate::theme::semibold;

/// A column's value line above its track, the track's length, and its name line under it
const VALUE_LINE: f32 = 20.0;
const COLUMN_TRACK: f32 = 72.0;
const NAME_LINE: f32 = 22.0;

/// How far a hovered column's face reaches past its value and name lines, and the air between neighbouring faces
const FACE_OVERHANG: f32 = 4.0;
const FACE_AIR: f32 = 2.0;

/// Width of a column's track and fill
const COLUMN_TRACK_WIDTH: f32 = 2.0;

/// How far one arrow key press moves a focused column: a twentieth of its track
const KEY_STEP: f32 = 0.05;

impl PropertySheet<'_> {
    /// One 0..1 vertical slider per name in `values`, side by side across the panel
    pub(crate) fn size_columns(&mut self, names: &[&str], values: &mut [f32]) -> SizeColumnsResponse {
        let block = self.take(VALUE_LINE + COLUMN_TRACK + NAME_LINE);
        let pitch = (self.right - self.left) / names.len().max(1) as f32;
        let (track_top, track_bottom) = (block.top() + VALUE_LINE, block.top() + VALUE_LINE + COLUMN_TRACK);
        let tokens = self.tokens;
        let mut response = SizeColumnsResponse::default();
        for (index, (name, value)) in names.iter().zip(values.iter_mut()).enumerate() {
            let x = self.left + pitch * (index as f32 + 0.5);
            let column = Rect::from_min_max(pos2(x - pitch / 2.0, block.top()), pos2(x + pitch / 2.0, block.bottom()));
            let id = self.next_id();
            let hit = self.ui.interact(column, id, Sense::click_and_drag());
            hit.widget_info(|| WidgetInfo::slider(true, f64::from(*value), *name));

            let mut moved = false;
            if (hit.dragged() || hit.drag_stopped() || hit.clicked())
                && let Some(pointer) = hit.interact_pointer_pos()
            {
                let picked = (1.0 - (pointer.y - track_top) / COLUMN_TRACK).clamp(0.0, 1.0);
                if picked != *value {
                    *value = picked;
                    moved = true;
                }
            }

            // A focused column keeps the arrow keys: up and right raise it, down and left lower it, each press its own step
            if hit.has_focus() {
                self.ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
                });
                let presses = self.ui.input(|input| {
                    let raised = input.num_presses(egui::Key::ArrowUp) + input.num_presses(egui::Key::ArrowRight);
                    let lowered = input.num_presses(egui::Key::ArrowDown) + input.num_presses(egui::Key::ArrowLeft);
                    raised as f32 - lowered as f32
                });
                let stepped = (*value + presses * KEY_STEP).clamp(0.0, 1.0);
                if stepped != *value {
                    *value = stepped;
                    moved = true;
                }
            }
            if let Some(merge) = crate::widgets::press_edit(self.ui, id, moved) {
                response.edit = Some((index, merge));
            }
            let hovered = hit.hovered() || hit.dragged();
            if hovered {
                response.hovered = Some(index);
            }

            let painter = self.ui.painter();
            if hovered {
                let face = Rect::from_min_max(
                    pos2(column.left() + FACE_AIR, block.top() - FACE_OVERHANG),
                    pos2(column.right() - FACE_AIR, block.bottom() + FACE_OVERHANG),
                );
                painter.rect(
                    face,
                    CornerRadius::same(super::CONTROL_RADIUS),
                    tokens.row_hover,
                    Stroke::new(1.0, tokens.field_border),
                    egui::StrokeKind::Inside,
                );
            }
            painter.text(pos2(x, block.top() + VALUE_LINE / 2.0), Align2::CENTER_CENTER, format!("{value:.2}"), semibold(SMALL_TEXT_SIZE), tokens.text);
            painter.line_segment([pos2(x, track_top), pos2(x, track_bottom)], Stroke::new(COLUMN_TRACK_WIDTH, tokens.button_border));
            let knob_y = track_bottom - value.clamp(0.0, 1.0) * COLUMN_TRACK;
            painter.line_segment([pos2(x, knob_y), pos2(x, track_bottom)], Stroke::new(COLUMN_TRACK_WIDTH, tokens.accent));
            let outline = if hovered { tokens.accent } else { tokens.text };
            painter.circle(pos2(x, knob_y), KNOB_RADIUS, tokens.panel, Stroke::new(KNOB_STROKE, outline));
            let (font, color) = if hovered { (semibold(SMALL_TEXT_SIZE), tokens.text) } else { (egui::FontId::proportional(SMALL_TEXT_SIZE), tokens.text_dim) };
            painter.text(pos2(x, track_bottom + NAME_LINE / 2.0), Align2::CENTER_CENTER, *name, font, color);
            self.paint_control_focus(&hit);
        }

        self.drawn(block.bottom());
        response
    }
}
