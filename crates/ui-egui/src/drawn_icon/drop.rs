//! A drop of liquid, point up: the badge of a tool variant that sheds particles

use super::distance::{self, Point};

/// The round body and the point above it
const BODY_CENTRE: Point = [0.5, 0.62];
const BODY_RADIUS: f32 = 0.3;
const POINT: Point = [0.5, 0.06];

/// Where the point's sides meet the body tangentially: the centre plus the radius times (±cos θ, −sin θ), sin θ = 0.3 / 0.56 (radius over the point's distance)
const LEFT_TANGENT: Point = [0.247, 0.459];
const RIGHT_TANGENT: Point = [0.753, 0.459];

/// The distance from `point` to the drop's ink
pub(crate) fn glyph(point: Point) -> f32 {
    let body = distance::disc(point, BODY_CENTRE, BODY_RADIUS);
    let tip = distance::convex_polygon(point, &[POINT, RIGHT_TANGENT, LEFT_TANGENT]);
    body.min(tip)
}
