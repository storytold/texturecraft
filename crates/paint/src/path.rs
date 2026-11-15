//! Path tool geometry: a smooth curve through control points and the antialiased coverage of
//! a closed curve (lasso fill), in any 2D space (screen pixels or texels); and curves in the mesh's space
//! ([`SpaceCurve`]): cubic pieces sampled by their length, walked in steps and searched for the point nearest a texel,
//! for strokes laid along a path on the surface (dabs, or a band: [`crate::ribbon`]).

use texturecraft_geom::glam::{Vec2, Vec3};

/// Most samples per curve segment.
const MAX_SEGMENT_SAMPLES: f32 = 1024.0;

/// A Catmull-Rom spline through `points`, sampled about every `step` units, starting on the first point (closed curves end there too)
pub fn spline(points: &[Vec2], closed: bool, step: f32) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = points.first().copied().into_iter().collect();
    for segment in segment_samples(points, closed, step) {
        out.extend(segment);
    }
    out
}

/// [`spline`]'s samples per segment: segment `s` holds those after point `s`, up to and including the next point
pub fn spline_segments(points: &[Vec2], closed: bool, step: f32) -> Vec<Vec<Vec2>> {
    segment_samples(points, closed, step).map(Iterator::collect).collect()
}

/// Stream samples so a flat curve does not allocate and retain every segment alongside its output
fn segment_samples(points: &[Vec2], closed: bool, step: f32) -> impl Iterator<Item = impl Iterator<Item = Vec2>> + '_ {
    let n = points.len();
    let at = move |i: isize| {
        let i = if closed { i.rem_euclid(n as isize) } else { i.clamp(0, n as isize - 1) };
        points.get(i as usize).copied().unwrap_or_default()
    };
    let step = if step.is_finite() && step > 1e-6 { step } else { 1.0 };
    let segments = if n < 2 {
        0
    } else if closed {
        n
    } else {
        n - 1
    } as isize;
    (0..segments).map(move |s| {
        let (p0, p1, p2, p3) = (at(s - 1), at(s), at(s + 1), at(s + 2));
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

/// Samples per cubic piece of a [`SpaceCurve`]: its length is summed over them and its walk steps along them.
pub const SPACE_SAMPLES: usize = 256;
/// Most samples a [`SpaceCurve`] keeps (a guard against hostile point lists).
pub const MAX_SPACE_SAMPLES: usize = 1 << 22;
/// Samples per chunk the nearest-point search is culled by.
const CHUNK: usize = 16;

/// One cubic Bézier piece of a curve in space: from `points[0]` to `points[3]` through the handles `points[1]` and
/// `points[2]`, with the surface's normal and a size (0..1, a pen pressure) at its two ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacePiece {
    pub points: [Vec3; 4],
    pub normals: [Vec3; 2],
    pub size: [f32; 2],
}

impl SpacePiece {
    /// The point at `t` in 0..1.
    pub fn at(&self, t: f32) -> Vec3 {
        let [a, b, c, d] = self.points;
        let s = 1.0 - t;
        a * (s * s * s) + b * (3.0 * s * s * t) + c * (3.0 * s * t * t) + d * (t * t * t)
    }
}

/// Where a point lies by a [`SpaceCurve`]: the arc length of its nearest point on the curve, the offset from there to
/// the point, and the sample the nearest point follows (`index` to `index + 1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveSpot {
    pub length: f32,
    pub offset: Vec3,
    pub index: usize,
}

/// A curve in the mesh's space, sampled [`SPACE_SAMPLES`] times per piece: points, the normal and size the pieces' ends
/// give there (linear between them), and the arc length up to each point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpaceCurve {
    points: Vec<Vec3>,
    normals: Vec<Vec3>,
    sizes: Vec<f32>,
    lengths: Vec<f32>,
}

impl SpaceCurve {
    /// The curve through `pieces`, in order; a curve of no pieces is the single point `alone` (a path of one anchor),
    /// if given. Points that are no numbers are dropped.
    pub fn new(pieces: &[SpacePiece], alone: Option<(Vec3, Vec3, f32)>) -> SpaceCurve {
        let mut c = SpaceCurve::default();
        let mut push = |p: Vec3, n: Vec3, size: f32| {
            if p.is_finite() && c.points.len() < MAX_SPACE_SAMPLES {
                c.points.push(p);
                c.normals.push(n.try_normalize().unwrap_or(Vec3::Z));
                c.sizes.push(if size.is_finite() { size.clamp(0.0, 1.0) } else { 1.0 });
            }
        };
        for (k, piece) in pieces.iter().enumerate() {
            for i in usize::from(k > 0)..=SPACE_SAMPLES {
                let t = i as f32 / SPACE_SAMPLES as f32;
                push(piece.at(t), piece.normals[0].lerp(piece.normals[1], t), piece.size[0] + (piece.size[1] - piece.size[0]) * t);
            }
        }
        if pieces.is_empty()
            && let Some((p, n, size)) = alone
        {
            push(p, n, size);
        }
        let mut total = 0.0;
        let mut last: Option<Vec3> = None;
        for p in &c.points {
            total += last.map_or(0.0, |l| (*p - l).length());
            c.lengths.push(total);
            last = Some(*p);
        }
        c
    }

    /// The curve's length.
    pub fn length(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// The samples.
    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    /// The normal the pieces' ends give at sample `i`.
    pub fn normal(&self, i: usize) -> Option<Vec3> {
        self.normals.get(i).copied()
    }

    /// The curve's direction after sample `i` (towards the next one).
    pub fn direction(&self, i: usize) -> Option<Vec3> {
        (*self.points.get(i + 1)? - *self.points.get(i)?).try_normalize()
    }

    /// Walks the curve by its length: calls `spot(point, direction, size)` at the first point and then every distance
    /// it returns further along, never past the end (no spot is forced onto it); at most `most` spots. A step that is
    /// no positive number is taken as a tiny one.
    pub fn walk(&self, most: usize, mut spot: impl FnMut(Vec3, Vec3, f32) -> f32) {
        let (Some(&first), Some(&size)) = (self.points.first(), self.sizes.first()) else { return };
        if most == 0 {
            return;
        }
        let heading = self.points.windows(2).find_map(|w| (w[1] - w[0]).try_normalize()).unwrap_or(Vec3::X);
        let step = |v: f32| if v > 1e-6 { v } else { 1e-6 };
        let mut left = step(spot(first, heading, size));
        let mut count = 1usize;
        for (i, w) in self.points.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let (sa, sb) = (self.sizes.get(i).copied().unwrap_or(1.0), self.sizes.get(i + 1).copied().unwrap_or(1.0));
            let l = (b - a).length();
            let Some(u) = (b - a).try_normalize() else { continue };
            let mut t = 0.0;
            while l - t >= left {
                t += left;
                if count >= most {
                    return;
                }
                count += 1;
                left = step(spot(a + u * t, u, sa + (sb - sa) * (t / l)));
            }
            left -= l - t;
        }
    }

    /// The nearest point of the curve to each of many points within `reach`: a searcher that culls the curve's samples
    /// by chunks.
    pub fn searcher(&self, reach: f32) -> CurveSearch<'_> {
        let n = self.points.len();
        let reach = if reach.is_finite() { reach.max(0.0) } else { 0.0 };
        let mut chunks = Vec::new();
        let mut a = 0;
        while a + 1 < n {
            let b = (a + CHUNK).min(n - 1);
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for p in self.points.get(a..=b).unwrap_or_default() {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            chunks.push((a, b, lo - Vec3::splat(reach), hi + Vec3::splat(reach)));
            a = b;
        }
        let (lo, hi) = chunks.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), c| (lo.min(c.2), hi.max(c.3)));
        CurveSearch { curve: self, chunks, lo, hi, reach }
    }
}

/// [`SpaceCurve::searcher`]'s culled search.
pub struct CurveSearch<'a> {
    curve: &'a SpaceCurve,
    chunks: Vec<(usize, usize, Vec3, Vec3)>,
    lo: Vec3,
    hi: Vec3,
    reach: f32,
}

impl CurveSearch<'_> {
    /// The box holding every point within reach of the curve.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        (self.lo, self.hi)
    }

    /// The point of the curve nearest `p` by the straight distance, if within reach.
    pub fn nearest(&self, p: Vec3) -> Option<CurveSpot> {
        let inside = |lo: Vec3, hi: Vec3| p.cmpge(lo).all() && p.cmple(hi).all();
        if !inside(self.lo, self.hi) {
            return None;
        }
        let (points, lengths) = (&self.curve.points, &self.curve.lengths);
        let mut best: Option<(f32, CurveSpot)> = None;
        for &(a, b, lo, hi) in &self.chunks {
            if !inside(lo, hi) {
                continue;
            }
            for i in a..b {
                let (Some(&q0), Some(&q1), Some(&l0), Some(&l1)) = (points.get(i), points.get(i + 1), lengths.get(i), lengths.get(i + 1)) else { continue };
                let d = q1 - q0;
                let l2 = d.length_squared();
                let t = if l2 > 0.0 { ((p - q0).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let offset = p - (q0 + d * t);
                let d2 = offset.length_squared();
                if best.is_none_or(|(b, _)| d2 < b) {
                    best = Some((d2, CurveSpot { length: l0 + (l1 - l0) * t, offset, index: i }));
                }
            }
        }
        best.filter(|(d2, _)| *d2 <= self.reach * self.reach * 1.0001).map(|(_, spot)| spot)
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

    #[test]
    fn lasso_covers_the_inside_with_a_soft_edge() {
        let sq = Lasso::new(vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0)]);
        assert_eq!(sq.coverage(Vec2::new(5.0, 5.0)), 1.0);
        assert_eq!(sq.coverage(Vec2::new(12.0, 5.0)), 0.0);
        assert!((sq.coverage(Vec2::new(10.0, 5.0)) - 0.5).abs() < 1e-4);
        assert_eq!(Lasso::new(vec![Vec2::ZERO, Vec2::ONE]).coverage(Vec2::ZERO), 0.0);
    }

    fn straight(from: Vec3, to: Vec3) -> SpacePiece {
        SpacePiece { points: [from, from.lerp(to, 1.0 / 3.0), from.lerp(to, 2.0 / 3.0), to], normals: [Vec3::Z; 2], size: [1.0, 0.5] }
    }

    #[test]
    fn space_curves_walk_by_their_length_and_find_the_nearest_point() {
        let c = SpaceCurve::new(&[straight(Vec3::ZERO, Vec3::X), straight(Vec3::X, Vec3::new(1.0, 1.0, 0.0))], None);
        assert!((c.length() - 2.0).abs() < 1e-4);
        assert_eq!(c.points().len(), 2 * SPACE_SAMPLES + 1);
        // steps of 0.3: the first spot on the start, none forced on the end, sizes linear along each piece
        let mut spots = Vec::new();
        c.walk(100, |p, u, size| {
            spots.push((p, u, size));
            0.3
        });
        assert_eq!(spots.len(), 7);
        assert!(spots[0].0.length() < 1e-6 && spots[0].1.abs_diff_eq(Vec3::X, 1e-5) && spots[0].2 == 1.0);
        assert!(spots[3].0.abs_diff_eq(Vec3::new(0.9, 0.0, 0.0), 1e-4) && (spots[3].2 - 0.55).abs() < 1e-3);
        assert!(spots[4].0.abs_diff_eq(Vec3::new(1.0, 0.2, 0.0), 1e-4) && spots[4].1.abs_diff_eq(Vec3::Y, 1e-4));
        let mut n = 0;
        c.walk(3, |_, _, _| {
            n += 1;
            0.0
        });
        assert_eq!(n, 3, "the walk stops at its most");
        // nearest points: arc length and offset, nothing out of reach
        let search = c.searcher(0.25);
        let spot = search.nearest(Vec3::new(0.5, -0.2, 0.0)).unwrap();
        assert!((spot.length - 0.5).abs() < 1e-4 && spot.offset.abs_diff_eq(Vec3::new(0.0, -0.2, 0.0), 1e-5));
        let spot = search.nearest(Vec3::new(1.1, 0.75, 0.0)).unwrap();
        assert!((spot.length - 1.75).abs() < 1e-4);
        assert_eq!(search.nearest(Vec3::new(0.5, 0.5, 0.0)), None);
        // one anchor: one point; nothing: no walk
        let one = SpaceCurve::new(&[], Some((Vec3::ONE, Vec3::Z, 1.0)));
        let mut n = 0;
        one.walk(10, |_, _, _| {
            n += 1;
            1.0
        });
        assert_eq!((n, one.length()), (1, 0.0));
        SpaceCurve::new(&[], None).walk(10, |_, _, _| panic!("no point"));
        assert_eq!(SpaceCurve::new(&[], None).searcher(1.0).nearest(Vec3::ZERO), None);
    }
}
