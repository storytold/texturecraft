//! The invert icon: a square, its left half clear and its right half solid, an arrow pointing out of each half

use super::distance::{self, Point};

/// The frame's outline width and where its stroke's middle runs
const STROKE: f32 = 0.08;
const FRAME: [Point; 2] = [[0.13, 0.13], [0.87, 0.87]];

/// Where the halves meet, and the arrows' middle line
const MIDDLE: f32 = 0.5;

/// An arrow's shaft (half thickness, length from the middle), its head (half height, from the middle to its base and to its point)
const SHAFT_HALF: f32 = 0.06;
const SHAFT_LENGTH: f32 = 0.16;
const HEAD_HALF: f32 = 0.17;
const HEAD_BASE: f32 = 0.12;
const HEAD_POINT: f32 = 0.29;

/// The distance from `point` to the icon's ink: the left arrow inked on the clear half, the right one cut out of the solid half (black becomes white and white becomes black)
pub(crate) fn glyph(point: Point) -> f32 {
    let frame = distance::rectangle(point, FRAME[0], FRAME[1]).abs() - STROKE / 2.0;
    let solid_half = distance::rectangle(point, [MIDDLE, FRAME[0][1]], FRAME[1]);
    frame.min(solid_half).max(-arrow(point, 1.0)).min(arrow(point, -1.0))
}

/// The distance from `point` to the arrow pointing from the middle toward `side` (1 right, -1 left)
fn arrow(point: Point, side: f32) -> f32 {
    let x = |from_middle: f32| MIDDLE + side * from_middle;
    let (shaft_end, base, tip) = (x(SHAFT_LENGTH), x(HEAD_BASE), x(HEAD_POINT));
    let shaft = distance::rectangle(point, [MIDDLE.min(shaft_end), MIDDLE - SHAFT_HALF], [MIDDLE.max(shaft_end), MIDDLE + SHAFT_HALF]);
    let head = distance::convex_polygon(point, &[[base, MIDDLE - HEAD_HALF], [tip, MIDDLE], [base, MIDDLE + HEAD_HALF]]);
    shaft.min(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_left_half_is_clear_with_an_inked_arrow_the_right_solid_with_a_cut_one() {
        let ink = |x: f32, y: f32| glyph([x, y]) < 0.0;

        assert!(!ink(0.3, 0.25) && ink(0.7, 0.25), "left half clear, right half solid");
        assert!(ink(0.13, 0.5) && ink(0.5, 0.13) && ink(0.87, 0.87), "the frame closes the square");
        assert!(ink(0.25, 0.5) && ink(0.4, 0.5), "the left arrow is ink, point and shaft");
        assert!(!ink(0.75, 0.5) && !ink(0.6, 0.5), "the right arrow is cut out of the solid half");
        assert!(!ink(0.25, 0.3) && ink(0.75, 0.3), "each head narrows toward its point");
        assert!(!ink(0.03, 0.5) && !ink(0.5, 0.97), "nothing past the frame");
    }
}
