//! Where a point in a warp projection's frame lands in its image: the inverse of the bilinear quad its four corners make

use texturecraft_geom::glam::Vec2;

/// Texture coordinates (v down) of a point in frame units.
/// Corners run from top left clockwise. Coordinates extend past the quad for repeating images.
pub fn warp_uv(corners: &[[f32; 2]; 4], point: Vec2) -> Option<Vec2> {
    let [top_left, top_right, bottom_right, bottom_left] = corners.map(Vec2::from);
    let top_edge = top_right - top_left;
    let left_edge = bottom_left - top_left;
    let twist = top_left - top_right + bottom_right - bottom_left;
    let relative_point = point - top_left;

    let base_area = top_edge.perp_dot(left_edge);
    let quadratic = twist.perp_dot(left_edge);
    let linear = base_area + relative_point.perp_dot(twist);
    let constant = relative_point.perp_dot(top_edge);
    let at = |v| across(top_edge, left_edge, twist, relative_point, v).map(|u| Vec2::new(u, v));

    let uv = if quadratic.abs() <= RELATIVE_PARALLEL * base_area.abs() {
        // Parallel sides make the quad's equation linear in v
        at(-constant / linear)?
    } else {
        let discriminant = linear * linear - 4.0 * constant * quadratic;
        if discriminant < 0.0 {
            return None;
        }

        // Avoid subtracting near-equal values when the quad is nearly a parallelogram
        let root_term = -0.5 * (linear + discriminant.sqrt().copysign(linear));

        // At a double root of zero only the first candidate is finite
        let candidates = [at(root_term / quadratic), at(constant / root_term)];
        candidates.into_iter().flatten().min_by(|x, y| outside(*x).total_cmp(&outside(*y)))?
    };

    uv.is_finite().then_some(uv)
}

/// How far across (u) a point lies at height `v`, solved on whichever axis the quad's edge spans most
fn across(top_edge: Vec2, left_edge: Vec2, twist: Vec2, relative_point: Vec2, v: f32) -> Option<f32> {
    let span = top_edge + twist * v;
    let u = if span.x.abs() >= span.y.abs() { (relative_point.x - left_edge.x * v) / span.x } else { (relative_point.y - left_edge.y * v) / span.y };
    u.is_finite().then_some(u)
}

/// How far `uv` lies outside the image's 0..1 square (0 inside)
fn outside(uv: Vec2) -> f32 {
    (uv - uv.clamp(Vec2::ZERO, Vec2::ONE)).length_squared()
}

/// Relative area below which the quadratic contribution counts as parallel
const RELATIVE_PARALLEL: f32 = 1e-6;

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: [[f32; 2]; 4] = [[-0.5, 0.5], [0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]];

    /// The point at texture coordinates `uv` inside the quad `corners`
    fn bilinear(corners: &[[f32; 2]; 4], uv: Vec2) -> Vec2 {
        let [a, b, c, d] = corners.map(Vec2::from);
        let top = a.lerp(b, uv.x);
        let bottom = d.lerp(c, uv.x);
        top.lerp(bottom, uv.y)
    }

    #[test]
    fn the_unpulled_frame_maps_like_a_planar_frame() {
        for (x, y) in [(0.0, 0.0), (-0.5, 0.5), (0.25, -0.4), (1.3, 0.7), (-2.0, -1.5)] {
            assert_eq!(warp_uv(&FRAME, Vec2::new(x, y)), Some(Vec2::new(x + 0.5, 0.5 - y)), "{x} {y}");
        }
    }

    #[test]
    fn pulled_corners_bend_the_image_to_their_quad() {
        let quads = [
            [[-0.6, 0.4], [0.5, 0.7], [0.3, -0.5], [-0.4, -0.45]],
            [[-0.5, 0.5], [0.9, 0.5], [0.5, -0.5], [-0.5, -0.5]],
            [[-0.2, 0.6], [0.2, 0.6], [0.7, -0.6], [-0.7, -0.6]],
            // Nearly a parallelogram: one corner a hair out
            [[0.0, 0.0], [1.0, 0.0], [1.0 + 1.0 / 262_144.0, 1.0], [0.0, 1.0]],
        ];
        for quad in quads {
            for i in 0..=10 {
                for j in 0..=10 {
                    let uv = Vec2::new(i as f32 / 10.0, j as f32 / 10.0);
                    let found = warp_uv(&quad, bilinear(&quad, uv)).unwrap_or(Vec2::NAN);

                    assert!((found - uv).length() < 1e-4, "{quad:?} {uv}: {found}");
                }
            }
        }
    }

    #[test]
    fn shrinking_a_distorted_quad_preserves_its_mapping() {
        let corners = [[0.0, 0.0], [1.0, 0.0], [2.0, 1.0], [0.0, 1.0]];
        let uv = Vec2::splat(0.5);
        let point = bilinear(&corners, uv);
        let scale = 0.0001;
        let smaller = corners.map(|corner| corner.map(|coordinate| coordinate * scale));

        let found = warp_uv(&smaller, point * scale).unwrap();

        assert!(found.distance(uv) < 1e-5, "the scaled quad still maps to the middle: {found}");
    }

    #[test]
    fn a_collapsed_quad_lands_nowhere_instead_of_failing() {
        let point = [[0.0, 0.0]; 4];

        assert_eq!(warp_uv(&point, Vec2::new(0.3, 0.2)), None);
        assert_eq!(warp_uv(&[[f32::NAN; 2]; 4], Vec2::ZERO), None);
    }
}
