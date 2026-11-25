//! The eyedropper: a pipette lying from its tip at the bottom left to its rubber bulb at the top right

use std::f32::consts::FRAC_1_SQRT_2;

use super::distance::{self, Point, dot, sub};

/// The tip's point in the unit square
const TIP: Point = [0.1, 0.9];

/// From the tip toward the bulb, and across the pipette, as unit directions
const ALONG: Point = [FRAC_1_SQRT_2, -FRAC_1_SQRT_2];
const ACROSS: Point = [FRAC_1_SQRT_2, FRAC_1_SQRT_2];

/// The width of the tube's outline
pub(crate) const STROKE: f32 = 0.08;

/// The tube's outline as `(along, across)` from the tip: a narrow tip widening into the straight tube, through the stroke's middle
const TUBE: [Point; 6] = [[0.0, -0.03], [0.18, -0.07], [0.62, -0.07], [0.62, 0.07], [0.18, 0.07], [0.0, 0.03]];

/// The collar between the tube and the bulb, wider than both
const COLLAR: [Point; 2] = [[0.62, -0.24], [0.7, 0.24]];

/// The bulb: straight from inside the collar (touching shapes leave a seam) to its round end's centre, as wide as that end
const BULB_FROM: f32 = 0.66;
const BULB_END: f32 = 0.93;
const BULB_RADIUS: f32 = 0.19;

/// The distance from `point` to the glyph's ink: the tube's outline (hollow, so it reads as glass), the collar and the bulb; the collar and the slim tube under a fatter bulb tell a pipette from a baster
pub(crate) fn glyph(point: Point) -> f32 {
    let local = local(point);
    let tube = distance::convex_polygon(local, &TUBE).abs() - STROKE / 2.0;
    tube.min(head(local))
}

/// The distance from `point` to the whole pipette's outer edge, the tube filled in: the pointer's white
pub(crate) fn silhouette(point: Point) -> f32 {
    let local = local(point);
    let tube = distance::convex_polygon(local, &TUBE) - STROKE / 2.0;
    tube.min(head(local))
}

/// The collar and the bulb, in the pipette's own `(along, across)` frame
fn head(local: Point) -> f32 {
    let collar = distance::rectangle(local, COLLAR[0], COLLAR[1]);
    let bulb = distance::rectangle(local, [BULB_FROM, -BULB_RADIUS], [BULB_END, BULB_RADIUS]).min(distance::disc(local, [BULB_END, 0.0], BULB_RADIUS));
    collar.min(bulb)
}

/// `point` as `(along, across)` from the tip
fn local(point: Point) -> Point {
    let offset = sub(point, TIP);
    [dot(offset, ALONG), dot(offset, ACROSS)]
}

/// `(along, across)` from the tip as a point in the unit square
#[cfg(test)]
pub(crate) fn square(along: f32, across: f32) -> Point {
    [TIP[0] + ALONG[0] * along + ACROSS[0] * across, TIP[1] + ALONG[1] * along + ACROSS[1] * across]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pipette's full width at `along` from the tip, sampled across it
    fn width(along: f32) -> f32 {
        let steps = 400;
        let inked = (0..=steps).filter(|step| silhouette(square(along, -0.3 + 0.6 * *step as f32 / steps as f32)) <= 0.0).count();
        0.6 * inked as f32 / steps as f32
    }

    #[test]
    fn a_slim_tube_then_a_collar_wider_than_the_bulb() {
        let (tip, tube, collar, bulb) = (width(0.02), width(0.4), width(0.66), width(0.85));

        assert!(tip < tube, "the tip narrows: {tip} < {tube}");
        assert!(tube * 1.5 < bulb, "the bulb is far fatter than the tube: {tube} vs {bulb}");
        assert!(collar > bulb, "the collar sticks out past the bulb: {collar} vs {bulb}");
    }

    #[test]
    fn the_glyphs_tube_is_hollow_and_its_head_solid() {
        let tube_middle = square(0.4, 0.0);
        assert!(glyph(tube_middle) > 0.0, "the glass tube shows the ground inside its outline");
        assert!(silhouette(tube_middle) < 0.0, "the pointer fills it");
        assert!(glyph(square(0.4, 0.07)) < 0.0, "the tube's outline is ink");
        assert!(glyph(square(0.85, 0.0)) < 0.0 && glyph(square(0.66, 0.2)) < 0.0, "bulb and collar are solid");
        let seam = (0..=40).map(|step| glyph(square(0.7, -0.15 + 0.3 * step as f32 / 40.0))).fold(f32::NEG_INFINITY, f32::max);
        assert!(seam < -0.02, "no seam where the collar meets the bulb: {seam}");
        for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            assert!(silhouette(corner) > 0.0, "the pipette stays inside its square: {corner:?}");
        }
    }
}
