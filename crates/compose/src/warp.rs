//! Where a point in a warp projection's frame lands in its image: the inverse of the bilinear quad its four corners make

use texturecraft_geom::glam::Vec2;

/// Texture coordinates (v down) of the point `p` (across and up in frame sizes) inside the quad `corners` makes (top left, top right, bottom right, bottom left); past the quad they run on, so the image can repeat. None where no texture coordinates land
pub fn warp_uv(corners: &[[f32; 2]; 4], p: Vec2) -> Option<Vec2> {
    let [a, b, c, d] = corners.map(Vec2::from);
    let (e, f, g, h) = (b - a, d - a, a - b + c - d, p - a);
    let k2 = g.perp_dot(f);
    let k1 = e.perp_dot(f) + h.perp_dot(g);
    let k0 = h.perp_dot(e);
    let uv = if k2.abs() < PARALLEL {
        // Opposite edges parallel: the quad's equation is linear in v
        let v = -k0 / k1;
        Vec2::new(across(e, f, g, h, v)?, v)
    } else {
        let discriminant = k1 * k1 - 4.0 * k0 * k2;
        if discriminant < 0.0 {
            return None;
        }

        let root = discriminant.sqrt();
        let near = |v: f32| across(e, f, g, h, v).map(|u| Vec2::new(u, v));

        // Two solutions, neither found by subtracting near-equal values (which loses the answer when the quad is nearly a parallelogram): keep the one nearer the quad
        let q = -0.5 * (k1 + root.copysign(k1));
        // q is 0 only for a double root at 0, which q / k2 still finds (k0 / q is then not finite, so no candidate)
        let candidates = [near(q / k2), near(k0 / q)];
        candidates.into_iter().flatten().min_by(|x, y| outside(*x).total_cmp(&outside(*y)))?
    };
    uv.is_finite().then_some(uv)
}

/// How far across (u) a point lies at height `v`, solved on whichever axis the quad's edge spans most
fn across(e: Vec2, f: Vec2, g: Vec2, h: Vec2, v: f32) -> Option<f32> {
    let span = e + g * v;
    let u = if span.x.abs() >= span.y.abs() { (h.x - f.x * v) / span.x } else { (h.y - f.y * v) / span.y };
    u.is_finite().then_some(u)
}

/// How far `uv` lies outside the image's 0..1 square (0 inside)
fn outside(uv: Vec2) -> f32 {
    (uv - uv.clamp(Vec2::ZERO, Vec2::ONE)).length_squared()
}

/// Below this, a quad's opposite edges count as parallel (frame sizes squared)
const PARALLEL: f32 = 1e-6;

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
    fn a_collapsed_quad_lands_nowhere_instead_of_failing() {
        let point = [[0.0, 0.0]; 4];
        assert_eq!(warp_uv(&point, Vec2::new(0.3, 0.2)), None);
        assert_eq!(warp_uv(&[[f32::NAN; 2]; 4], Vec2::ZERO), None);
    }
}
