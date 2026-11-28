//! A pen nib drawn solid, point down: the Path tool filling its outline instead of painting along it

use super::distance::{self, Point};

/// The cap the nib hangs from, apart from the body
const CAP: [Point; 2] = [[0.34, 0.06], [0.66, 0.22]];

/// The body: square shoulders under the cap, widening, then narrowing to the point
const BODY: [Point; 5] = [[0.3, 0.29], [0.7, 0.29], [0.84, 0.56], [0.5, 0.96], [0.16, 0.56]];

/// The breather hole, and the slit from it down to the point, both clear
const HOLE_CENTRE: Point = [0.5, 0.56];
const HOLE_RADIUS: f32 = 0.08;
const SLIT: [Point; 2] = [[0.475, 0.56], [0.525, 1.0]];

/// The distance from `point` to the nib's ink
pub(crate) fn glyph(point: Point) -> f32 {
    let cap = distance::rectangle(point, CAP[0], CAP[1]);
    let cut = distance::disc(point, HOLE_CENTRE, HOLE_RADIUS).min(distance::rectangle(point, SLIT[0], SLIT[1]));
    let body = distance::convex_polygon(point, &BODY).max(-cut);
    cap.min(body)
}
