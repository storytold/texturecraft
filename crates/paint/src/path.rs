//! Path tool geometry: a smooth curve through control points and the antialiased coverage of
//! a closed curve (lasso fill). Works in any 2D space (screen pixels or texels).

use texturecraft_geom::glam::Vec2;

/// Most samples per curve segment.
const MAX_SEGMENT_SAMPLES: f32 = 1024.0;

/// A Catmull-Rom spline through `points`, sampled about every `step` units, starting on the first point (closed curves end there too)
pub fn spline(points: &[Vec2], closed: bool, step: f32) -> Vec<Vec2> {
    spline_with_corners(points, &[], closed, step)
}

/// [`spline`] with corner points: the curve turns sharply at a point whose entry in `corners` is true (a missing entry is smooth).
/// Each run between corners is its own Catmull-Rom spline ending on the corners, so two adjacent corners give a straight segment.
pub fn spline_with_corners(points: &[Vec2], corners: &[bool], closed: bool, step: f32) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = points.first().copied().into_iter().collect();
    for segment in segment_samples(points, corners, closed, step) {
        out.extend(segment);
    }
    out
}

/// [`spline`]'s samples per segment: segment `s` holds those after point `s`, up to and including the next point
pub fn spline_segments(points: &[Vec2], closed: bool, step: f32) -> Vec<Vec<Vec2>> {
    spline_segments_with_corners(points, &[], closed, step)
}

/// [`spline_segments`] with corner points as in [`spline_with_corners`]
pub fn spline_segments_with_corners(points: &[Vec2], corners: &[bool], closed: bool, step: f32) -> Vec<Vec<Vec2>> {
    segment_samples(points, corners, closed, step).map(Iterator::collect).collect()
}

/// Stream samples so a flat curve does not allocate and retain every segment alongside its output
fn segment_samples<'a>(points: &'a [Vec2], corners: &'a [bool], closed: bool, step: f32) -> impl Iterator<Item = impl Iterator<Item = Vec2> + 'a> + 'a {
    let n = points.len();
    let wrap = move |i: isize| if closed { i.rem_euclid(n as isize) } else { i.clamp(0, n as isize - 1) };
    let at = move |i: isize| points.get(wrap(i) as usize).copied().unwrap_or_default();
    let corner = move |i: isize| corners.get(wrap(i) as usize).copied().unwrap_or(false);
    let step = if step.is_finite() && step > 1e-6 { step } else { 1.0 };
    let segments = if n < 2 {
        0
    } else if closed {
        n
    } else {
        n - 1
    } as isize;
    (0..segments).map(move |s| {
        let (p1, p2) = (at(s), at(s + 1));

        // A corner ends its run: the neighbour beyond it is replaced by the corner itself
        let p0 = if corner(s) { p1 } else { at(s - 1) };
        let p3 = if corner(s + 1) { p2 } else { at(s + 2) };
        let k = ((p2 - p1).length() / step).ceil().clamp(1.0, MAX_SEGMENT_SAMPLES) as usize;
        (1..=k).map(move |j| {
            let t = j as f32 / k as f32;
            let (t2, t3) = (t * t, t * t * t);
            0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
        })
    })
}

/// A closed polygon's coverage: 1 inside, 0 outside, a one-unit ramp across the edge.
#[derive(Clone, Debug)]
pub struct Lasso {
    poly: Vec<Vec2>,
    lo: Vec2,
    hi: Vec2,
}

impl Lasso {
    /// Fewer than three finite points cover nothing.
    pub fn new(poly: Vec<Vec2>) -> Lasso {
        let poly: Vec<Vec2> = if poly.len() >= 3 && poly.iter().all(|p| p.is_finite()) { poly } else { Vec::new() };
        let lo = poly.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p)) - 1.0;
        let hi = poly.iter().fold(Vec2::splat(f32::MIN), |a, p| a.max(*p)) + 1.0;
        Lasso { poly, lo, hi }
    }

    pub fn coverage(&self, p: Vec2) -> f32 {
        if !(p.x >= self.lo.x && p.y >= self.lo.y && p.x <= self.hi.x && p.y <= self.hi.y) {
            return 0.0;
        }
        let mut inside = false;
        let mut d2 = f32::MAX;
        for (a, b) in self.poly.iter().zip(self.poly.iter().cycle().skip(1)) {
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
            let e = *b - *a;
            let t = ((p - *a).dot(e) / e.length_squared().max(1e-12)).clamp(0.0, 1.0);
            d2 = d2.min((*a + e * t).distance_squared(p));
        }
        let d = d2.sqrt();
        (if inside { 0.5 + d } else { 0.5 - d }).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_passes_through_the_points() {
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 5.0), Vec2::new(20.0, 0.0)];
        let c = spline(&pts, false, 1.0);
        for p in pts {
            assert!(c.iter().any(|q| q.distance(p) < 1e-4), "{p}");
        }
        assert_eq!(c.last().copied(), Some(pts[2]));
        let closed = spline(&pts, true, 1.0);
        assert!(closed.last().is_some_and(|q| q.distance(pts[0]) < 1e-4));
        assert!(closed.len() > c.len());
        assert_eq!(spline(&pts[..1], false, f32::NAN), vec![pts[0]]);
    }

    fn square() -> [Vec2; 4] {
        [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0)]
    }

    /// Distance from `p` to the closed polygon `poly`
    fn edge_distance(poly: &[Vec2], p: Vec2) -> f32 {
        poly.iter()
            .zip(poly.iter().cycle().skip(1))
            .map(|(a, b)| {
                let e = *b - *a;
                (*a + e * ((p - *a).dot(e) / e.length_squared()).clamp(0.0, 1.0)).distance(p)
            })
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn all_corner_closed_path_is_the_straight_edged_quadrilateral() {
        let pts = square();
        let curve = spline_with_corners(&pts, &[true; 4], true, 0.5);
        assert!(curve.len() > 40);
        for p in &curve {
            assert!(edge_distance(&pts, *p) < 1e-3, "{p} left the edges");
        }
        for corner in pts {
            assert!(curve.iter().any(|q| q.distance(corner) < 1e-4), "{corner}");
        }
        let smooth = spline(&pts, true, 0.5);
        assert!(smooth.iter().any(|q| edge_distance(&pts, *q) > 0.5), "the smooth curve bulges off the edges");
    }

    #[test]
    fn mixed_corners_split_the_spline_into_runs() {
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 8.0), Vec2::new(20.0, 0.0), Vec2::new(30.0, 8.0), Vec2::new(40.0, 0.0)];
        let corners = [false, false, true, false, false];
        let mixed = spline_segments_with_corners(&pts, &corners, false, 0.5);
        let smooth = spline_segments(&pts, false, 0.5);

        // The segments next to the corner differ; the corner point itself is still reached
        assert_ne!(mixed[1], smooth[1]);
        assert_ne!(mixed[2], smooth[2]);
        assert_eq!(mixed[1].last().copied(), Some(pts[2]));

        // Neither side of the corner overshoots past it: the turn is a cusp, not a loop
        let before = mixed[1][mixed[1].len() - 2];
        let after = mixed[2][0];
        assert!(before.x < 20.0 && after.x > 20.0);

        // Pure smooth input is unchanged, and a short or missing flag list means smooth
        assert_eq!(spline_segments_with_corners(&pts, &[], false, 0.5), smooth);
        assert_eq!(spline_segments_with_corners(&pts, &[false; 5], false, 0.5), smooth);
        assert_eq!(spline_with_corners(&pts, &[false, false], false, 0.5), spline(&pts, false, 0.5));
    }

    #[test]
    fn lasso_covers_the_inside_with_a_soft_edge() {
        let sq = Lasso::new(vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0)]);
        assert_eq!(sq.coverage(Vec2::new(5.0, 5.0)), 1.0);
        assert_eq!(sq.coverage(Vec2::new(12.0, 5.0)), 0.0);
        assert!((sq.coverage(Vec2::new(10.0, 5.0)) - 0.5).abs() < 1e-4);
        assert_eq!(Lasso::new(vec![Vec2::ZERO, Vec2::ONE]).coverage(Vec2::ZERO), 0.0);
    }
}
