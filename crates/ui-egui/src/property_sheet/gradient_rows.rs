//! A gradient's ramp with a marker per color, and a color's row with its place on the ramp

use std::ops::RangeInclusive;

use egui::{CornerRadius, Id, Mesh, Rect, Sense, Shape, Stroke, StrokeKind, WidgetInfo, pos2, vec2};

use super::PropertySheet;
use super::marker_drag::MarkerDrag;
use super::slider::KNOB_RADIUS;
use super::track::Track;
use crate::color_picker::ColorPick;
use crate::fill_material_panel::shown_color;

/// The ramp's inset from the rows' edges, so the end markers stay inside them
const RAMP_INSET: f32 = 5.0;

/// Height of the ramp, the air under it, and a marker's height and half width
const RAMP_HEIGHT: f32 = 16.0;
const MARKER_AIR: f32 = 2.0;
const MARKER_HEIGHT: f32 = 12.0;
const MARKER_HALF: f32 = 5.0;

/// How far beyond a marker's sides it still answers a drag
const MARKER_HIT_MARGIN: f32 = 1.0;

/// From the marker's tip to where its sides turn straight down
const MARKER_SHOULDER: f32 = 6.0;

/// The whole ramp block: ramp, markers and the air under them
const RAMP_BLOCK: f32 = 40.0;

/// Slices the ramp is drawn in (colors blend between them)
const RAMP_SLICES: usize = 64;

/// Width of a color row's swatch and its height
const SWATCH_WIDTH: f32 = 44.0;
const SWATCH_HEIGHT: f32 = 18.0;

/// From the controls column to the eyedropper's centre and to the position track
const EYEDROPPER_X: f32 = 56.0;
const STOP_TRACK_X: f32 = 72.0;

impl PropertySheet<'_> {
    /// The ramp as `at` (linear colors along 0..1) draws it, with a marker under it for each of `colors` at its place in `positions`; a marker drags along it
    pub(crate) fn gradient_ramp(&mut self, colors: &[[f32; 3]], positions: &[f32], at: &dyn Fn(f32) -> [f32; 3]) -> Option<MarkerDrag> {
        let block = self.take(RAMP_BLOCK);
        let (x0, x1) = (self.left + RAMP_INSET, self.right - RAMP_INSET);
        let ramp = Rect::from_min_max(pos2(x0, block.top()), pos2(x1, block.top() + RAMP_HEIGHT));
        let tokens = self.tokens;
        let mut mesh = Mesh::default();
        for slice in 0..=RAMP_SLICES {
            let share = slice as f32 / RAMP_SLICES as f32;
            let x = x0 + (x1 - x0) * share;
            let color = shown_color(at(share));
            mesh.colored_vertex(pos2(x, ramp.top()), color);
            mesh.colored_vertex(pos2(x, ramp.bottom()), color);
            if slice > 0 {
                let base = (slice as u32) * 2;
                mesh.add_triangle(base - 2, base - 1, base);
                mesh.add_triangle(base - 1, base, base + 1);
            }
        }
        let painter = self.ui.painter().clone();
        painter.add(Shape::mesh(mesh));
        painter.rect_stroke(ramp, CornerRadius::ZERO, Stroke::new(1.0, tokens.field_border), StrokeKind::Inside);

        let mut drag = None;
        for (index, color) in colors.iter().enumerate() {
            let position = positions.get(index).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let x = x0 + (x1 - x0) * position;
            let top = ramp.bottom() + MARKER_AIR;
            let hit = Rect::from_min_max(pos2(x - MARKER_HALF - MARKER_HIT_MARGIN, top), pos2(x + MARKER_HALF + MARKER_HIT_MARGIN, top + MARKER_HEIGHT));
            let id = self.next_id();
            let response = self.ui.interact(hit, id, Sense::drag());
            response.widget_info(|| WidgetInfo::slider(true, f64::from(position), crate::i18n::trf("Color {number}", &[("number", &(index + 1))])));
            let moved_to = response
                .interact_pointer_pos()
                .filter(|_| response.dragged())
                .map(|pointer| ((pointer.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0))
                .filter(|picked| *picked != position);
            if let Some(merge) = crate::widgets::press_edit(self.ui, response.id, moved_to.is_some())
                && let Some(moved_to) = moved_to
            {
                drag = Some(MarkerDrag { index, position: moved_to, merge });
            }
            let outline = if response.hovered() || response.dragged() { tokens.accent } else { tokens.text_dim };
            let points = vec![
                pos2(x, top),
                pos2(x + MARKER_HALF, top + MARKER_SHOULDER),
                pos2(x + MARKER_HALF, top + MARKER_HEIGHT),
                pos2(x - MARKER_HALF, top + MARKER_HEIGHT),
                pos2(x - MARKER_HALF, top + MARKER_SHOULDER),
            ];
            painter.add(Shape::convex_polygon(points, shown_color(*color), Stroke::new(1.0, outline)));
        }
        self.drawn(ramp.bottom() + MARKER_AIR + MARKER_HEIGHT);
        drag
    }

    /// Color N: its swatch (a click opens the picker) and eyedropper, then its place on the ramp; returns the pick and the place's edit
    pub(crate) fn color_stop(
        &mut self,
        label: &str,
        linear: [f32; 3],
        position: &mut f32,
        eyedropping: Option<Id>,
        swatches: &[[f32; 3]],
    ) -> (ColorPick, Option<bool>) {
        let (control, center_y) = self.row(label);
        let swatch = Rect::from_min_size(pos2(control.left(), center_y - SWATCH_HEIGHT / 2.0), vec2(SWATCH_WIDTH, SWATCH_HEIGHT));
        let pick = self.color_bar_at(swatch, control.left() + EYEDROPPER_X, label, linear, eyedropping, swatches);

        let id = self.next_id();
        let range: RangeInclusive<f32> = 0.0..=1.0;
        let track = Track { start: control.left() + STOP_TRACK_X + KNOB_RADIUS, end: self.track_end(), center_y, zero: None, ramp: false };
        let dragged = self.track(id, label, &track, position, &range);
        let typed = self.value_field(id, label, center_y, *position, &range, 2, "");
        (pick, Self::apply_typed(position, typed, dragged))
    }
}
