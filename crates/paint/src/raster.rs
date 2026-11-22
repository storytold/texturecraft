//! Stamps → stroke alpha in texture space. Every texel whose surface point lies inside a
//! stamp's oriented volume receives paint, so UV seams are seamless by construction.

use std::collections::BTreeSet;

use rayon::prelude::*;

use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_geom::{Bvh, Mesh, NO_TRI, UvGBuffer};
use texturecraft_texel::{TILE, TileImage, from_u16, to_u16};

use crate::brush::{Shape, StampBlending};
use crate::pickup::Pickup;
use crate::stamp_source::StampSource;
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
    stamp_blending: StampBlending,
    /// The stamp being added: `(x, y, alpha)` per texel (reused between stamps).
    texels: Vec<(u32, u32, f32)>,
}

impl StrokeCanvas {
    pub fn new(width: u32, height: u32) -> StrokeCanvas {
        StrokeCanvas {
            alpha: TileImage::new(width, height, 1),
            dirty: BTreeSet::new(),
            recent: BTreeSet::new(),
            limit: None,
            pickup: None,
            stamp_blending: StampBlending::Normal,
            texels: Vec::new(),
        }
    }

    pub fn with_stamp_blending(mut self, stamp_blending: StampBlending) -> Self {
        self.stamp_blending = stamp_blending;
        self
    }

    /// Start over when the canvas is not the size of `gbuf`.
    fn fit(&mut self, gbuf: &UvGBuffer) {
        if (self.alpha.width(), self.alpha.height()) != (gbuf.width, gbuf.height) {
            *self = StrokeCanvas::new(gbuf.width, gbuf.height).with_stamp_blending(self.stamp_blending);
        }
    }

    /// Multiply the stroke's coverage by `factor(x, y)` (clamped to 0..1) on every texel it touched: a stencil limiting
    /// the stroke as a whole ([`crate::stencil`]).
    pub fn scale(&mut self, factor: impl Fn(u32, u32) -> f32 + Sync) {
        let (width, height) = (self.alpha.width(), self.alpha.height());
        let alpha = &self.alpha;
        let scaled: Vec<((u32, u32), Vec<u16>)> = self
            .dirty
            .par_iter()
            .filter_map(|&(tx, ty)| {
                let mut data = alpha.tile(tx, ty)?.to_vec();
                for (i, v) in data.iter_mut().enumerate() {
                    let (x, y) = (tx * TILE + i as u32 % TILE, ty * TILE + i as u32 / TILE);
                    if *v == 0 || x >= width || y >= height {
                        continue;
                    }
                    let f = factor(x, y);
                    *v = to_u16(from_u16(*v) * if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 });
                }
                Some(((tx, ty), data))
            })
            .collect();
        for ((tx, ty), data) in scaled {
            self.alpha.set_tile(tx, ty, Some(std::sync::Arc::new(data)));
        }
    }

    fn touch(&mut self, x: u32, y: u32) {
        let t = (x / TILE, y / TILE);
        self.dirty.insert(t);
        self.recent.insert(t);
    }

    /// Lays `texels` (`(x, y, alpha)`) as one stamp, "over" what the canvas holds; a canvas not the size of `gbuf`
    /// starts over first (bands along a path, [`crate::ribbon`]).
    pub(crate) fn add_texels(&mut self, gbuf: &UvGBuffer, texels: Vec<(u32, u32, f32)>) {
        self.fit(gbuf);
        self.texels = texels;
        self.add_stamp([0.0; 2], None);
    }

    /// Blend the gathered stamp texels (stamp center `center` in texels) into the stroke: the
    /// quick mask scales them, then the pickup deposits its values or the alpha accumulates
    /// with the brush's stamp blending mode.
    fn add_stamp(&mut self, center: [f32; 2], source: Option<StampSource>) {
        let mut texels = std::mem::take(&mut self.texels);
        if let Some((mask, invert)) = &self.limit {
            for t in &mut texels {
                let m = from_u16(mask.get(t.0, t.1)[0]);
                t.2 *= if *invert { 1.0 - m } else { m };
            }
        }
        let painted = match &mut self.pickup {
            Some(p) => p.stamp(&mut self.alpha, &texels, center, source, self.stamp_blending),
            None => {
                for &(x, y, a) in &texels {
                    let m = from_u16(self.alpha.get(x, y)[0]);
                    self.alpha.set(x, y, &[to_u16(self.stamp_blending.accumulate(m, a))]);
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

/// Whether a ray from `point` toward the eye along `to_eye` reaches
/// the eye without hitting the mesh. The coarse depth buffer is checked first; this ray resolves
/// uncertain texels and those made apparently visible only by the grazing allowance. Orthographic
/// rays travel parallel to the view direction up to the eye plane.
fn ray_reaches_eye(context: &RasterCtx, depth: &EyeDepth, point: Vec3, geometric_normal: Vec3, to_eye: Vec3) -> bool {
    let direction = depth.orthographic_to_eye.unwrap_or(to_eye);
    let eye_distance = (context.eye - point).dot(direction);
    let lift = depth.bias * RAY_START_PER_BIAS;
    if !eye_distance.is_finite() {
        return true;
    }

    // Orthographic points on or behind the eye plane cannot be visible
    if eye_distance <= 0.0 {
        return false;
    }

    if lift >= eye_distance {
        return true;
    }

    // Lift off the surface on the eye's side to avoid self-hits without skipping nearby occluders
    let side = if geometric_normal.dot(direction) < 0.0 { -geometric_normal } else { geometric_normal };
    let origin = point + side * lift + direction * lift;
    !context.bvh.occluded(origin, direction, eye_distance - lift)
}

/// Resolve coarse depth uncertainty without letting a grazing allowance hide nearby geometry
fn surface_visible(context: &RasterCtx, depth: &EyeDepth, point: Vec3, geometric_normal: Vec3, to_eye: Vec3, grazing_offset: f32, off_surface: f32) -> bool {
    // Mirror displacement may cross the source surface; grazing displacement may cross an occluder
    let allowed_point = point + to_eye * off_surface;
    let tested = allowed_point + to_eye * grazing_offset;
    let (visible, uncertain) = depth.visibility(tested);
    if visible {
        // Confirm past mirror displacement so a grazing offset cannot skip a foreground sheet
        return (grazing_offset <= 0.0 || depth.visible(allowed_point)) || ray_reaches_eye(context, depth, allowed_point, geometric_normal, to_eye);
    }

    uncertain && ray_reaches_eye(context, depth, allowed_point, geometric_normal, to_eye)
}

/// Which surfaces in a stamp's volume it reaches, by how far their normal turns from the stamp's.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Facing {
    /// Surfaces on the stamp's side (TextureCraft's own painting).
    #[default]
    Front,
    /// A smooth fade over the cosine between a texel's normal and the stamp's: everything at `full` and above,
    /// nothing at `none` and below (`none` < `full`). Off by default; for strokes replayed from an imported project,
    /// whose brush fades by that angle and not by the view.
    Fade { full: f32, none: f32 },
    /// Every surface in the volume, whichever way it faces.
    Any,
}

/// How far from a stamp's centre a texel lies, and so which surfaces in its volume it reaches.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Reach {
    /// The texel's offset laid flat on the stamp's plane, cut off at the stamp's depth (TextureCraft's own painting).
    #[default]
    Plane,
    /// The texel's distance along the surface ([`surface_reach`]): a stamp keeps its size on grazing surfaces and
    /// wraps over edges, and a surface lying off the stamp's surface (a sheet behind another) fades out the further off
    /// it lies; the stamp's depth is not used. Off by default; for strokes replayed from an imported project, whose
    /// dabs reach so (measured).
    Surface,
}

/// Reach off the smooth curve: the distance scales by `(1 − x²)` to this power (measured on stacked sheets and a
/// cylinder).
const OFF_CURVE_POWER: f32 = -0.85;

/// Least `cos(φ / 2)` the surface distance divides by (a texel turned almost a half turn from the stamp).
const MIN_HALF_TURN: f32 = 0.05;

/// How far from a stamp's centre a texel lies along the surface, as a share of `radius` (infinite: out of reach).
/// `offset` goes from the centre to the texel, `center_normal` and `texel_normal` are the unit normals there. On a
/// smooth surface this is the straight-line distance: `t / cos(φ / 2)`, with `t` the offset's length in the centre's
/// tangent plane and `φ` the turn between the normals. A surface lying off that curve by `x` radii along the centre's
/// normal comes nearer the rim the further off it lies: the distance scales by `(1 − x²)^−0.85`, and `x ≥ 1` is out of
/// reach.
pub fn surface_reach(offset: Vec3, center_normal: Vec3, texel_normal: Vec3, radius: f32) -> f32 {
    if radius.is_nan() || radius <= 0.0 {
        return f32::INFINITY;
    }
    let along = offset.dot(center_normal);
    let flat = (offset - center_normal * along).length();
    let half = (0.5 * (1.0 + center_normal.dot(texel_normal)).clamp(0.0, 2.0)).sqrt();
    let sin_half = (1.0 - half * half).max(0.0).sqrt();
    let curve = if half > 1e-3 { flat * sin_half / half } else { 0.0 };
    let beyond = ((along.abs() - curve).max(0.0) / radius).min(1.0);
    if beyond >= 1.0 || beyond.is_nan() {
        return f32::INFINITY;
    }
    flat / half.max(MIN_HALF_TURN) / radius * (1.0 - beyond * beyond).powf(OFF_CURVE_POWER)
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
    /// Which surfaces a stamp reaches by their turn from its normal ([`Facing::Front`] for painting).
    pub facing: Facing,
    /// How far a stamp reaches ([`Reach::Plane`] for painting).
    pub reach: Reach,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Accumulate `stamps` into `canvas` ("over": `m = a + m·(1 − a)`).
pub fn rasterize(canvas: &mut StrokeCanvas, context: &RasterCtx, stamps: &[Stamp]) {
    canvas.fit(context.gbuf);
    let scan = Scan::new(context);
    let mut triangles = Vec::new();
    for stamp in stamps {
        let mut texels = std::mem::take(&mut canvas.texels);
        let center = scan.stamp(stamp, &mut triangles, &mut texels);
        canvas.texels = texels;
        if let Some(center) = center {
            canvas.add_stamp(center, stamp.source);
        }
    }
}

/// The texels one stamp reaches, `(x, y, alpha)` with its shape, flow and fades: what [`rasterize`] lays for it,
/// without laying it (for tools that use a stamp's texels themselves, such as replayed smudging).
pub fn footprint(context: &RasterCtx, stamp: &Stamp) -> Vec<(u32, u32, f32)> {
    let mut texels = Vec::new();
    Scan::new(context).stamp(stamp, &mut Vec::new(), &mut texels);
    texels
}

/// What every stamp of a batch shares while its texels are found.
struct Scan<'c, 'a> {
    context: &'c RasterCtx<'a>,
    fade_lo: f32,
    fade_hi: f32,
    /// The camera's scale, shared by every candidate texel.
    pixels_per_unit: f32,
}

impl<'c, 'a> Scan<'c, 'a> {
    fn new(context: &'c RasterCtx<'a>) -> Scan<'c, 'a> {
        let pixels_per_unit = context.depth.map_or(1.0, |depth| {
            let ndc_per_unit = depth.view_proj.row(1).truncate().length();
            (depth.height as f32 * ndc_per_unit).max(f32::EPSILON)
        });
        Scan { context, fade_lo: context.angle_fade.1.to_radians().cos(), fade_hi: context.angle_fade.0.to_radians().cos(), pixels_per_unit }
    }

    /// Appends the texels `stamp` reaches to `texels` (`triangles`: scratch); the texel centre nearest the stamp's
    /// centre (the pickup's centre), `None` for a stamp that is no stamp.
    fn stamp(&self, stamp: &Stamp, triangles: &mut Vec<u32>, texels: &mut Vec<(u32, u32, f32)>) -> Option<[f32; 2]> {
        let context = self.context;
        if stamp.radius.is_nan() || stamp.radius <= 0.0 || !stamp.center.is_finite() {
            return None;
        }
        let (width, height) = (context.gbuf.width, context.gbuf.height);
        let extent = match context.reach {
            Reach::Plane => stamp.world_extent(),
            Reach::Surface => Vec3::splat(stamp.radius * stamp.depth.max(1.0) * 1.42),
        };
        triangles.clear();
        context.bvh.query_box(stamp.center - extent, stamp.center + extent, |triangle| triangles.push(triangle));

        // Each texel belongs to one triangle, so large stamps can scan triangles in parallel
        let scan = |&triangle: &u32| scan_triangle(context, stamp, triangle, width, height, self.fade_lo, self.fade_hi, self.pixels_per_unit);
        let parts: Vec<TriTexels> =
            if triangles.len() > 16 { triangles.par_iter().filter_map(scan).collect() } else { triangles.iter().filter_map(scan).collect() };

        // The footprint texel nearest the stamp center supplies the pickup's center
        let mut center = ([0.0f32; 2], f32::MAX);
        for (found, nearest) in parts {
            if nearest.1 < center.1 {
                center = nearest;
            }
            texels.extend(found);
        }
        Some(center.0)
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
    let seen_geometric_normal = (*position_b - *position_a)
        .cross(*position_c - *position_a)
        .try_normalize()
        .map(|normal| stamp.source.map_or(normal, |source| source.direction(normal)));
    let (x0, y0, x1, y1) = uv_texel_bounds([*uv_a, *uv_b, *uv_c], (width, height));

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
            let mut local_x = offset.dot(stamp.tangent) / stamp.radius;
            let mut local_y = offset.dot(stamp.bitangent) / stamp.radius;
            let flat_squared = local_x * local_x + local_y * local_y;
            // the surface distance is never shorter than the flat one, so this bound holds for both reaches
            if flat_squared > 2.0 {
                continue;
            }

            if context.reach == Reach::Plane && offset.dot(stamp.normal).abs() > stamp.radius * stamp.depth {
                continue;
            }

            let normal = (*normal_a * b0 + *normal_b * b1 + *normal_c * b2).try_normalize().unwrap_or(stamp.normal);
            if context.reach == Reach::Surface {
                // the tip is read at the surface distance, in the offset's direction on the stamp's plane
                let distance = surface_reach(offset, stamp.normal, normal, stamp.radius);
                if distance.is_nan() || distance * distance > 2.0 {
                    continue;
                }
                let flat = flat_squared.sqrt();
                let scale = if flat > 1e-6 { distance / flat } else { 1.0 };
                local_x *= scale;
                local_y *= scale;
            }

            let distance_squared = offset.length_squared();
            if distance_squared < center.1 {
                center = ([x as f32 + 0.5, y as f32 + 0.5], distance_squared);
            }

            let turn = match context.facing {
                Facing::Front if normal.dot(stamp.normal) <= 0.0 => continue,
                Facing::Front | Facing::Any => 1.0,
                Facing::Fade { full, none } => smoothstep(none, full, normal.dot(stamp.normal)),
            };
            if turn <= 0.0 {
                continue;
            }

            // Allow half the stamp depth when the mirrored point is off an asymmetric surface
            let (seen, seen_normal) = stamp.source.map_or((position, normal), |source| (source.point(position), source.direction(normal)));
            let off_surface = if stamp.source.is_some() { stamp.radius * stamp.depth } else { 0.0 };
            let to_eye =
                context.depth.and_then(|depth| depth.orthographic_to_eye).unwrap_or_else(|| (context.eye - seen).try_normalize().unwrap_or(seen_normal));
            let facing = seen_normal.dot(to_eye);
            if context.backface_culling && facing <= 0.0 {
                continue;
            }

            if let Some(depth) = context.depth {
                let grazing_offset = grazing_allowance(depth, seen, facing, pixels_per_unit);
                if !surface_visible(context, depth, seen, seen_geometric_normal.unwrap_or(seen_normal), to_eye, grazing_offset, off_surface) {
                    continue;
                }
            }

            let fade = if context.backface_culling { smoothstep(fade_lo, fade_hi, facing) } else { 1.0 };
            let alpha = shape.sample(local_x, local_y, stamp.hardness, stamp.seed) * stamp.flow * fade * turn;
            if alpha > 0.0 {
                texels.push((x, y, alpha));
            }
        }
    }

    (center.1 < f32::MAX || !texels.is_empty()).then_some((texels, center))
}

fn uv_texel_bounds([a, b, c]: [Vec2; 3], (width, height): (u32, u32)) -> (u32, u32, u32, u32) {
    let size = Vec2::new(width as f32, height as f32);
    let low = (a.min(b).min(c) * size).floor().max(Vec2::ZERO).as_uvec2();
    let high = (a.max(b).max(c) * size).ceil().max(Vec2::ZERO).min(size).as_uvec2();
    (low.x, low.y, high.x, high.y)
}

/// Maximum texels the world brush scans for this candidate triangle.
pub fn triangle_scan_texels(mesh: &Mesh, triangle: u32, size: (u32, u32)) -> u64 {
    let Some([a, b, c]) = mesh.tri(triangle as usize) else { return 0 };
    let (Some(a), Some(b), Some(c)) = (mesh.uvs.get(a), mesh.uvs.get(b), mesh.uvs.get(c)) else { return 0 };
    let (x0, y0, x1, y1) = uv_texel_bounds([*a, *b, *c], size);
    u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0))
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
    canvas.add_stamp([0.0; 2], None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::primitives;

    #[test]
    fn tilted_normals_do_not_hide_a_visible_grazing_plane() {
        let mut plane = primitives::plane(2.0, 8);
        plane.normals.fill(Vec3::new(0.0, 0.866_025_4, -0.5));
        let camera = looking_down(3.0);
        let all = painted(&plane, &camera, Vec3::ZERO, 0.3, false, false);
        let visible = painted(&plane, &camera, Vec3::ZERO, 0.3, false, true);

        let to_eye = camera.eye().normalize();
        assert!(plane.normals.first().is_some_and(|normal| normal.dot(to_eye) < 0.0));
        assert!(Vec3::Y.dot(to_eye) > 0.0, "the geometric plane faces the eye");
        assert!(all > 1000, "the stamp reaches the plane");
        assert_eq!(visible, all, "tilted shading normals must not move the ray below the plane");
    }

    #[test]
    fn mirrored_stamp_accepts_a_source_behind_its_surface() {
        use texturecraft_geom::glam::{Mat3, Vec2};

        let corners = [Vec3::new(-1.0, 0.03, -1.0), Vec3::new(1.0, 0.03, -1.0), Vec3::new(1.0, 0.03, 1.0), Vec3::new(-1.0, 0.03, 1.0)];
        let source_corners = corners.map(|point| Vec3::new(point.x, -0.02, point.z));
        let uvs = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().chain([Vec2::ZERO; 4]).collect();
        let indices = [0, 2, 1, 0, 3, 2, 4, 6, 5, 4, 7, 6].to_vec();
        let mesh = Mesh::from_parts(corners.into_iter().chain(source_corners).collect(), None, uvs, indices, "asymmetric mirror").unwrap();
        let camera = looking_down(45.0);
        let bvh = Bvh::new(&mesh);
        let gbuf = UvGBuffer::new(&mesh, 0, 128, 128);
        let depth = EyeDepth::render(&mesh, &camera, 480, 270);
        let context = RasterCtx {
            mesh: &mesh,
            bvh: &bvh,
            gbuf: &gbuf,
            material: 0,
            depth: Some(&depth),
            eye: camera.eye(),
            backface_culling: false,
            angle_fade: (89.0, 90.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing: Facing::Front,
            reach: Reach::Plane,
        };
        let stamp = Stamp {
            center: Vec3::new(0.0, 0.03, 0.0),
            normal: Vec3::Y,
            tangent: Vec3::X,
            bitangent: Vec3::Z,
            radius: 0.3,
            flow: 1.0,
            hardness: 0.9,
            depth: 1.0,
            seed: 0,
            source: Some(crate::StampSource { origin: Vec3::ZERO, to_source: Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0)) }),
        };
        let mut canvas = StrokeCanvas::new(128, 128);
        let mapped = Vec3::new(0.0, -0.03, 0.0);
        let to_eye = (camera.eye() - mapped).normalize();

        assert_eq!(stamp.source.map(|source| source.point(stamp.center)), Some(mapped));
        assert!(stamp.radius * stamp.depth > 0.01, "the mapped point is inside the off-surface allowance");
        assert!(bvh.closest(mapped, to_eye, stamp.radius * stamp.depth).is_some(), "the old exact ray hits the source surface");
        assert!(!depth.visible(mapped), "the unshifted depth query rejects the mapped point");
        assert!(depth.visible(mapped + to_eye * stamp.radius * stamp.depth), "the off-surface query accepts it");
        rasterize(&mut canvas, &context, &[stamp]);

        assert!(from_u16(canvas.alpha.get(64, 64)[0]) > 0.9, "the mirrored stamp paints the visible copy texel");
    }

    #[test]
    fn grazing_ray_detects_a_sheet_inside_the_depth_allowance() {
        let positions = vec![Vec3::new(-0.2, -0.2, 0.005), Vec3::new(0.2, -0.2, 0.005), Vec3::new(0.0, 0.2, 0.005)];
        let mesh = Mesh::from_parts(positions, None, vec![texturecraft_geom::glam::Vec2::ZERO; 3], vec![0, 1, 2], "thin sheet").unwrap();
        let camera = looking_down(3.0);
        let mesh_bvh = Bvh::new(&mesh);
        let uv_buffer = UvGBuffer::new(&mesh, 0, 4, 4);
        let depth = EyeDepth::render(&mesh, &camera, 480, 270);
        let context = RasterCtx {
            mesh: &mesh,
            bvh: &mesh_bvh,
            gbuf: &uv_buffer,
            material: 0,
            depth: Some(&depth),
            eye: camera.eye(),
            backface_culling: true,
            angle_fade: (89.0, 90.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing: Facing::Front,
            reach: Reach::Plane,
        };
        let point = Vec3::ZERO;
        let to_eye = (camera.eye() - point).normalize();
        let pixels_per_unit = depth.height as f32 * depth.view_proj.row(1).truncate().length();
        let facing = Vec3::Y.dot(to_eye);
        let skip = grazing_allowance(&depth, point, facing, pixels_per_unit);

        assert!(skip > 0.005 / to_eye.z, "the sheet lies inside the allowance band");
        assert_eq!(depth.visibility(point), (false, true));
        assert!(depth.visible(point + to_eye * skip), "the allowance alone marks the hidden point visible");
        assert!(!surface_visible(&context, &depth, point, Vec3::Y, to_eye, skip, 0.0));
    }

    #[test]
    fn orthographic_occlusion_ray_stays_above_an_off_axis_point() {
        let positions = vec![Vec3::new(0.9, -0.2, 1.0), Vec3::new(1.1, -0.2, 1.0), Vec3::new(1.0, 0.2, 1.0)];
        let mesh = Mesh::from_parts(positions, None, vec![texturecraft_geom::glam::Vec2::ZERO; 3], vec![0, 1, 2], "blocker").unwrap();
        let camera = texturecraft_geom::OrbitCamera { pitch: 0.0, ortho: true, ..Default::default() };
        let mesh_bvh = Bvh::new(&mesh);
        let uv_buffer = UvGBuffer::new(&mesh, 0, 4, 4);
        let depth = EyeDepth::render(&mesh, &camera, 128, 128);
        let context = RasterCtx {
            mesh: &mesh,
            bvh: &mesh_bvh,
            gbuf: &uv_buffer,
            material: 0,
            depth: Some(&depth),
            eye: camera.eye(),
            backface_culling: true,
            angle_fade: (89.0, 90.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing: Facing::Front,
            reach: Reach::Plane,
        };
        let point = Vec3::new(1.0, 0.0, 0.0);
        let to_eye = (camera.eye() - point).normalize();

        assert!(!ray_reaches_eye(&context, &depth, point, Vec3::Z, to_eye));
    }

    #[test]
    fn orthographic_ray_rejects_points_on_or_behind_the_eye_plane() {
        let mesh = primitives::plane(2.0, 8);
        let camera = texturecraft_geom::OrbitCamera { ortho: true, ..Default::default() };
        let bvh = Bvh::new(&mesh);
        let gbuf = UvGBuffer::new(&mesh, 0, 4, 4);
        let mut depth = EyeDepth::render(&mesh, &camera, 4, 4);
        depth.dist.fill(0.1);
        depth.dist[10] = f32::MAX;
        let context = RasterCtx {
            mesh: &mesh,
            bvh: &bvh,
            gbuf: &gbuf,
            material: 0,
            depth: Some(&depth),
            eye: camera.eye(),
            backface_culling: false,
            angle_fade: (89.0, 90.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing: Facing::Front,
            reach: Reach::Plane,
        };
        let to_eye = -camera.forward();
        let behind = context.eye + to_eye;
        let near = context.eye - to_eye * (depth.bias * RAY_START_PER_BIAS * 0.5);

        assert_eq!(depth.visibility(behind), (false, true), "the uncertain depth reaches the exact ray fallback");
        assert!(!surface_visible(&context, &depth, behind, to_eye, to_eye, 0.0, 0.0));
        assert!(!ray_reaches_eye(&context, &depth, context.eye, to_eye, to_eye));
        assert!(ray_reaches_eye(&context, &depth, near, to_eye, to_eye), "the existing near-eye fallback remains accepted");
        assert!(ray_reaches_eye(&context, &depth, Vec3::splat(f32::NAN), to_eye, to_eye), "nonfinite input retains its fallback");
    }

    #[test]
    fn orthographic_stamps_do_not_paint_behind_an_off_axis_shelf() {
        let floor = [Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0), Vec3::new(-2.0, 0.0, 2.0)];
        let shelf = [Vec3::new(0.95, 0.025, -2.0), Vec3::new(1.05, 0.025, -2.0), Vec3::new(1.05, 0.025, 2.0), Vec3::new(0.95, 0.025, 2.0)];
        let floor_uvs = floor.iter().map(|point| texturecraft_geom::glam::Vec2::new((point.x + 2.0) / 4.0, (point.z + 2.0) / 4.0));
        let uvs = floor_uvs.chain([texturecraft_geom::glam::Vec2::ZERO; 4]).collect();
        let quad_indices = [0u32, 2, 1, 0, 3, 2];
        let indices = quad_indices.iter().copied().chain(quad_indices.iter().map(|index| index + 4)).collect();
        let mesh = Mesh::from_parts(floor.into_iter().chain(shelf).collect(), None, uvs, indices, "floor and shelf").unwrap();
        let camera = texturecraft_geom::OrbitCamera { ortho: true, ..looking_down(3.0) };
        let center = Vec3::new(1.0, 0.0, 0.0);
        let depth = EyeDepth::render(&mesh, &camera, 480, 270);
        let to_eye = -camera.forward();
        let pixels_per_unit = depth.height as f32 * depth.view_proj.row(1).truncate().length();
        let skip = grazing_allowance(&depth, center, Vec3::Y.dot(to_eye), pixels_per_unit);

        assert_eq!(depth.visibility(center + to_eye * skip), (false, true), "the stamp center requires the exact visibility test");

        let all = painted_texels(&mesh, &camera, center, 0.2, false, false);
        let visible = painted_texels(&mesh, &camera, center, 0.2, false, true);

        // Columns 191 and 192 lie strictly beneath the shelf at x = 1
        assert!(all.iter().any(|&(x, _)| (191..=192).contains(&x)), "the stamp covers the hidden floor");
        assert!(!visible.is_empty(), "the floor beside the shelf remains paintable");
        assert!(!visible.iter().any(|&(x, _)| (191..=192).contains(&x)), "the shelf hides the floor in the orthographic view");
    }

    /// Texels a stamp of `radius` at `center` paints (alpha above 1 %) on `mesh` seen by `camera`,
    /// with the brush's visibility test (as a 3D stroke) or without it.
    fn painted(mesh: &Mesh, camera: &texturecraft_geom::OrbitCamera, center: Vec3, radius: f32, culling: bool, visibility: bool) -> usize {
        painted_texels(mesh, camera, center, radius, culling, visibility).len()
    }

    fn painted_texels(mesh: &Mesh, camera: &texturecraft_geom::OrbitCamera, center: Vec3, radius: f32, culling: bool, visibility: bool) -> Vec<(u32, u32)> {
        let mesh_bvh = Bvh::new(mesh);
        let uv_buffer = UvGBuffer::new(mesh, 0, 256, 256);
        let mut canvas = StrokeCanvas::new(256, 256);

        // Half a 960×540 viewport, as the session renders it for a stroke
        let depth = EyeDepth::render(mesh, camera, 480, 270);
        let context = RasterCtx {
            mesh,
            bvh: &mesh_bvh,
            gbuf: &uv_buffer,
            material: 0,
            depth: visibility.then_some(&depth),
            eye: camera.eye(),
            backface_culling: culling,
            angle_fade: (89.0, 90.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing: Facing::Front,
            reach: Reach::Plane,
        };
        let stamp =
            Stamp { center, normal: Vec3::Y, tangent: Vec3::X, bitangent: Vec3::Z, radius, flow: 1.0, hardness: 0.9, depth: 1.0, seed: 0, source: None };

        rasterize(&mut canvas, &context, &[stamp]);

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
        let uv = |point: Vec3| texturecraft_geom::glam::Vec2::new((point.x + 1.0) / 2.0, (point.z + 1.0) / 2.0);

        // The shelf maps to one corner point of the texture, so only the floor has texels
        let uvs = floor.iter().map(|p| uv(*p)).chain([texturecraft_geom::glam::Vec2::ONE; 4]).collect();

        // Wound to face up (+Y), toward the camera
        let quad_indices = [0u32, 2, 1, 0, 3, 2];
        let indices = quad_indices.iter().copied().chain(quad_indices.iter().map(|i| i + 4)).collect();
        let mesh = Mesh::from_parts(floor.into_iter().chain(shelf).collect(), None, uvs, indices, "floor and shelf").unwrap();

        for pitch in [60.0, 30.0] {
            let camera = looking_down(pitch);
            let open = Vec3::new(0.5, 0.0, -0.5);
            let open_coverage = painted(&mesh, &camera, open, 0.2, true, false);
            let visible_coverage = painted(&mesh, &camera, open, 0.2, true, true);

            assert!(open_coverage > 100, "the stamp covers the open half");
            assert_eq!(visible_coverage, open_coverage, "the open half takes the stamp at {pitch}°");

            let under = Vec3::new(-0.5, 0.0, -0.5);

            assert!(painted(&mesh, &camera, under, 0.2, true, false) > 100, "the stamp reaches under the shelf");
            assert_eq!(painted(&mesh, &camera, under, 0.2, true, true), 0, "the shelf hides the floor under it at {pitch}°");

            // The depth buffer cannot resolve the shelf edge; the exact ray decides
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

    /// Stamps straddling a sphere's perspective silhouette exercise visible and hidden texels.
    /// `cargo test --release -p texturecraft-paint --lib -- --ignored benchmark_silhouette --nocapture`
    #[test]
    #[ignore = "release performance measurement"]
    fn benchmark_silhouette_stamps() {
        let sphere = primitives::sphere(1.0, 256, 128);
        let mesh_bvh = Bvh::new(&sphere);
        let uv_buffer = UvGBuffer::new(&sphere, 0, 2048, 2048);
        let camera = looking_down(0.0);
        let depth = EyeDepth::render(&sphere, &camera, 960, 540);
        let context = RasterCtx {
            mesh: &sphere,
            bvh: &mesh_bvh,
            gbuf: &uv_buffer,
            material: 0,
            depth: Some(&depth),
            eye: camera.eye(),
            backface_culling: false,
            angle_fade: (89.0, 90.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing: Facing::Front,
            reach: Reach::Plane,
        };

        // A silhouette normal is perpendicular to the direction from the surface to the eye
        let silhouette_z = 1.0 / camera.eye().z;
        let silhouette_radius = (1.0 - silhouette_z * silhouette_z).sqrt();
        let stamps: Vec<Stamp> = (0..64)
            .map(|stamp_index| {
                let angle = stamp_index as f32 / 64.0 * std::f32::consts::TAU;
                let center = Vec3::new(silhouette_radius * angle.cos(), silhouette_radius * angle.sin(), silhouette_z);
                let tangent = (camera.eye() - center).normalize();
                Stamp {
                    center,
                    normal: center,
                    tangent,
                    bitangent: center.cross(tangent),
                    radius: 0.15,
                    flow: 1.0,
                    hardness: 0.9,
                    depth: 1.0,
                    seed: stamp_index,
                    source: None,
                }
            })
            .collect();

        let painted_count = |canvas: &StrokeCanvas| {
            canvas.alpha.allocated().filter_map(|(x, y)| canvas.alpha.tile(x, y)).map(|tile| tile.iter().filter(|&&alpha| alpha > 0).count()).sum::<usize>()
        };
        let mut visible = StrokeCanvas::new(2048, 2048);
        rasterize(&mut visible, &context, &stamps);
        let visible_texels = painted_count(&visible);
        drop(visible);
        let mut all = StrokeCanvas::new(2048, 2048);
        rasterize(&mut all, &RasterCtx { depth: None, ..context }, &stamps);
        let all_texels = painted_count(&all);
        drop(all);

        assert!(visible_texels > 0, "the benchmark must exercise visible paint");
        assert!(visible_texels < all_texels, "the benchmark must also reject hidden texels");
        eprintln!("silhouette coverage: {visible_texels} visible texels / {all_texels} without visibility");

        let mut elapsed_ms = Vec::new();
        for _ in 0..9 {
            let mut canvas = StrokeCanvas::new(2048, 2048);
            let start = std::time::Instant::now();
            rasterize(&mut canvas, &context, &stamps);
            elapsed_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }

        eprintln!("elapsed milliseconds: {elapsed_ms:?}");
        elapsed_ms.sort_by(f64::total_cmp);
        eprintln!("64 silhouette stamps at 2048²: median {:.2} ms, range {:.2}..{:.2} ms", elapsed_ms[4], elapsed_ms[0], elapsed_ms[8]);
    }

    #[test]
    fn one_stamp_on_a_plane_paints_a_disc() {
        let mesh = primitives::plane(2.0, 8);
        let bvh = Bvh::new(&mesh);
        let uv_buffer = UvGBuffer::new(&mesh, 0, 128, 128);
        let mut canvas = StrokeCanvas::new(128, 128);
        let context = RasterCtx {
            mesh: &mesh,
            bvh: &bvh,
            gbuf: &uv_buffer,
            material: 0,
            depth: None,
            eye: Vec3::new(0.0, 5.0, 0.0),
            backface_culling: true,
            angle_fade: (70.0, 85.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing: Facing::Front,
            reach: Reach::Plane,
        };
        let stamp = Stamp {
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

        rasterize(&mut canvas, &context, &[stamp]);

        // Plane is 2 units over 128 texels: radius 0.25 = 16 texels around the center (64, 64)
        let at = |x: u32, y: u32| from_u16(canvas.alpha.get(x, y)[0]);
        assert!(at(64, 64) > 0.99);
        assert!(at(64 + 12, 64) > 0.9);
        assert!(at(64 + 20, 64) < 0.01);
        assert!(!canvas.dirty.is_empty());

        // Backface culling keeps the underside unpainted
        let mut underside = StrokeCanvas::new(128, 128);
        let below = RasterCtx { eye: Vec3::new(0.0, -5.0, 0.0), ..context };

        rasterize(&mut underside, &below, &[stamp]);

        assert!(underside.dirty.is_empty());

        // its footprint is what it lays
        let texels = footprint(&context, &stamp);
        assert!(!texels.is_empty());
        assert!(texels.iter().all(|&(x, y, a)| (from_u16(canvas.alpha.get(x, y)[0]) - a.min(1.0)).abs() < 1e-4));
        let uv = UvStamp { center: texturecraft_geom::glam::Vec2::splat(0.5), radius: 0.125, angle: 0.0, flow: 0.5, hardness: 0.9, seed: 0, source: None };
        let mut c3 = StrokeCanvas::new(128, 128);
        rasterize_uv(&mut c3, &uv_buffer, context.shape, &[uv]);
        let texels = footprint_uv(&uv_buffer, context.shape, &uv);
        assert!(!texels.is_empty() && texels.iter().all(|&(x, y, a)| (from_u16(c3.alpha.get(x, y)[0]) - a).abs() < 1e-4));
    }

    #[test]
    fn facing_fades_or_reaches_surfaces_turned_from_the_stamp() {
        // a folded strip: the left half faces +Y, the right half turns down to face +X (a wall at x = 0)
        let mut m = primitives::plane(2.0, 16);
        for (p, n) in m.positions.iter_mut().zip(m.normals.iter_mut()) {
            if p.x > 0.0 {
                *p = Vec3::new(0.0, -p.x, p.z);
                *n = Vec3::X;
            }
        }
        let bvh = Bvh::new(&m);
        let g = UvGBuffer::new(&m, 0, 128, 128);
        let ctx = |facing| RasterCtx {
            mesh: &m,
            bvh: &bvh,
            gbuf: &g,
            material: 0,
            depth: None,
            eye: Vec3::new(1.0, 5.0, 0.0),
            backface_culling: false,
            angle_fade: (70.0, 85.0),
            shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
            facing,
            reach: Reach::Plane,
        };
        // a stamp on the fold, facing up: the wall turns a quarter from it (cosine 0)
        let st = Stamp {
            center: Vec3::new(0.0, 0.0, 0.0),
            normal: Vec3::Y,
            tangent: Vec3::X,
            bitangent: Vec3::Z,
            radius: 0.6,
            flow: 1.0,
            hardness: 0.9,
            depth: 1.0,
            seed: 0,
            source: None,
        };
        let wall = |facing| {
            let mut c = StrokeCanvas::new(128, 128);
            rasterize(&mut c, &ctx(facing), &[st]);
            // a wall texel a quarter below the fold (past the triangles whose normals blend across it), and a floor
            // texel as far the other way
            (from_u16(c.alpha.get(64 + 16, 64)[0]), from_u16(c.alpha.get(64 - 16, 64)[0]))
        };
        let (front, floor) = wall(Facing::Front);
        assert_eq!(front, 0.0, "a quarter turn is not on the stamp's side");
        assert!(floor > 0.9);
        let (half, floor) = wall(Facing::Fade { full: 0.25, none: -0.25 });
        assert!((half - 0.5).abs() < 0.05 && floor > 0.9, "cosine 0 sits halfway through the fade: {half}");
        let (none, _) = wall(Facing::Fade { full: 1.0, none: 0.5 });
        assert_eq!(none, 0.0);
        assert!(wall(Facing::Any).0 > 0.9);
    }

    #[test]
    fn surface_reach_follows_the_measured_rule() {
        // offset, centre normal, texel normal, radius, expected share (measured on stacked sheets and a cylinder)
        let z = [0.0, 0.0, 1.0];
        type Row = ([f32; 3], [f32; 3], [f32; 3], f32, f32);
        let rows: [Row; 8] = [
            ([0.05, 0.0, 0.0], z, z, 0.1, 0.5),
            ([0.05, 0.05, 0.0], z, z, 0.1, std::f32::consts::FRAC_1_SQRT_2),
            ([0.12, 0.0, 0.0], z, z, 0.1, 1.1999999),
            ([0.05, 0.0, -0.01], z, [0.38941833, 0.0, 0.921061], 0.1, 0.5101694),
            ([0.03, 0.0, -0.002], z, [0.19866933, 0.0, 0.9800666], 0.05, 0.6030125),
            ([0.0, 0.02, -0.03], z, z, 0.1, 0.21669295),
            ([0.02, 0.0, -0.05], z, z, 0.1, 0.2554041),
            ([0.0, 0.0, -0.2], z, z, 0.1, f32::INFINITY),
        ];
        for (offset, center, texel, radius, expected) in rows {
            let got = surface_reach(Vec3::from(offset), Vec3::from(center), Vec3::from(texel), radius);
            let agrees = if expected.is_infinite() { got.is_infinite() } else { (got - expected).abs() < 1e-5 };
            assert!(agrees, "{offset:?}: got {got}, expected {expected}");
        }
    }

    #[test]
    fn surface_reach_is_the_chord_on_a_curve_and_grows_off_it() {
        let n = Vec3::Z;
        assert!((surface_reach(Vec3::new(0.5, 0.0, 0.0), n, n, 1.0) - 0.5).abs() < 1e-6);
        // a point on a circle of radius 1 a turn `a` from the centre: its distance is the chord, longer than the flat offset
        let a = 0.6f32;
        let offset = Vec3::new(a.sin(), 0.0, a.cos() - 1.0);
        let got = surface_reach(offset, n, Vec3::new(a.sin(), 0.0, a.cos()), 1.0);
        assert!((got - offset.length()).abs() < 1e-5 && got > a.sin());
        // straight under the centre stays at the centre; off to the side it nears the rim; too far off is out of reach
        assert!(surface_reach(Vec3::new(0.0, 0.0, -0.3), n, n, 0.5) < 1e-6);
        assert!(surface_reach(Vec3::new(0.2, 0.0, -0.3), n, n, 0.5) > 0.4 * 1.4);
        assert!(surface_reach(Vec3::new(0.1, 0.0, -0.6), n, n, 0.5).is_infinite());
        // no radius, no reach (and no panic on hostile values)
        for radius in [0.0, -1.0, f32::NAN] {
            assert!(surface_reach(Vec3::X, n, n, radius).is_infinite());
        }
        let hostile = surface_reach(Vec3::splat(f32::NAN), n, n, 1.0);
        assert!(hostile.is_nan() || hostile >= 1.0, "a hostile offset reaches nothing");
    }

    #[test]
    fn surface_reach_fades_a_sheet_behind_and_plane_reach_does_not() {
        // two 2 × 2 sheets facing +Y, one `gap` under the other; the top on the left half of the texture, the bottom on the right
        let radius = 0.4;
        let gap = 0.5 * radius;
        let mut m = primitives::plane(2.0, 16);
        let mut under = m.clone();
        for uv in &mut m.uvs {
            uv.x *= 0.5;
        }
        for (p, uv) in under.positions.iter_mut().zip(under.uvs.iter_mut()) {
            p.y = -gap;
            uv.x = 0.5 + uv.x * 0.5;
        }
        under.materials.clear();
        under.submeshes.clear();
        m.append(&under);
        let count = m.indices.len() as u32;
        if let Some(s) = m.submeshes.first_mut() {
            s.count = count;
        }
        let bvh = Bvh::new(&m);
        let g = UvGBuffer::new(&m, 0, 256, 128);
        let st = Stamp {
            center: Vec3::ZERO,
            normal: Vec3::Y,
            tangent: Vec3::X,
            bitangent: Vec3::Z,
            radius,
            flow: 1.0,
            hardness: 0.5,
            depth: 1.0,
            seed: 0,
            source: None,
        };
        let laid = |reach| {
            let ctx = RasterCtx {
                mesh: &m,
                bvh: &bvh,
                gbuf: &g,
                material: 0,
                depth: None,
                eye: Vec3::new(0.0, 5.0, 0.0),
                backface_culling: false,
                angle_fade: (90.0, 90.0),
                shape: Shape::new(crate::BrushAlpha::Round, None, None, Default::default()),
                facing: Facing::Any,
                reach,
            };
            let mut c = StrokeCanvas::new(256, 128);
            rasterize(&mut c, &ctx, &[st]);
            let sum = |x0: u32| (x0..x0 + 128).flat_map(|x| (0..128).map(move |y| (x, y))).map(|(x, y)| from_u16(c.alpha.get(x, y)[0])).sum::<f32>();
            // the top sheet, the sheet behind, and a texel of the sheet behind about 0.6 radii off the centre (15 texels of 64 per unit)
            (sum(0), sum(128), from_u16(c.alpha.get(128 + 64, 64 + 15)[0]))
        };
        let (top_plane, under_plane, side_plane) = laid(Reach::Plane);
        let (top_surface, under_surface, side_surface) = laid(Reach::Surface);
        assert!((top_plane - top_surface).abs() < 1e-3 * top_plane, "the dab's own surface is the same: {top_plane} vs {top_surface}");
        assert!((under_plane - top_plane).abs() < 0.02 * top_plane, "the plane reach lays the same disc on the sheet behind");
        assert!(under_surface < 0.8 * under_plane && under_surface > 0.0, "the sheet behind fades: {under_surface} vs {under_plane}");
        assert!(side_surface < side_plane, "off the centre the sheet behind nears the rim: {side_surface} vs {side_plane}");
        assert_eq!(Reach::default(), Reach::Plane);
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
    /// The symmetry copy this dab belongs to (None: the stroke itself), so a smudge's copies each
    /// drag their own trail as in the 3D view
    pub source: Option<StampSource>,
}

impl UvStamp {
    /// This stamp as a stamp on the mesh, at the surface under its centre: its texture's x and up as the tip's axes
    /// (turned by its angle), its radius `world_per_uv` mesh units per unit of texture. For laying a 2D-view stamp
    /// where only the mesh can say where it goes, such as a symmetry copy. `None` off the texture set.
    pub fn on_mesh(&self, mesh: &Mesh, gbuf: &UvGBuffer, world_per_uv: f32) -> Option<Stamp> {
        let (w, h) = (gbuf.width, gbuf.height);
        let (x, y) = (self.center.x * w as f32, self.center.y * h as f32);
        if !(x >= 0.0 && y >= 0.0 && x < w as f32 && y < h as f32) {
            return None;
        }
        let i = y as usize * w as usize + x as usize;
        let surface = gbuf.surface(mesh, i)?;
        let triangle = *gbuf.tri.get(i)?;
        let [a, b, c] = mesh.tri(triangle as usize)?;
        let (pa, pb, pc) = (mesh.positions.get(a)?, mesh.positions.get(b)?, mesh.positions.get(c)?);
        let (ua, ub, uc) = (mesh.uvs.get(a)?, mesh.uvs.get(b)?, mesh.uvs.get(c)?);
        // the surface's change along the texture's u and v
        let (e1, e2, d1, d2) = (*pb - *pa, *pc - *pa, *ub - *ua, *uc - *ua);
        let det = d1.perp_dot(d2);
        if det.abs() < f32::MIN_POSITIVE {
            return None;
        }
        let along_u = (e1 * d2.y - e2 * d1.y) / det;
        let along_v = (e2 * d1.x - e1 * d2.x) / det;
        let normal = surface.nrm.try_normalize()?;
        let flat = |v: Vec3| (v - normal * normal.dot(v)).try_normalize();
        // texel rows run down: the tip's up is against v
        let (right, up) = (flat(along_u)?, flat(-along_v)?);
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let tangent = (right * cos + up * sin).try_normalize()?;
        let radius = self.radius * world_per_uv;
        (radius.is_finite() && radius > 0.0).then_some(Stamp {
            center: surface.pos,
            normal,
            tangent,
            bitangent: normal.cross(tangent),
            radius,
            flow: self.flow,
            hardness: self.hardness,
            depth: 1.0,
            seed: self.seed,
            source: self.source,
        })
    }
}

/// Accumulate UV-space stamps into `canvas` (only texels covered by the texture set).
pub fn rasterize_uv(canvas: &mut StrokeCanvas, gbuf: &UvGBuffer, shape: Shape, stamps: &[UvStamp]) {
    canvas.fit(gbuf);
    for s in stamps {
        let mut texels = std::mem::take(&mut canvas.texels);
        let center = uv_texels(gbuf, shape, s, &mut texels);
        canvas.texels = texels;
        if let Some(center) = center {
            canvas.add_stamp(center, s.source);
        }
    }
}

/// The texels one UV-space stamp reaches, `(x, y, alpha)`: what [`rasterize_uv`] lays for it, without laying it.
pub fn footprint_uv(gbuf: &UvGBuffer, shape: Shape, stamp: &UvStamp) -> Vec<(u32, u32, f32)> {
    let mut texels = Vec::new();
    uv_texels(gbuf, shape, stamp, &mut texels);
    texels
}

/// Appends the texels `s` reaches to `texels`; its centre in texels, `None` for a stamp that is no stamp.
fn uv_texels(gbuf: &UvGBuffer, shape: Shape, s: &UvStamp, texels: &mut Vec<(u32, u32, f32)>) -> Option<[f32; 2]> {
    let (w, h) = (gbuf.width, gbuf.height);
    if s.radius.is_nan() || s.radius <= 0.0 || !s.center.is_finite() {
        return None;
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
                texels.push((x, y, a));
            }
        }
    }
    Some([cx, cy])
}
