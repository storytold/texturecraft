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

/// Most a texel's eye distance may exceed the nearest surface in its 3×3 pixel neighbourhood, in pixels along the surface
const NEIGHBOUR_REACH: f32 = 1.5;

/// Facing (cosine) below which the allowance stops growing
const MIN_FACING: f32 = 0.05;

/// Height of the view in normalized device coordinates (−1 to 1)
const NDC_HEIGHT: f32 = 2.0;

/// How far toward the eye to test a texel turned `facing` away: at grazing angles the neighbouring pixels of the same surface are much nearer, and the depth test would speckle
fn grazing_allowance(depth: &EyeDepth, point: Vec3, facing: f32, pixels_per_unit: f32) -> f32 {
    // Back faces get none, so painting with culling off doesn't reach through thin parts
    if facing <= 0.0 {
        return 0.0;
    }

    let clip_w = (depth.view_proj * point.extend(1.0)).w.max(0.0);
    let world_per_pixel = clip_w * NDC_HEIGHT / pixels_per_unit;
    let facing = facing.clamp(MIN_FACING, 1.0);
    world_per_pixel * NEIGHBOUR_REACH * (1.0 - facing * facing).sqrt() / facing
}

/// How far off the surface [`ray_reaches_eye`] starts its ray, so it doesn't hit the texel's own
/// triangle, as a fraction of [`EyeDepth::bias`] (itself 0.4 % of the mesh radius: this is
/// 0.001 % of the radius)
const RAY_START_PER_BIAS: f32 = 0.0025;

/// Whether a ray from `point` (on a surface facing `facing` toward the eye along `to_eye`) reaches
/// the eye without hitting the mesh, ignoring the first `skip` world units. The depth buffer is
/// asked first; this exact test only checks the texels it calls hidden but can't rule out
/// ([`EyeDepth::uncertain`]): its pixels are coarse, so on a surface turned far from the camera it
/// hides texels the camera does see, and strokes painted at a steep angle came out in stripes and
/// jagged gaps
fn ray_reaches_eye(context: &RasterCtx, depth: &EyeDepth, point: Vec3, normal: Vec3, facing: f32, to_eye: Vec3, skip: f32) -> bool {
    let eye_distance = context.eye.distance(point);
    let lift = depth.bias * RAY_START_PER_BIAS;
    let start = skip.max(0.0) + lift;
    if !eye_distance.is_finite() || start >= eye_distance {
        return true;
    }

    // Lift off the surface on the eye's side, so back faces (culling off) don't hit themselves
    let side = if facing < 0.0 { -normal } else { normal };
    let origin = point + side * lift + to_eye * start;
    !context.bvh.occluded(origin, to_eye, eye_distance - start)
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

/// Accumulate `stamps` into `canvas` ("over": `m = a + m·(1 − a)`).
pub fn rasterize(canvas: &mut StrokeCanvas, context: &RasterCtx, stamps: &[Stamp]) {
    let (width, height) = (context.gbuf.width, context.gbuf.height);
    canvas.fit(context.gbuf);
    let fade_lo = context.angle_fade.1.to_radians().cos();
    let fade_hi = context.angle_fade.0.to_radians().cos();

    // The camera scale is shared by every candidate texel in this batch
    let pixels_per_unit = context.depth.map_or(1.0, |depth| {
        let ndc_per_unit = depth.view_proj.row(1).truncate().length();
        (depth.height as f32 * ndc_per_unit).max(f32::EPSILON)
    });
    let mut triangles = Vec::new();
    for stamp in stamps {
        if stamp.radius.is_nan() || stamp.radius <= 0.0 || !stamp.center.is_finite() {
            continue;
        }

        let extent = Vec3::splat(stamp.radius * stamp.depth.max(1.0) * 1.42);
        triangles.clear();
        context.bvh.query_box(stamp.center - extent, stamp.center + extent, |triangle| triangles.push(triangle));

        // Each texel belongs to one triangle, so large stamps can scan triangles in parallel
        let scan = |&triangle: &u32| scan_triangle(context, stamp, triangle, width, height, fade_lo, fade_hi, pixels_per_unit);
        let parts: Vec<TriTexels> =
            if triangles.len() > 16 { triangles.par_iter().filter_map(scan).collect() } else { triangles.iter().filter_map(scan).collect() };

        // The footprint texel nearest the stamp center supplies the pickup's center
        let mut center = ([0.0f32; 2], f32::MAX);
        for (texels, nearest) in parts {
            if nearest.1 < center.1 {
                center = nearest;
            }
            canvas.texels.extend(texels);
        }
        canvas.add_stamp(center.0);
    }
}

/// Texels of one triangle a stamp covers, and the one nearest the stamp center.
type TriTexels = (Vec<(u32, u32, f32)>, ([f32; 2], f32));

#[allow(clippy::too_many_arguments)]
fn scan_triangle(
    context: &RasterCtx,
    stamp: &Stamp,
    triangle: u32,
    width: u32,
    height: u32,
    fade_lo: f32,
    fade_hi: f32,
    pixels_per_unit: f32,
) -> Option<TriTexels> {
    if context.mesh.material_of(triangle as usize) != Some(context.material) {
        return None;
    }

    let [first, second, third] = context.mesh.tri(triangle as usize)?;
    let (uv_a, uv_b, uv_c) = (context.mesh.uvs.get(first)?, context.mesh.uvs.get(second)?, context.mesh.uvs.get(third)?);
    let (position_a, position_b, position_c) = (context.mesh.positions.get(first)?, context.mesh.positions.get(second)?, context.mesh.positions.get(third)?);
    let (normal_a, normal_b, normal_c) = (context.mesh.normals.get(first)?, context.mesh.normals.get(second)?, context.mesh.normals.get(third)?);
    let x0 = (uv_a.x.min(uv_b.x).min(uv_c.x) * width as f32).floor().max(0.0) as u32;
    let x1 = ((uv_a.x.max(uv_b.x).max(uv_c.x) * width as f32).ceil().max(0.0) as u32).min(width);
    let y0 = (uv_a.y.min(uv_b.y).min(uv_c.y) * height as f32).floor().max(0.0) as u32;
    let y1 = ((uv_a.y.max(uv_b.y).max(uv_c.y) * height as f32).ceil().max(0.0) as u32).min(height);

    // Stamp radius in texels on this triangle: texel area over world area
    let uv_area = (*uv_b - *uv_a).perp_dot(*uv_c - *uv_a).abs() * width as f32 * height as f32;
    let world_area = (*position_b - *position_a).cross(*position_c - *position_a).length();
    let shape = context.shape.at_radius(stamp.radius * (uv_area / world_area).sqrt());

    let mut texels = Vec::new();
    let mut center = ([0.0f32; 2], f32::MAX);

    for y in y0..y1 {
        for x in x0..x1 {
            let texel = (y * width + x) as usize;
            if context.gbuf.tri.get(texel).copied().unwrap_or(NO_TRI) != triangle {
                continue;
            }

            let Some([b1, b2]) = context.gbuf.bary.get(texel).copied() else { continue };
            let b0 = 1.0 - b1 - b2;
            let position = *position_a * b0 + *position_b * b1 + *position_c * b2;
            let offset = position - stamp.center;
            let local_x = offset.dot(stamp.tangent) / stamp.radius;
            let local_y = offset.dot(stamp.bitangent) / stamp.radius;
            if local_x * local_x + local_y * local_y > 2.0 {
                continue;
            }

            if offset.dot(stamp.normal).abs() > stamp.radius * stamp.depth {
                continue;
            }

            let distance_squared = offset.length_squared();
            if distance_squared < center.1 {
                center = ([x as f32 + 0.5, y as f32 + 0.5], distance_squared);
            }

            let normal = (*normal_a * b0 + *normal_b * b1 + *normal_c * b2).try_normalize().unwrap_or(stamp.normal);
            if normal.dot(stamp.normal) <= 0.0 {
                continue;
            }

            // Allow half the stamp depth when the mirrored point is off an asymmetric surface
            let (seen, seen_normal) = stamp.source.map_or((position, normal), |source| (source.point(position), source.direction(normal)));
            let off_surface = if stamp.source.is_some() { stamp.radius * stamp.depth } else { 0.0 };
            let to_eye = (context.eye - seen).try_normalize().unwrap_or(seen_normal);
            let facing = seen_normal.dot(to_eye);
            if context.backface_culling && facing <= 0.0 {
                continue;
            }

            if let Some(depth) = context.depth {
                let skip = grazing_allowance(depth, seen, facing, pixels_per_unit) + off_surface;
                let tested = seen + to_eye * skip;
                // The exact test runs only where the buffer calls a texel hidden but can't be sure
                let hidden = !depth.visible(tested) && (!depth.uncertain(tested) || !ray_reaches_eye(context, depth, seen, seen_normal, facing, to_eye, skip));
                if hidden {
                    continue;
                }
            }

            let fade = if context.backface_culling { smoothstep(fade_lo, fade_hi, facing) } else { 1.0 };
            let alpha = shape.sample(local_x, local_y, stamp.hardness, stamp.seed) * stamp.flow * fade;
            if alpha > 0.0 {
                texels.push((x, y, alpha));
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

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::primitives;

    /// Texels a stamp of `radius` at `center` paints (alpha above 1 %) on `mesh` seen by `camera`,
    /// with the brush's visibility test (as a 3D stroke) or without it.
    fn painted(mesh: &Mesh, camera: &texturecraft_geom::OrbitCamera, center: Vec3, radius: f32, culling: bool, visibility: bool) -> usize {
        painted_texels(mesh, camera, center, radius, culling, visibility).len()
    }

    fn painted_texels(mesh: &Mesh, camera: &texturecraft_geom::OrbitCamera, center: Vec3, radius: f32, culling: bool, visibility: bool) -> Vec<(u32, u32)> {
        let bvh = Bvh::new(mesh);
        let g = UvGBuffer::new(mesh, 0, 256, 256);
        let mut canvas = StrokeCanvas::new(256, 256);
        // Half a 960×540 viewport, as the session renders it for a stroke
        let depth = EyeDepth::render(mesh, camera, 480, 270);
        let ctx = RasterCtx {
            mesh,
            bvh: &bvh,
            gbuf: &g,
            material: 0,
            depth: visibility.then_some(&depth),
            eye: camera.eye(),
            backface_culling: culling,
            angle_fade: (89.0, 90.0),
            shape: Shape { alpha: crate::BrushAlpha::Round, image: None, library: None, level: 0 },
        };
        let stamp =
            Stamp { center, normal: Vec3::Y, tangent: Vec3::X, bitangent: Vec3::Z, radius, flow: 1.0, hardness: 0.9, depth: 1.0, seed: 0, source: None };
        rasterize(&mut canvas, &ctx, &[stamp]);
        (0..256).flat_map(|y| (0..256).map(move |x| (x, y))).filter(|&(x, y)| from_u16(canvas.alpha.get(x, y)[0]) > 0.01).collect()
    }

    fn looking_down(pitch_degrees: f32) -> texturecraft_geom::OrbitCamera {
        texturecraft_geom::OrbitCamera { pitch: pitch_degrees.to_radians(), ..Default::default() }
    }

    /// A plane seen almost edge-on: every texel of the stamp is in plain view, so the brush must
    /// paint all of them. The coarse depth buffer used to call many hidden, and strokes painted at
    /// a steep angle came out in stripes and jagged gaps.
    #[test]
    fn a_surface_seen_at_a_grazing_angle_takes_the_whole_stamp() {
        let plane = primitives::plane(2.0, 8);
        for pitch in [10.0, 3.0, 1.0] {
            let all = painted(&plane, &looking_down(pitch), Vec3::ZERO, 0.3, true, false);
            let seen = painted(&plane, &looking_down(pitch), Vec3::ZERO, 0.3, true, true);
            assert!(all > 1000, "the stamp covers the plane: {all}");
            assert_eq!(seen, all, "at {pitch}° every texel the stamp covers is visible");
        }
    }

    /// A shelf over half of a floor: under it the brush paints nothing, steep angle or not, while
    /// the open half takes the stamp.
    #[test]
    fn geometry_in_front_still_hides_what_is_behind_it() {
        let floor = [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 1.0), Vec3::new(-1.0, 0.0, 1.0)];
        let shelf = [Vec3::new(-1.0, 0.3, -1.0), Vec3::new(0.0, 0.3, -1.0), Vec3::new(0.0, 0.3, 1.0), Vec3::new(-1.0, 0.3, 1.0)];
        let uv = |p: Vec3| texturecraft_geom::glam::Vec2::new((p.x + 1.0) / 2.0, (p.z + 1.0) / 2.0);
        // The shelf maps to one corner point of the texture, so only the floor has texels
        let uvs = floor.iter().map(|p| uv(*p)).chain([texturecraft_geom::glam::Vec2::ONE; 4]).collect();
        // Wound to face up (+Y), toward the camera
        let quads = [0u32, 2, 1, 0, 3, 2];
        let indices = quads.iter().copied().chain(quads.iter().map(|i| i + 4)).collect();
        let mesh = Mesh::from_parts(floor.into_iter().chain(shelf).collect(), None, uvs, indices, "m").unwrap();
        for pitch in [60.0, 30.0] {
            let camera = looking_down(pitch);
            let open = Vec3::new(0.5, 0.0, -0.5);
            assert!(painted(&mesh, &camera, open, 0.2, true, false) > 100, "the stamp covers the open half");
            assert_eq!(
                painted(&mesh, &camera, open, 0.2, true, true),
                painted(&mesh, &camera, open, 0.2, true, false),
                "the open half takes the stamp at {pitch}°"
            );
            let under = Vec3::new(-0.5, 0.0, -0.5);
            assert!(painted(&mesh, &camera, under, 0.2, true, false) > 100, "the stamp reaches under the shelf");
            assert_eq!(painted(&mesh, &camera, under, 0.2, true, true), 0, "the shelf hides the floor under it at {pitch}°");
            // Across the shelf's edge (x = 0, texel column 128) the buffer can't tell the two sides
            // apart, and the exact test decides: the open side is painted, the side under the shelf isn't
            let edge = Vec3::new(0.0, 0.0, -0.5);
            let all = painted_texels(&mesh, &camera, edge, 0.2, true, false);
            let seen = painted_texels(&mesh, &camera, edge, 0.2, true, true);
            let open: Vec<_> = all.iter().filter(|t| t.0 >= 129).copied().collect();
            assert!(open.len() > 100 && all.len() > open.len() + 100, "the stamp straddles the edge");
            assert!(open.iter().all(|t| seen.contains(t)), "every open texel is painted at {pitch}°");
            assert!(seen.iter().all(|t| t.0 >= 127), "nothing under the shelf is painted at {pitch}°");
        }
    }

    /// A stamp at the far edge of a grazing plane: the pixels beyond the edge are empty, so the
    /// texels at the edge lie farther than every finite neighbour. They are in plain view, so they
    /// must still be painted (the buffer can't prove them hidden, the exact test must decide).
    #[test]
    fn the_far_edge_of_a_grazing_surface_takes_paint() {
        let plane = primitives::plane(2.0, 8);
        for culling in [true, false] {
            for pitch in [10.0, 3.0, 1.0, -3.0] {
                if culling && pitch < 0.0 {
                    continue;
                }
                for edge in [Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 1.0), Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)] {
                    let all = painted_texels(&plane, &looking_down(pitch), edge, 0.3, culling, false);
                    let seen = painted_texels(&plane, &looking_down(pitch), edge, 0.3, culling, true);
                    assert!(all.len() > 200, "the stamp covers part of the plane: {}", all.len());
                    assert_eq!(seen.len(), all.len(), "edge {edge} at {pitch} degrees (culling {culling}): every texel is in view");
                }
            }
        }
    }

    /// With backface culling off, a single-sided surface seen from behind still takes paint: the
    /// exact test starts on the eye's side of the surface, so it doesn't hit the surface itself.
    #[test]
    fn a_back_face_with_culling_off_takes_paint() {
        let plane = primitives::plane(2.0, 8);
        for pitch in [-45.0, -3.0] {
            let all = painted(&plane, &looking_down(pitch), Vec3::ZERO, 0.3, false, false);
            assert!(all > 1000);
            assert_eq!(painted(&plane, &looking_down(pitch), Vec3::ZERO, 0.3, false, true), all, "from below at {pitch}°");
        }
    }

    /// Stamps along a sphere's silhouette, half of each really hidden: the most texels the exact
    /// ray test has to check. `cargo test --release -p texturecraft-paint --lib -- --ignored benchmark_silhouette --nocapture`
    #[test]
    #[ignore = "release performance measurement"]
    fn benchmark_silhouette_stamps() {
        let sphere = primitives::sphere(1.0, 256, 128);
        let bvh = Bvh::new(&sphere);
        let g = UvGBuffer::new(&sphere, 0, 2048, 2048);
        let camera = looking_down(0.0);
        let depth = EyeDepth::render(&sphere, &camera, 960, 540);
        let ctx = RasterCtx {
            mesh: &sphere,
            bvh: &bvh,
            gbuf: &g,
            material: 0,
            depth: Some(&depth),
            eye: camera.eye(),
            backface_culling: false,
            angle_fade: (89.0, 90.0),
            shape: Shape { alpha: crate::BrushAlpha::Round, image: None, library: None, level: 0 },
        };
        // The silhouette seen from +Z is the circle z = 0
        let stamps: Vec<Stamp> = (0..64)
            .map(|k| {
                let a = k as f32 / 64.0 * std::f32::consts::TAU;
                let center = Vec3::new(a.cos(), a.sin(), 0.0);
                let tangent = Vec3::Z;
                Stamp {
                    center,
                    normal: center,
                    tangent,
                    bitangent: center.cross(tangent),
                    radius: 0.15,
                    flow: 1.0,
                    hardness: 0.9,
                    depth: 1.0,
                    seed: k,
                    source: None,
                }
            })
            .collect();
        let mut times = Vec::new();
        for _ in 0..9 {
            let mut canvas = StrokeCanvas::new(2048, 2048);
            let start = std::time::Instant::now();
            rasterize(&mut canvas, &ctx, &stamps);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        eprintln!("64 silhouette stamps at 2048²: median {:.2} ms, range {:.2}..{:.2} ms", times[4], times[0], times[8]);
    }

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
            source: None,
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
