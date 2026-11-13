//! Ribbons: a brush's shape laid along a curve on the surface as one band, instead of stamps (path tools such as
//! dashed lines, panels and tape). The band is [`Ribbon::width`] wide, centred on the curve; the shape's rows run across
//! it and its columns along it, placed by [`RibbonTiling`]. A texel takes the shape at its nearest point of the curve
//! (the straight distance in space): along by that point's arc length, across by its offset to the side of the curve,
//! and it is on the band while its offset along the surface's normal stays within [`Ribbon::depth`].

use rayon::prelude::*;
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer};

use crate::brush::Shape;
use crate::path::SpaceCurve;
use crate::raster::StrokeCanvas;

/// How the shape is placed along the band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RibbonTiling {
    /// Copies [`Ribbon::copy`] long, as many whole ones as fit best (at least one), stretched to fill the curve exactly;
    /// the first starts on the curve's start.
    Repeat,
    /// One copy stretched over the whole curve.
    Whole,
    /// One copy over the whole curve with its ends kept: the shape before `guides[0]` and after `guides[1]` (shares of
    /// its length) laid [`Ribbon::copy`] long, the part between them stretched; a curve shorter than its two ends
    /// takes the whole shape squeezed.
    Caps { guides: [f32; 2] },
}

/// A band along a curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ribbon {
    /// Full width across the curve, in the mesh's units.
    pub width: f32,
    /// Length of one copy of the shape along the curve before fitting, in the mesh's units.
    pub copy: f32,
    pub tiling: RibbonTiling,
    /// How far from the curve along the surface's normal a texel may lie and still be on the band.
    pub depth: f32,
}

impl Ribbon {
    /// Where the shape is read at arc length `s` of a curve `total` long: its column, 0..1 (`None` off the curve).
    pub fn along(&self, s: f32, total: f32) -> Option<f32> {
        if !(total > 0.0 && (0.0..=total).contains(&s)) {
            return None;
        }
        let copy = if self.copy.is_finite() { self.copy.max(1e-6) } else { total };
        Some(match self.tiling {
            RibbonTiling::Whole => s / total,
            RibbonTiling::Caps { guides: [g0, g1] } => {
                let (g0, g1) = (g0.clamp(0.0, 1.0), g1.clamp(0.0, 1.0).max(g0.clamp(0.0, 1.0)));
                let (head, tail) = (g0 * copy, (1.0 - g1) * copy);
                if head + tail >= total {
                    s / total
                } else if s < head {
                    s / copy
                } else if s > total - tail {
                    1.0 - (total - s) / copy
                } else {
                    g0 + (g1 - g0) * (s - head) / (total - head - tail)
                }
            }
            RibbonTiling::Repeat => {
                let n = (total / copy).round().clamp(1.0, 1e6);
                let u = s / (total / n);
                u - u.floor()
            }
        })
    }
}

/// Lays `ribbon` along `curve` into `canvas` with `shape` (sampled with `hardness`, one coverage per texel, "over" what
/// the canvas holds): every texel of the set in `gbuf` on the band. A curve of fewer than two points or no length lays
/// nothing.
pub fn rasterize_ribbon(canvas: &mut StrokeCanvas, mesh: &Mesh, gbuf: &UvGBuffer, curve: &SpaceCurve, ribbon: &Ribbon, shape: Shape, hardness: f32) {
    rasterize_ribbon_at(canvas, mesh, gbuf, curve, ribbon, shape, hardness);
}

/// [`rasterize_ribbon`], returning every texel laid with its place on the band: the shape's column along it and its row
/// across it (0..1, row 0 on the curve's left), where the shape is read. What a band lays along it (a material painted
/// along a path) is read at that place.
pub fn rasterize_ribbon_at(
    canvas: &mut StrokeCanvas,
    mesh: &Mesh,
    gbuf: &UvGBuffer,
    curve: &SpaceCurve,
    ribbon: &Ribbon,
    shape: Shape,
    hardness: f32,
) -> Vec<(u32, u32, [f32; 2])> {
    let texels = ribbon_texels(mesh, gbuf, curve, ribbon, shape, hardness);
    let places = texels.iter().map(|&(x, y, _, at)| (x, y, at)).collect();
    canvas.add_texels(gbuf, texels.into_iter().map(|(x, y, a, _)| (x, y, a)).collect());
    places
}

/// The texels a band lays: coverage and place on the band (see [`rasterize_ribbon_at`]).
fn ribbon_texels(mesh: &Mesh, gbuf: &UvGBuffer, curve: &SpaceCurve, ribbon: &Ribbon, shape: Shape, hardness: f32) -> Vec<(u32, u32, f32, [f32; 2])> {
    let total = curve.length();
    let half = ribbon.width * 0.5;
    let positive = |v: f32| v > 0.0 && v.is_finite();
    if curve.points().len() < 2 || !positive(total) || !positive(half) {
        return Vec::new();
    }
    let depth = if ribbon.depth.is_finite() { ribbon.depth.max(half) } else { half };
    let search = curve.searcher(depth);
    let (lo, hi) = search.bounds();
    let w = gbuf.width.max(1) as usize;
    let last = curve.points().len() - 2;
    let position = |i: usize| -> Option<Vec3> {
        let t = *gbuf.tri.get(i)?;
        if t == NO_TRI {
            return None;
        }
        let [b1, b2] = *gbuf.bary.get(i)?;
        let [a, b, c] = mesh.tri(t as usize)?;
        Some(*mesh.positions.get(a)? * (1.0 - b1 - b2) + *mesh.positions.get(b)? * b1 + *mesh.positions.get(c)? * b2)
    };
    (0..gbuf.tri.len())
        .into_par_iter()
        .filter_map(|i| {
            let p = position(i)?;
            if !(p.cmpge(lo).all() && p.cmple(hi).all()) {
                return None;
            }
            let spot = search.nearest(p)?;
            // across: the curve's direction turned on the surface; the rest of the offset is depth
            let dir = curve.direction(spot.index)?;
            let side = dir.cross(curve.normal(spot.index)?).try_normalize()?;
            let normal = side.cross(dir).try_normalize()?;
            let (wide, deep) = (spot.offset.dot(side), spot.offset.dot(normal));
            if wide.abs() > half * 1.0001 || deep.abs() > depth {
                return None;
            }
            // beyond an end is off the band
            let ahead = spot.offset.dot(dir);
            if (spot.index == 0 && spot.length <= 0.0 && ahead < -1e-6) || (spot.index == last && spot.length >= total && ahead > 1e-6) {
                return None;
            }
            let u = ribbon.along(spot.length, total)?;
            let v = 0.5 + wide / ribbon.width;
            // the shape's square: x along, y up with its rows across (row 0 on the curve's left)
            let a = shape.sample(2.0 * u - 1.0, 1.0 - 2.0 * v, hardness, 0).min(1.0);
            (a > 0.0).then_some(((i % w) as u32, (i / w) as u32, a, [u, v]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BrushAlpha;
    use crate::alphas::AlphaImage;
    use crate::path::SpacePiece;
    use texturecraft_geom::primitives;
    use texturecraft_texel::from_u16;

    fn ribbon(tiling: RibbonTiling) -> Ribbon {
        Ribbon { width: 0.1, copy: 0.1, tiling, depth: 0.075 }
    }

    #[test]
    fn repeated_shapes_fit_whole_copies_and_capped_ones_keep_their_ends() {
        // 21 copies on a curve 20.97 copies long, the first on the start
        let r = ribbon(RibbonTiling::Repeat);
        let total = 2.0969;
        let copy = total / 21.0;
        assert!(r.along(0.0, total).unwrap().abs() < 1e-6);
        assert!((r.along(copy * 0.5, total).unwrap() - 0.5).abs() < 1e-4);
        assert!((r.along(copy * 20.5, total).unwrap() - 0.5).abs() < 1e-3);
        assert_eq!(r.along(-0.01, total), None);
        assert_eq!(r.along(0.5, 0.0), None);
        // caps: each half of the shape one copy (half a unit... here 0.05) long at its end, the middle its centre column
        let s = ribbon(RibbonTiling::Caps { guides: [0.5, 0.5] });
        assert!((s.along(0.025, 1.0).unwrap() - 0.25).abs() < 1e-5);
        assert!((s.along(0.5, 1.0).unwrap() - 0.5).abs() < 1e-5);
        assert!((s.along(0.975, 1.0).unwrap() - 0.75).abs() < 1e-5);
        assert!((s.along(0.04, 0.08).unwrap() - 0.5).abs() < 1e-5, "shorter than its ends: squeezed whole");
        assert!((ribbon(RibbonTiling::Whole).along(0.3, 1.2).unwrap() - 0.25).abs() < 1e-5);
    }

    /// A 2 × 2 sheet at z = 0 facing +z, texture rows down its −y.
    fn sheet() -> Mesh {
        let mut m = primitives::plane(2.0, 8);
        for p in &mut m.positions {
            *p = Vec3::new(p.x, -p.z, 0.0);
        }
        m.normals.fill(Vec3::Z);
        m
    }

    #[test]
    fn a_band_lays_the_shape_along_the_curve() {
        let m = sheet();
        let g = UvGBuffer::new(&m, 0, 128, 128);
        let at = |x: f32| Vec3::new(x, 0.0, 0.0);
        let curve = SpaceCurve::new(&[SpacePiece { points: [at(-0.5), at(-1.0 / 6.0), at(1.0 / 6.0), at(0.5)], normals: [Vec3::Z; 2], size: [1.0; 2] }], None);
        let wide = Ribbon { width: 0.25, copy: 0.25, tiling: RibbonTiling::Whole, depth: 0.2 };
        // the shape: its left half (the start of the curve) white, its right half black
        let half = AlphaImage::from_pixels(2, 1, &[1.0, 0.0]).unwrap();
        let shape = Shape { alpha: BrushAlpha::Round, image: None, library: Some(&half), level: 0 };
        let mut canvas = StrokeCanvas::new(128, 128);
        rasterize_ribbon(&mut canvas, &m, &g, &curve, &wide, shape, 0.5);
        let a = |x: u32, y: u32| from_u16(canvas.alpha.get(x, y)[0]);
        // the sheet's centre is texel 64; the band runs x 32..96, y 56..72 (0.125 = 8 texels either side)
        assert!(a(40, 64) > 0.99 && a(40, 58) > 0.99 && a(40, 70) > 0.99);
        assert!(a(88, 64) < 0.01, "the shape's black half lies on the curve's second half");
        assert_eq!(a(40, 50), 0.0, "off the band's side");
        assert_eq!(a(20, 64), 0.0, "before the curve's start");
        assert!(!canvas.dirty.is_empty());
        // a round tip across the band: full on the curve, nothing at its sides' ends
        let round = Shape { alpha: BrushAlpha::Round, image: None, library: None, level: 0 };
        let mut c2 = StrokeCanvas::new(128, 128);
        rasterize_ribbon(&mut c2, &m, &g, &curve, &wide, round, 0.5);
        let b = |x: u32, y: u32| from_u16(c2.alpha.get(x, y)[0]);
        assert!(b(64, 64) > 0.99 && b(64, 71) < b(64, 64) && b(34, 64) < 0.2);
        // a curve too far above the sheet (deeper than the band reaches) lays nothing; no curve lays nothing
        let high = SpaceCurve::new(
            &[SpacePiece {
                points: [at(-0.5) + Vec3::Z * 0.3, at(0.0) + Vec3::Z * 0.3, at(0.1) + Vec3::Z * 0.3, at(0.5) + Vec3::Z * 0.3],
                normals: [Vec3::Z; 2],
                size: [1.0; 2],
            }],
            None,
        );
        let mut c3 = StrokeCanvas::new(128, 128);
        rasterize_ribbon(&mut c3, &m, &g, &high, &wide, round, 0.5);
        rasterize_ribbon(&mut c3, &m, &g, &SpaceCurve::new(&[], Some((Vec3::ZERO, Vec3::Z, 1.0))), &wide, round, 0.5);
        assert!(c3.dirty.is_empty());
    }

    #[test]
    fn band_texels_carry_their_place_on_the_band() {
        let m = sheet();
        let g = UvGBuffer::new(&m, 0, 128, 128);
        let at = |x: f32| Vec3::new(x, 0.0, 0.0);
        let curve = SpaceCurve::new(&[SpacePiece { points: [at(-0.5), at(-1.0 / 6.0), at(1.0 / 6.0), at(0.5)], normals: [Vec3::Z; 2], size: [1.0; 2] }], None);
        let wide = Ribbon { width: 0.25, copy: 0.25, tiling: RibbonTiling::Whole, depth: 0.2 };
        let round = Shape { alpha: BrushAlpha::Round, image: None, library: None, level: 0 };
        let mut canvas = StrokeCanvas::new(128, 128);
        let texels = rasterize_ribbon_at(&mut canvas, &m, &g, &curve, &wide, round, 0.5);
        let place = |x: u32, y: u32| texels.iter().find(|t| t.0 == x && t.1 == y).map(|t| t.2);
        // along: the share of the curve walked (the sheet's texel 64 is its middle); across: row 0 on the curve's left
        let mid = place(64, 64).unwrap();
        assert!((mid[0] - 0.5).abs() < 0.02 && (mid[1] - 0.5).abs() < 0.07, "{mid:?}");
        let early = place(40, 64).unwrap();
        assert!((early[0] - 0.125).abs() < 0.02, "{early:?}");
        // texture rows run down the sheet's −y: the curve's left (+y, its direction +x turned on the +z surface) is up
        let (up, down) = (place(64, 58).unwrap(), place(64, 70).unwrap());
        assert!(up[1] < 0.3 && down[1] > 0.7, "{up:?} {down:?}");
        // every texel laid has its place, and only those
        let mut plain = StrokeCanvas::new(128, 128);
        rasterize_ribbon(&mut plain, &m, &g, &curve, &wide, round, 0.5);
        let laid = (0..128u32).flat_map(|y| (0..128u32).map(move |x| (x, y))).filter(|&(x, y)| plain.alpha.get(x, y)[0] > 0).count();
        assert_eq!(laid, texels.len());
        assert_eq!(plain.alpha.get(64, 64), canvas.alpha.get(64, 64));
    }
}
