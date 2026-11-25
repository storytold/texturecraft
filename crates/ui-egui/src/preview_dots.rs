//! The preview square's two dots: a ring where the projection sits with Offset 0, a filled dot where its Offset puts it, a dotted line between

use egui::{Painter, Pos2, Rect, Stroke, Vec2};

use crate::theme::Tokens;

/// The ring's radius (points)
const RING_RADIUS: f32 = 5.5;

/// The ring's line width (points)
const RING_LINE: f32 = 1.6;

/// The width of the line under the ring that keeps it readable on the checker (points)
const RING_UNDER: f32 = 3.6;

/// The filled dot's radius (points)
const DOT_RADIUS: f32 = 3.3;

/// The width of the rim under the filled dot (points)
const DOT_RIM: f32 = 1.0;

/// The dotted line's air between its dots (points)
const LINE_SPACING: f32 = 3.5;

/// The radius of the dotted line's dots (points)
const LINE_DOT: f32 = 0.9;

/// The width of the rim under the dotted line's dots (points)
const LINE_RIM: f32 = 0.6;

/// Bound paint work even when the preview stretches across a very large clip rectangle.
const MAX_LINE_DOTS: usize = 256;

/// Paint the two markers and their dotted connector, retaining the spacing from the filled dot.
pub(crate) fn preview_dots(painter: &Painter, center: Pos2, position: Pos2, tokens: &Tokens) {
    let span = center - position;
    let length = span.length();
    let start = DOT_RADIUS + LINE_SPACING;
    let end = length - RING_RADIUS - LINE_SPACING / 2.0;
    let clip = painter.clip_rect();

    if length.is_finite() && end > start && !clip.is_negative() {
        let toward = span / length;
        let radius = LINE_DOT + LINE_RIM;
        if let Some((visible_start, visible_end)) = visible_distances(clip.expand(radius), position, toward, start, end) {
            let first = ((visible_start - start) / LINE_SPACING).ceil() as usize;
            let last = ((visible_end - start) / LINE_SPACING).floor() as usize;
            for step in first..=last.min(first.saturating_add(MAX_LINE_DOTS - 1)) {
                let at = position + toward * (start + step as f32 * LINE_SPACING);
                painter.circle_filled(at, radius, tokens.field);
                painter.circle_filled(at, LINE_DOT, tokens.text);
            }
        }
    }

    painter.circle_stroke(center, RING_RADIUS, Stroke::new(RING_UNDER, tokens.field));
    painter.circle_stroke(center, RING_RADIUS, Stroke::new(RING_LINE, tokens.text));
    painter.circle_filled(position, DOT_RADIUS + DOT_RIM, tokens.field);
    painter.circle_filled(position, DOT_RADIUS, tokens.text);
}

/// Restrict the connector's distance interval to where it intersects the clip rectangle.
fn visible_distances(clip: Rect, origin: Pos2, direction: Vec2, mut start: f32, mut end: f32) -> Option<(f32, f32)> {
    for axis in 0..2 {
        let origin = origin[axis];
        let (low, high) = (clip.min[axis], clip.max[axis]);
        let direction = direction[axis];
        if direction == 0.0 {
            if origin < low || origin > high {
                return None;
            }

            continue;
        }

        let (near, far) = ((low - origin) / direction, (high - origin) / direction);
        start = start.max(near.min(far));
        end = end.min(near.max(far));
    }

    (start <= end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use egui::{Context, Id, LayerId, Order, Shape, pos2};

    use super::*;

    #[test]
    fn the_visible_connector_reaches_a_ring_when_the_offset_dot_is_far_outside() {
        let clip = Rect::from_min_max(Pos2::ZERO, pos2(100.0, 100.0));
        let tokens = Tokens::for_brightness(crate::theme::Brightness::MediumDark);
        let ctx = Context::default();
        let painter_layer = LayerId::new(Order::Foreground, Id::unique("connector"));

        let connectors = [(clip.center(), pos2(4000.0, 50.0)), (clip.center(), pos2(80.0, 50.0)), (pos2(-4000.0, -20.0), pos2(4000.0, -20.0))];
        let results = connectors.map(|(ring, dot)| {
            let mut output = ctx.run_ui(Default::default(), |ui| preview_dots(&ui.ctx().layer_painter(painter_layer).with_clip_rect(clip), ring, dot, &tokens));
            output.textures_delta.clear();
            let dots = output.shapes.iter().filter_map(|shape| match &shape.shape {
                Shape::Circle(circle) if circle.radius == LINE_DOT => Some(circle.center),
                _ => None,
            });
            let (queued, visible) = dots.fold((0, 0), |(queued, visible), dot| (queued + 1, visible + usize::from(clip.contains(dot))));
            eprintln!("{ring:?} to {dot:?}: {queued} queued connector dots, {visible} visible");

            (queued, visible)
        });

        assert!(results[0].1 > 2 && results[1].1 > 2, "the painted connectors reach the visible ring");
        assert_eq!(results[2].0, 0, "a connector wholly outside the square queues no dots");
    }
}
