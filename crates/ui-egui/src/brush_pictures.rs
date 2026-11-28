//! Pictures of what the brush settings do: the brush's stamp turned to its Angle, a strip of jittered dabs and the alignment modes

use egui::{ColorImage, Painter, Pos2, Rect, Shape, Stroke, pos2};
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::paint::{Alignment, BrushSettings};

use crate::preview_image::stroke_strip;
use crate::theme::Tokens;

/// The stamp's radius in the Orientation picture, as a share of half its side: room for the dial's ring around it
const STAMP_RADIUS_SHARE: f32 = 0.62;

/// The dial's ring radius, as a share of half the picture's side
const RING_RADIUS_SHARE: f32 = 0.86;

/// How far the Angle tick reaches in and out of the ring, in points
const TICK_REACH: f32 = 3.0;

/// The ring's and the tick's line widths, in points
const RING_WIDTH: f32 = 1.0;
const TICK_WIDTH: f32 = 2.0;

/// How far a dab may reach from the strip's centre line, as a share of half its height
const STRIP_REACH_SHARE: f32 = 0.85;

/// The least spacing between the strip's dabs, in stamp diameters, so each dab reads on its own
const STRIP_DAB_SPACING: f32 = 1.4;

/// The brush's own stamp turned to its Angle, alone in a `size` square: jitter, flow and opacity left out so only the shape and its turn show
pub(crate) fn stamp_image(brush: &BrushSettings, image: Option<&ImageRes>, size: [usize; 2], _pixels_per_point: f32, tints: &[[u8; 3]]) -> ColorImage {
    let still = BrushSettings {
        jitter_size: 0.0,
        jitter_angle: 0.0,
        jitter_flow: 0.0,
        jitter_position: 0.0,
        follow_path: false,
        flow: 1.0,
        opacity: 1.0,
        ..brush.clone()
    };
    let center = (size[0] as f32 * 0.5, size[1] as f32 * 0.5);
    let radius = center.0.min(center.1) * STAMP_RADIUS_SHARE;

    // A curve that stays put lays exactly one stamp
    stroke_strip(&still, image, size, |_| radius, |_| center, |x| column_tint(tints, x))
}

/// The brush's alpha as its Parameters shape it, upright: the Alpha section's Parameters picture
pub(crate) fn alpha_image(brush: &BrushSettings, image: Option<&ImageRes>, size: [usize; 2], pixels_per_point: f32, tints: &[[u8; 3]]) -> ColorImage {
    stamp_image(&BrushSettings { angle: 0.0, ..brush.clone() }, image, size, pixels_per_point, tints)
}

/// A row of the brush's dabs across a `size` strip, scattered by its four jitters; dabs shrink as Position Jitter grows so the scatter stays inside
pub(crate) fn jitter_image(brush: &BrushSettings, image: Option<&ImageRes>, size: [usize; 2], _pixels_per_point: f32, tints: &[[u8; 3]]) -> ColorImage {
    let spaced = BrushSettings { spacing: brush.spacing.max(STRIP_DAB_SPACING), follow_path: false, flow: 1.0, opacity: 1.0, ..brush.clone() };
    let (width, middle) = (size[0] as f32, size[1] as f32 * 0.5);

    // A dab lands up to Position Jitter radii from the line, so its radius and that reach together fill the strip's half height
    let radius = middle * STRIP_REACH_SHARE / (1.0 + brush.jitter_position.max(0.0));
    let curve = |t: f32| (radius + t * (width - 2.0 * radius), middle);
    stroke_strip(&spaced, image, size, |_| radius, curve, |x| column_tint(tints, x))
}

fn column_tint(tints: &[[u8; 3]], x: usize) -> [u8; 3] {
    tints.get(x).copied().unwrap_or([u8::MAX; 3])
}

/// The dial around the Orientation picture's stamp: a ring and an accent tick at `degrees` (counter-clockwise from the right, as the stamp turns)
pub(crate) fn dial_ring(painter: &Painter, rect: Rect, degrees: f32, tokens: &Tokens) {
    let center = rect.center();
    let ring = rect.width().min(rect.height()) * 0.5 * RING_RADIUS_SHARE;
    painter.circle_stroke(center, ring, Stroke::new(RING_WIDTH, tokens.button_border));

    let (sin, cos) = degrees.to_radians().sin_cos();
    let at = |reach: f32| pos2(center.x + reach * cos, center.y - reach * sin);
    painter.line_segment([at(ring - TICK_REACH), at(ring + TICK_REACH)], Stroke::new(TICK_WIDTH, tokens.accent));
}

/// Points along the Alignment picture's surface
const SURFACE_POINTS: usize = 16;

/// The surface's bulge and the stamp's height above it, as shares of the picture's height
const SURFACE_BULGE_SHARE: f32 = 0.45;
const STAMP_LIFT_SHARE: f32 = 0.12;

/// Margin inside the Alignment picture, in points
const ALIGNMENT_INSET: f32 = 2.0;

/// The middle share of the surface the stamp covers
const STAMP_SPAN_SHARE: f32 = 0.6;

/// How an Alignment lays the stamp on a curved surface: bent along it (Tangent | Wrap) or flat towards the view (Camera)
pub(crate) fn alignment_picture(painter: &Painter, rect: Rect, alignment: Alignment, tokens: &Tokens) {
    let rect = rect.shrink(ALIGNMENT_INSET);
    let bulge = rect.height() * SURFACE_BULGE_SHARE;
    let surface_y = |across: f32| rect.bottom() - bulge * (1.0 - across * across);
    let point = |across: f32, lift: f32| pos2(rect.center().x + across * rect.width() * 0.5, surface_y(across) - lift);
    let line = |from: f32, to: f32, lift: f32| -> Vec<Pos2> {
        (0..=SURFACE_POINTS).map(|step| point(from + (to - from) * step as f32 / SURFACE_POINTS as f32, lift)).collect()
    };
    painter.add(Shape::line(line(-1.0, 1.0, 0.0), Stroke::new(RING_WIDTH, tokens.text_dim)));

    let lift = rect.height() * STAMP_LIFT_SHARE;
    let stamp = match alignment {
        Alignment::Tangent => line(-STAMP_SPAN_SHARE, STAMP_SPAN_SHARE, lift),
        Alignment::Camera => {
            let top = surface_y(0.0) - lift;
            vec![pos2(point(-STAMP_SPAN_SHARE, 0.0).x, top), pos2(point(STAMP_SPAN_SHARE, 0.0).x, top)]
        }
    };
    painter.add(Shape::line(stamp, Stroke::new(TICK_WIDTH, tokens.accent)));
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::paint::BrushAlpha;

    use super::*;

    const STAMP_SIZE: [usize; 2] = [48, 48];
    const STRIP_SIZE: [usize; 2] = [240, 40];

    fn white(width: usize) -> Vec<[u8; 3]> {
        vec![[u8::MAX; 3]; width]
    }

    #[test]
    fn the_stamp_turns_with_angle() {
        let upright = BrushSettings { alpha: BrushAlpha::Streaks, angle: 0.0, ..BrushSettings::default() };
        let turned = BrushSettings { angle: 45.0, ..upright.clone() };

        let picture = |brush: &BrushSettings| stamp_image(brush, None, STAMP_SIZE, 1.0, &white(STAMP_SIZE[0])).pixels;

        assert!(picture(&upright).iter().any(|pixel| pixel.a() > 0), "the stamp shows");
        assert_ne!(picture(&upright), picture(&turned));
    }

    #[test]
    fn each_jitter_changes_the_strip() {
        let steady = BrushSettings { hardness: 1.0, ..BrushSettings::default() };
        let strip = |brush: &BrushSettings| jitter_image(brush, None, STRIP_SIZE, 1.0, &white(STRIP_SIZE[0])).pixels;
        let jittered = [
            BrushSettings { jitter_size: 0.8, ..steady.clone() },
            BrushSettings { jitter_flow: 0.8, ..steady.clone() },
            BrushSettings { jitter_angle: 0.8, alpha: BrushAlpha::Streaks, ..steady.clone() },
            BrushSettings { jitter_position: 1.0, ..steady.clone() },
        ];

        for brush in &jittered {
            let unjittered = BrushSettings { jitter_size: 0.0, jitter_flow: 0.0, jitter_angle: 0.0, jitter_position: 0.0, ..brush.clone() };
            assert_ne!(strip(brush), strip(&unjittered), "{brush:?}");
        }
    }
}
