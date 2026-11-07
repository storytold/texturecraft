//! Stamps → stroke alpha in texture space. Every texel whose surface point lies inside a
//! stamp's oriented volume receives paint, so UV seams are seamless by construction.

use std::collections::BTreeSet;

use rayon::prelude::*;

use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Bvh, Mesh, NO_TRI, UvGBuffer};
use texturecraft_texel::{TILE, TileImage, from_u16, to_u16};

use crate::brush::Shape;
use crate::pickup::Pickup;
use crate::stroke::Stamp;
use crate::visibility::EyeDepth;

/// The stroke alpha being accumulated for one texture set.
#[derive(Clone, Debug)]
pub struct StrokeCanvas {
    pub alpha: TileImage,
    /// Every tile the stroke touched.
    pub dirty: BTreeSet<(u32, u32)>,
    /// Tiles touched since the caller last cleared this set (incremental preview updates).
    pub recent: BTreeSet<(u32, u32)>,
    /// Quick mask: paint only where this coverage image (1 component) is set; inverted when
    /// the flag is true.
    pub limit: Option<(TileImage, bool)>,
    /// Clone and smudge: the channel values the stroke carries.
    pub pickup: Option<Pickup>,
    /// The stamp being added: `(x, y, alpha)` per texel (reused between stamps).
    texels: Vec<(u32, u32, f32)>,
}

impl StrokeCanvas {
    pub fn new(width: u32, height: u32) -> StrokeCanvas {
        StrokeCanvas { alpha: TileImage::new(width, height, 1), dirty: BTreeSet::new(), recent: BTreeSet::new(), limit: None, pickup: None, texels: Vec::new() }
    }

    /// Start over when the canvas is not the size of `gbuf`.
    fn fit(&mut self, gbuf: &UvGBuffer) {
        if (self.alpha.width(), self.alpha.height()) != (gbuf.width, gbuf.height) {
            *self = StrokeCanvas::new(gbuf.width, gbuf.height);
        }
    }

    fn touch(&mut self, x: u32, y: u32) {
        let t = (x / TILE, y / TILE);
        self.dirty.insert(t);
        self.recent.insert(t);
    }

    /// Blend the gathered stamp texels (stamp center `center` in texels) into the stroke: the
    /// quick mask scales them, then the pickup deposits its values or the alpha accumulates
    /// ("over": `m = a + m·(1 − a)`).
    fn add_stamp(&mut self, center: [f32; 2]) {
        let mut texels = std::mem::take(&mut self.texels);
        if let Some((mask, invert)) = &self.limit {
            for t in &mut texels {
                let m = from_u16(mask.get(t.0, t.1)[0]);
                t.2 *= if *invert { 1.0 - m } else { m };
            }
        }
        let painted = match &mut self.pickup {
            Some(p) => p.stamp(&mut self.alpha, &texels, center),
            None => {
                for &(x, y, a) in &texels {
                    let m = from_u16(self.alpha.get(x, y)[0]);
                    self.alpha.set(x, y, &[to_u16(a + m * (1.0 - a))]);
                }
                true
            }
        };
        if painted {
            for &(x, y, a) in &texels {
                if a > 0.0 {
                    self.touch(x, y);
                }
            }
        }
        texels.clear();
        self.texels = texels;
    }
}

/// What the rasterizer reads.
pub struct RasterCtx<'a> {
    pub mesh: &'a Mesh,
    pub bvh: &'a Bvh,
    pub gbuf: &'a UvGBuffer,
    pub material: usize,
    /// Camera visibility (None paints hidden surfaces too, e.g. for scripted strokes).
    pub depth: Option<&'a EyeDepth>,
    pub eye: Vec3,
    pub backface_culling: bool,
    /// Angle fade from/to in degrees.
    pub angle_fade: (f32, f32),
    pub shape: Shape<'a>,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much a surface seen at `facing` (cosine between its normal and the direction to the eye)
/// is painted: 0 for back faces when `culling`, and a smooth fade between the angle-fade start
/// and end (degrees from the view direction) so grazing surfaces do not stretch the paint.
/// Without culling every surface counts in full.
pub fn view_weight(facing: f32, culling: bool, angle_fade: (f32, f32)) -> f32 {
    if !culling {
        return 1.0;
    }
    if facing.is_nan() || facing <= 0.0 {
        return 0.0;
    }
    smoothstep(angle_fade.1.to_radians().cos(), angle_fade.0.to_radians().cos(), facing)
}

/// Accumulate `stamps` into `canvas` ("over": `m = a + m·(1 − a)`).
pub fn rasterize(canvas: &mut StrokeCanvas, ctx: &RasterCtx, stamps: &[Stamp]) {
    let (w, h) = (ctx.gbuf.width, ctx.gbuf.height);
    canvas.fit(ctx.gbuf);
    let fade_lo = ctx.angle_fade.1.to_radians().cos();
    let fade_hi = ctx.angle_fade.0.to_radians().cos();
    let mut tris = Vec::new();
    for s in stamps {
        if s.radius.is_nan() || s.radius <= 0.0 || !s.center.is_finite() {
            continue;
        }
        let ext = Vec3::splat(s.radius * s.depth.max(1.0) * 1.42);
        tris.clear();
        ctx.bvh.query_box(s.center - ext, s.center + ext, |t| tris.push(t));
        // Each texel belongs to one triangle, so triangles scan independently (in parallel for
        // large stamps); stamp alphas combine commutatively, so the order does not matter.
        let scan = |&t: &u32| scan_triangle(ctx, s, t, w, h, fade_lo, fade_hi);
        let parts: Vec<TriTexels> = if tris.len() > 16 { tris.par_iter().filter_map(scan).collect() } else { tris.iter().filter_map(scan).collect() };
        // The stamp center in texels: the footprint texel nearest to it on the surface.
        let mut center = ([0.0f32; 2], f32::MAX);
        for (texels, c) in parts {
            if c.1 < center.1 {
                center = c;
            }
            canvas.texels.extend(texels);
        }
        canvas.add_stamp(center.0);
    }
}

/// Texels of one triangle a stamp covers, and the one nearest the stamp center.
type TriTexels = (Vec<(u32, u32, f32)>, ([f32; 2], f32));

#[allow(clippy::too_many_arguments)]
fn scan_triangle(ctx: &RasterCtx, s: &Stamp, t: u32, w: u32, h: u32, fade_lo: f32, fade_hi: f32) -> Option<TriTexels> {
    if ctx.mesh.material_of(t as usize) != Some(ctx.material) {
        return None;
    }
    let [a, b, c] = ctx.mesh.tri(t as usize)?;
    let (ua, ub, uc) = (ctx.mesh.uvs.get(a)?, ctx.mesh.uvs.get(b)?, ctx.mesh.uvs.get(c)?);
    let (pa, pb, pc) = (ctx.mesh.positions.get(a)?, ctx.mesh.positions.get(b)?, ctx.mesh.positions.get(c)?);
    let (na, nb, nc) = (ctx.mesh.normals.get(a)?, ctx.mesh.normals.get(b)?, ctx.mesh.normals.get(c)?);
    let x0 = (ua.x.min(ub.x).min(uc.x) * w as f32).floor().max(0.0) as u32;
    let x1 = ((ua.x.max(ub.x).max(uc.x) * w as f32).ceil().max(0.0) as u32).min(w);
    let y0 = (ua.y.min(ub.y).min(uc.y) * h as f32).floor().max(0.0) as u32;
    let y1 = ((ua.y.max(ub.y).max(uc.y) * h as f32).ceil().max(0.0) as u32).min(h);
    // Stamp radius in texels on this triangle: texel area over world area.
    let uv_area = (*ub - *ua).perp_dot(*uc - *ua).abs() * w as f32 * h as f32;
    let world_area = (*pb - *pa).cross(*pc - *pa).length();
    let shape = ctx.shape.at_radius(s.radius * (uv_area / world_area).sqrt());
    let mut texels = Vec::new();
    let mut center = ([0.0f32; 2], f32::MAX);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) as usize;
            if ctx.gbuf.tri.get(i).copied().unwrap_or(NO_TRI) != t {
                continue;
            }
            let Some([b1, b2]) = ctx.gbuf.bary.get(i).copied() else { continue };
            let b0 = 1.0 - b1 - b2;
            let p = *pa * b0 + *pb * b1 + *pc * b2;
            let d = p - s.center;
            let lx = d.dot(s.tangent) / s.radius;
            let ly = d.dot(s.bitangent) / s.radius;
            if lx * lx + ly * ly > 2.0 {
                continue;
            }
            if d.dot(s.normal).abs() > s.radius * s.depth {
                continue;
            }
            let dist = d.length_squared();
            if dist < center.1 {
                center = ([x as f32 + 0.5, y as f32 + 0.5], dist);
            }
            let n = (*na * b0 + *nb * b1 + *nc * b2).try_normalize().unwrap_or(s.normal);
            let to_eye = (ctx.eye - p).try_normalize().unwrap_or(n);
            let facing = n.dot(to_eye);
            // Symmetry copies paint where the camera may not see (the back of the model).
            let culled = ctx.backface_culling && !s.copy;
            if culled && facing <= 0.0 {
                continue;
            }
            if n.dot(s.normal) <= 0.0 {
                continue;
            }
            if let Some(depth) = ctx.depth.filter(|_| !s.copy)
                && !depth.visible(p)
            {
                continue;
            }
            let fade = if culled { smoothstep(fade_lo, fade_hi, facing) } else { 1.0 };
            let a = shape.sample(lx, ly, s.hardness, s.seed) * s.flow * fade;
            if a > 0.0 {
                texels.push((x, y, a));
            }
        }
    }
    (center.1 < f32::MAX || !texels.is_empty()).then_some((texels, center))
}

/// Stroke alpha covering whole triangles (polygon fill): every texel of `tris` gets alpha 1.
pub fn fill_triangles(canvas: &mut StrokeCanvas, gbuf: &UvGBuffer, tris: &BTreeSet<u32>) {
    fill_coverage(canvas, gbuf, |_, t| if tris.contains(&t) { 1.0 } else { 0.0 });
}

/// One-shot fill of the texture set's texels: `coverage(texel index, triangle)` gives each
/// covered texel's alpha (lasso and polygon fills).
pub fn fill_coverage(canvas: &mut StrokeCanvas, gbuf: &UvGBuffer, coverage: impl Fn(usize, u32) -> f32 + Sync) {
    let w = gbuf.width.max(1);
    canvas.fit(gbuf);
    canvas.texels = gbuf
        .tri
        .par_iter()
        .enumerate()
        .filter(|(_, t)| **t != NO_TRI)
        .filter_map(|(i, t)| {
            let a = coverage(i, *t).min(1.0);
            (a > 0.0).then_some(((i as u32) % w, (i as u32) / w, a))
        })
        .collect();
    canvas.add_stamp([0.0; 2]);
}

/// What [`project_coverage`] reads: the surface, the camera's view of it and the fade settings.
pub struct ProjectCtx<'a> {
    pub mesh: &'a Mesh,
    pub gbuf: &'a UvGBuffer,
    /// Camera visibility (None projects onto hidden surfaces too).
    pub depth: Option<&'a EyeDepth>,
    pub eye: Vec3,
    pub backface_culling: bool,
    /// Angle fade from/to in degrees.
    pub angle_fade: (f32, f32),
}

/// One-shot "project the whole image": every texel the camera sees through the stencil gets the
/// alpha of its [`view_weight`] (back faces, hidden surfaces and texels outside the image get 0).
/// The stencil's own color and alpha are applied when the canvas is committed with
/// `per_texel`, so this is only the surface part of the weight.
pub fn project_coverage(canvas: &mut StrokeCanvas, ctx: &ProjectCtx, stencil: &texturecraft_compose::Stencil) {
    fill_coverage(canvas, ctx.gbuf, |i, _| {
        let Some(s) = ctx.gbuf.surface(ctx.mesh, i) else { return 0.0 };
        if stencil.sample_world(s.pos).is_none() || ctx.depth.is_some_and(|d| !d.visible(s.pos)) {
            return 0.0;
        }
        let to_eye = (ctx.eye - s.pos).try_normalize().unwrap_or(s.nrm);
        view_weight(s.nrm.dot(to_eye), ctx.backface_culling, ctx.angle_fade)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::primitives;

    #[test]
    fn one_stamp_on_a_plane_paints_a_disc() {
        let m = primitives::plane(2.0, 8);
        let bvh = Bvh::new(&m);
        let g = UvGBuffer::new(&m, 0, 128, 128);
        let mut canvas = StrokeCanvas::new(128, 128);
        let ctx = RasterCtx {
            mesh: &m,
            bvh: &bvh,
            gbuf: &g,
            material: 0,
            depth: None,
            eye: Vec3::new(0.0, 5.0, 0.0),
            backface_culling: true,
            angle_fade: (70.0, 85.0),
            shape: Shape { alpha: crate::BrushAlpha::Round, image: None, library: None, level: 0 },
        };
        let st = Stamp {
            center: Vec3::ZERO,
            normal: Vec3::Y,
            tangent: Vec3::X,
            bitangent: Vec3::Z,
            radius: 0.25,
            flow: 1.0,
            hardness: 0.9,
            depth: 1.0,
            seed: 0,
            copy: false,
        };
        rasterize(&mut canvas, &ctx, &[st]);
        // Plane is 2 units over 128 texels: radius 0.25 = 16 texels around the center (64, 64).
        let at = |x: u32, y: u32| from_u16(canvas.alpha.get(x, y)[0]);
        assert!(at(64, 64) > 0.99);
        assert!(at(64 + 12, 64) > 0.9);
        assert!(at(64 + 20, 64) < 0.01);
        assert!(!canvas.dirty.is_empty());

        // From below (backface), nothing is painted.
        let mut c2 = StrokeCanvas::new(128, 128);
        let below = RasterCtx { eye: Vec3::new(0.0, -5.0, 0.0), ..ctx };
        rasterize(&mut c2, &below, &[st]);
        assert!(c2.dirty.is_empty());
    }

    #[test]
    fn view_weight_fades_towards_grazing_angles() {
        let fade = (70.0, 85.0);
        let w = |deg: f32| view_weight(deg.to_radians().cos(), true, fade);
        assert_eq!(w(0.0), 1.0);
        assert_eq!(w(70.0), 1.0, "full strength up to the fade start");
        assert!(w(78.0) > 0.05 && w(78.0) < 0.95, "partway: {}", w(78.0));
        assert!(w(60.0) > w(78.0) && w(78.0) > w(84.0), "monotonic");
        assert!(w(89.9) < 0.01, "grazing paints nothing");
        // Back faces and NaN never paint with culling; without culling everything counts.
        assert_eq!(view_weight(-0.5, true, fade), 0.0);
        assert_eq!(view_weight(f32::NAN, true, fade), 0.0);
        assert_eq!(view_weight(-0.5, false, fade), 1.0);
    }

    fn projected_plane(eye: Vec3, culling: bool, depth: Option<&EyeDepth>) -> StrokeCanvas {
        use texturecraft_compose::{Stencil, store::ImageRes};
        let m = primitives::plane(2.0, 4);
        let g = UvGBuffer::new(&m, 0, 64, 64);
        let image = std::sync::Arc::new(ImageRes { width: 1, height: 1, rgba: vec![[1.0; 4]], srgb: false });
        let view_proj =
            texturecraft_geom::glam::Mat4::perspective_rh(1.0, 1.0, 0.1, 50.0) * texturecraft_geom::glam::Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Z);
        // The image covers the left half of the screen only (x from -1 to 0).
        let stencil = Stencil { image, view_proj, center: [-0.5, 0.0], half: [0.5, 1.0], rotation: 0.0, tile: false };
        let mut canvas = StrokeCanvas::new(64, 64);
        let ctx = ProjectCtx { mesh: &m, gbuf: &g, depth, eye, backface_culling: culling, angle_fade: (70.0, 85.0) };
        project_coverage(&mut canvas, &ctx, &stencil);
        canvas
    }

    #[test]
    fn projection_maps_camera_left_to_texels_and_culls_backfaces() {
        let canvas = projected_plane(Vec3::new(0.0, 4.0, 0.0), true, None);
        let at = |x: u32, y: u32| from_u16(canvas.alpha.get(x, y)[0]);
        // Exactly one side of the middle row is under the image (the other half of the screen is empty).
        let (a, b) = (at(8, 32), at(56, 32));
        assert!((a > 0.99) != (b > 0.99), "only one half is projected: {a} {b}");
        assert!(a.max(b) > 0.99 && a.min(b) < 0.01);
        // From below the plane is a back face: nothing is painted; without culling it is.
        assert!(projected_plane(Vec3::new(0.0, -4.0, 0.0), true, None).dirty.is_empty());
        assert!(!projected_plane(Vec3::new(0.0, -4.0, 0.0), false, None).dirty.is_empty());
    }

    #[test]
    fn projection_fades_surfaces_seen_at_a_grazing_angle() {
        // Eye almost level with the plane (about 87 degrees off its normal): faded out.
        let canvas = projected_plane(Vec3::new(0.0, 0.15, 4.0), true, None);
        let max = (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).map(|(x, y)| from_u16(canvas.alpha.get(x, y)[0])).fold(0.0f32, f32::max);
        assert!(max < 0.5, "grazing view stays faint: {max}");
    }
}

/// A dab in texture space (2D view painting): center and radius in UV units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvStamp {
    pub center: texturecraft_geom::glam::Vec2,
    pub radius: f32,
    pub angle: f32,
    pub flow: f32,
    pub hardness: f32,
    pub seed: u32,
}

/// Accumulate UV-space stamps into `canvas` (only texels covered by the texture set).
pub fn rasterize_uv(canvas: &mut StrokeCanvas, gbuf: &UvGBuffer, shape: Shape, stamps: &[UvStamp]) {
    let (w, h) = (gbuf.width, gbuf.height);
    canvas.fit(gbuf);
    for s in stamps {
        if s.radius.is_nan() || s.radius <= 0.0 || !s.center.is_finite() {
            continue;
        }
        let (cx, cy, r) = (s.center.x * w as f32, s.center.y * h as f32, s.radius * w as f32);
        let x0 = (cx - r * 1.42).floor().max(0.0) as u32;
        let x1 = ((cx + r * 1.42).ceil().max(0.0) as u32).min(w);
        let y0 = (cy - r * 1.42).floor().max(0.0) as u32;
        let y1 = ((cy + r * 1.42).ceil().max(0.0) as u32).min(h);
        let (sn, cs) = s.angle.to_radians().sin_cos();
        let shape = shape.at_radius(r);
        for y in y0..y1 {
            for x in x0..x1 {
                if gbuf.tri.get((y * w + x) as usize).copied().unwrap_or(NO_TRI) == NO_TRI {
                    continue;
                }
                // Local frame with y up on screen (texel rows grow downward), as in the 3D view.
                let (dx, dy) = ((x as f32 + 0.5 - cx) / r, (cy - y as f32 - 0.5) / r);
                let (lx, ly) = (dx * cs + dy * sn, -dx * sn + dy * cs);
                let a = shape.sample(lx, ly, s.hardness, s.seed) * s.flow;
                if a > 0.0 {
                    canvas.texels.push((x, y, a));
                }
            }
        }
        canvas.add_stamp([cx, cy]);
    }
}
