//! Replays an imported project's brush strokes with TextureCraft's brush ([`BrushReplayer`]): the engine step of
//! `file.importProject` that turns the translator's replay jobs ([`texturecraft_import::replay`]) into pixels.
//!
//! The translator sits below the brush engine and rasterises nothing; it hands every job to this replayer per texture
//! set, before its safety net composites. Each stroke is laid as TextureCraft paints:
//!
//! - **3D view**: the stroke's recorded camera becomes an orbit camera (its roll turned into the recorded points, an
//!   off-centre projection shifted out) on a view [`VIEWPORT_HEIGHT`] pixels high; the pen's points feed a
//!   [`texturecraft_paint::Stroke`] (picking, lazy mouse, spacing along the surface, jitter drawn from the stroke's seed
//!   as the file's app draws it), picking only the set's own triangles.
//! - **2D view**: dabs walked along the path in texels, spaced by the set's average density (measured), jittered from
//!   the stroke's seed, laid by [`texturecraft_paint::rasterize_uv`].
//! - **Symmetry**: copies of either view's dabs are the recorded matrix applied once more per copy, laid on the mesh
//!   (2D-view dabs at the surface under their centre, [`texturecraft_paint::UvStamp::on_mesh`]).
//! - **Dynamic alphas**: dab `k` lays stamp `k + start` of the cycle (`start` drawn from the seed where the alpha says).
//! - **Stencil**: once a stroke's dabs are laid, its coverage is multiplied by the stencil where each texel lies on the
//!   stroke's view ([`texturecraft_paint::ViewStencil`]).
//! - Dabs reach every surface in their volume whatever the view sees (measured: nothing occludes), faded by the turn
//!   from the dab's normal as the brush's culling says ([`Facing::Fade`]); each stroke is laid on its own and committed
//!   with the effect's opacity ("over" for paint, coverage lowered for the eraser).
//!
//! Everything runs on rayon; no files, threads or clocks, so it works in the web build.

use std::collections::BTreeMap;

use rayon::prelude::*;
use texturecraft_compose::PixelStore;
use texturecraft_core::Channel;
use texturecraft_geom::glam::{Mat4, Vec2, Vec3};
use texturecraft_geom::{Bvh, Mesh, NO_TRI, OrbitCamera, SubMesh, UvGBuffer};
use texturecraft_import::replay::{
    PaintMode, ReplayBrush, ReplayJob, ReplayScene, ReplayStencil, ReplayStep, ReplayStroke, ReplayTarget, ReplayTip, ReplayedPixels, StencilImage,
    StrokeReplayer, StrokeSpace, TipAlignment,
};
use texturecraft_paint::alphas::AlphaImage;
use texturecraft_paint::jitter::{Jittered, StrokeRandom, cycle_start};
use texturecraft_paint::{
    Alignment, BrushAlpha, BrushSettings, CommitSource, Facing, PickCtx, RasterCtx, SeededJitter, Shape, Staging, Stamp, Stroke, StrokeCanvas, UvStamp,
    ViewStencil, commit_layer, commit_mask, pressure_scale, rasterize, rasterize_uv,
};
use texturecraft_texel::TileImage;

/// Height of the view 3D-view strokes are placed on, in pixels: only how finely the pen's path is followed.
pub const VIEWPORT_HEIGHT: f32 = 2048.0;
/// 2D-view steps run this much short of `spacing × 2R` (measured on a long stroke; not explained).
const UV_STEP: f32 = 0.99986;
/// Off the surface, a 2D-view stroke goes on in steps of this share of the texture (measured).
const OFF_SURFACE_STEP: f32 = 0.01;
/// Culling fades over this half width in the cosine around the culling angle's (measured).
const CULLING_HALF_WIDTH: f32 = 0.25;
/// Most dabs one stroke lays (a guard against degenerate steps in hostile files).
const MAX_DABS: usize = 1 << 20;
/// Pen points fed to the brush per batch before its dabs are laid (bounds the memory one stroke holds).
const POINTS_PER_BATCH: usize = 64;
/// Steepest pitch the orbit camera takes (it looks along the vertical badly).
const MAX_PITCH_DEGREES: f32 = 89.95;

/// The replayer `file.importProject` gives the import.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrushReplayer;

impl StrokeReplayer for BrushReplayer {
    fn replay(&self, scene: &ReplayScene, jobs: &[ReplayJob]) -> Vec<Result<ReplayedPixels, String>> {
        replay_jobs(scene, jobs, &|| false)
    }
}

/// [`BrushReplayer`] that stops when its flag is set: the stroke being laid finishes, the job fails as cancelled, and
/// so do the jobs not started (the import checks the same flag and discards the set).
pub struct CancellableReplayer(pub texturecraft_import::Cancel);

impl StrokeReplayer for CancellableReplayer {
    fn replay(&self, scene: &ReplayScene, jobs: &[ReplayJob]) -> Vec<Result<ReplayedPixels, String>> {
        replay_jobs(scene, jobs, &|| self.0.is_cancelled())
    }
}

fn replay_jobs(scene: &ReplayScene, jobs: &[ReplayJob], stop: &(dyn Fn() -> bool + Sync)) -> Vec<Result<ReplayedPixels, String>> {
    let Some(set) = SetMesh::new(scene) else {
        return jobs.iter().map(|_| Err("the texture set has no triangles to paint".to_string())).collect();
    };
    jobs.par_iter().map(|job| replay_job(&set, job, stop)).collect()
}

/// The texture set's own triangles with what painting them needs.
struct SetMesh {
    mesh: Mesh,
    bvh: Bvh,
    gbuf: UvGBuffer,
    material: usize,
    resolution: u32,
    /// A texel's side on the mesh, on average over the set.
    world_per_texel: f32,
    center: Vec3,
    radius: f32,
}

impl SetMesh {
    fn new(scene: &ReplayScene) -> Option<SetMesh> {
        let source = scene.mesh;
        let mut mesh = Mesh { positions: source.positions.clone(), normals: source.normals.clone(), uvs: source.uvs.clone(), ..Mesh::default() };
        mesh.tangents = source.tangents.clone();
        mesh.materials = source.materials.clone();
        for s in source.submeshes.iter().filter(|s| s.material == scene.material) {
            let range = s.start as usize..(s.start as usize).saturating_add(s.count as usize);
            let Some(indices) = source.indices.get(range) else { continue };
            let start = u32::try_from(mesh.indices.len()).ok()?;
            mesh.indices.extend_from_slice(indices);
            mesh.submeshes.push(SubMesh { name: s.name.clone(), material: s.material, start, count: s.count });
        }
        if mesh.indices.is_empty() {
            return None;
        }
        let res = scene.resolution.max(1);
        let gbuf = UvGBuffer::new(&mesh, scene.material, res, res);
        let bvh = Bvh::new(&mesh);
        let (mut world, mut uv) = (0.0f64, 0.0f64);
        for t in mesh.triangles_of(scene.material) {
            let Some([a, b, c]) = mesh.tri(t) else { continue };
            let (Some(pa), Some(pb), Some(pc)) = (mesh.positions.get(a), mesh.positions.get(b), mesh.positions.get(c)) else { continue };
            let (Some(ua), Some(ub), Some(uc)) = (mesh.uvs.get(a), mesh.uvs.get(b), mesh.uvs.get(c)) else { continue };
            world += f64::from((*pb - *pa).cross(*pc - *pa).length());
            uv += f64::from((*ub - *ua).perp_dot(*uc - *ua).abs());
        }
        let world_per_uv = if uv > 0.0 && world > 0.0 { (world / uv).sqrt() as f32 } else { 1.0 };
        let (lo, hi) = source.bounds();
        Some(SetMesh {
            gbuf,
            bvh,
            material: scene.material,
            resolution: res,
            world_per_texel: (world_per_uv / res as f32).max(1e-9),
            center: (lo + hi) * 0.5,
            radius: ((hi - lo).length() * 0.5).max(1e-3),
            mesh,
        })
    }

    fn size(&self) -> (u32, u32) {
        (self.resolution, self.resolution)
    }
}

/// A step's tip, ready to sample.
struct Tip {
    alpha: BrushAlpha,
    hardness: f32,
    image: Option<AlphaImage>,
    /// A dynamic alpha's stamps, laid in turn (empty: every dab lays the tip).
    cycle: Vec<AlphaImage>,
    drawn_start: bool,
}

fn tip_image(width: u32, height: u32, values: &[f32]) -> Result<AlphaImage, String> {
    AlphaImage::from_pixels(width as usize, height as usize, values).ok_or_else(|| "the brush tip image is empty".to_string())
}

impl Tip {
    fn of(brush: &ReplayBrush) -> Result<Tip, String> {
        let mut cycle = Vec::with_capacity(brush.cycle.len());
        for stamp in &brush.cycle {
            match stamp {
                ReplayTip::Image { width, height, values } => cycle.push(tip_image(*width, *height, values)?),
                ReplayTip::Round { .. } => return Err("a dynamic alpha's stamp is not an image".into()),
            }
        }
        let drawn_start = brush.cycle_drawn_start;
        match &brush.tip {
            ReplayTip::Round { hardness } => Ok(Tip { alpha: BrushAlpha::Round, hardness: *hardness, image: None, cycle, drawn_start }),
            ReplayTip::Image { width, height, values } => {
                Ok(Tip { alpha: BrushAlpha::Round, hardness: 0.5, image: Some(tip_image(*width, *height, values)?), cycle, drawn_start })
            }
        }
    }

    fn shape(&self) -> Shape<'_> {
        Shape { alpha: self.alpha, image: None, library: self.image.as_ref(), level: 0 }
    }

    /// Where a stroke with `seed` starts the cycle.
    fn start(&self, seed: u32) -> usize {
        if self.drawn_start { cycle_start(seed, self.cycle.len()) } else { 0 }
    }

    /// `dabs` by the shape they lay: dab `k` (its `seed(dab)`) lays stamp `k + start` of the cycle; without a cycle,
    /// all lay the tip.
    fn by_stamp<T: Copy>(&self, dabs: &[T], start: usize, seed: impl Fn(&T) -> u32) -> Vec<(Shape<'_>, Vec<T>)> {
        let n = self.cycle.len();
        if n == 0 {
            return vec![(self.shape(), dabs.to_vec())];
        }
        let mut groups: Vec<Vec<T>> = vec![Vec::new(); n];
        for d in dabs {
            if let Some(g) = groups.get_mut((seed(d) as usize).wrapping_add(start) % n) {
                g.push(*d);
            }
        }
        groups.into_iter().zip(&self.cycle).filter(|(g, _)| !g.is_empty()).map(|(g, image)| (Shape { library: Some(image), ..self.shape() }, g)).collect()
    }
}

/// A step's stencil, ready to sample.
fn view_stencil(stencil: &ReplayStencil) -> Result<ViewStencil, String> {
    match &stencil.image {
        StencilImage::Uniform(v) => Ok(ViewStencil::uniform(*v)),
        StencilImage::Image { width, height, values } => {
            ViewStencil::image(*width as usize, *height as usize, values.clone(), stencil.matrix).ok_or_else(|| "the stencil image is empty".to_string())
        }
    }
}

/// Multiplies a laid stroke's coverage by `stencil` where each texel lies on the stroke's view: through the recorded
/// camera for the 3D view (nothing behind it), the texture spanning the view for the 2D view.
fn through(set: &SetMesh, stroke: &ReplayStroke, stencil: &ViewStencil, canvas: &mut StrokeCanvas) {
    let res = set.resolution;
    match &stroke.space {
        StrokeSpace::Mesh { view, projection } => {
            let view_projection = Mat4::from_cols_array(projection) * Mat4::from_cols_array(view);
            canvas.scale(|x, y| {
                let Some(surface) = set.gbuf.surface(&set.mesh, y as usize * res as usize + x as usize) else { return 0.0 };
                let clip = view_projection * surface.pos.extend(1.0);
                if clip.w.is_nan() || clip.w <= 1e-6 {
                    return 0.0;
                }
                stencil.at(Vec2::new(clip.x / clip.w, clip.y / clip.w))
            });
        }
        StrokeSpace::Uv => {
            let r = res as f32;
            canvas.scale(|x, y| stencil.at(Vec2::new((x as f32 + 0.5) / r * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / r * 2.0)));
        }
    }
}

/// Lays every step of `job` into its target.
fn replay_job(set: &SetMesh, job: &ReplayJob, stop: &(dyn Fn() -> bool + Sync)) -> Result<ReplayedPixels, String> {
    let mut pixels = PixelStore::default();
    let id = job.target.id();
    for step in &job.steps {
        let tip = Tip::of(&step.brush)?;
        let stencil = step.stencil.as_ref().map(view_stencil).transpose()?;
        for stroke in &step.strokes {
            if stop() {
                return Err("cancelled".into());
            }
            let mut canvas = StrokeCanvas::new(set.resolution, set.resolution);
            lay_stroke(set, step, &tip, stroke, &mut canvas);
            if let Some(stencil) = &stencil {
                through(set, stroke, stencil, &mut canvas);
            }
            if canvas.dirty.is_empty() {
                continue;
            }
            commit(&mut pixels, set, job.target, step, &canvas);
        }
    }
    Ok(match job.target {
        ReplayTarget::Content(_) => {
            let mut images: Vec<(Channel, TileImage)> =
                pixels.layers.into_iter().filter(|((owner, _), _)| *owner == id).map(|((_, c), img)| (c, img)).collect();
            images.sort_by_key(|(c, _)| *c);
            ReplayedPixels::Content(images)
        }
        ReplayTarget::Mask(_) => ReplayedPixels::Mask(pixels.masks.remove(&id).unwrap_or_else(|| TileImage::new(set.resolution, set.resolution, 2))),
    })
}

/// Commits one stroke's coverage into the target with the step's values and opacity.
fn commit(pixels: &mut PixelStore, set: &SetMesh, target: ReplayTarget, step: &ReplayStep, canvas: &StrokeCanvas) {
    let source = |opacity: f32| CommitSource {
        alpha: &canvas.alpha,
        tiles: &canvas.dirty,
        opacity: (step.opacity * opacity).clamp(0.0, 1.0),
        erase: step.mode == PaintMode::Erase,
        per_texel: None,
        carried: None,
    };
    match target {
        ReplayTarget::Mask(id) => commit_mask(pixels, id, set.size(), step.mask_value, source(1.0), &mut Staging::default()),
        ReplayTarget::Content(id) => {
            // channels with their own opacity commit apart
            let mut groups: Vec<(f32, BTreeMap<Channel, [f32; 3]>)> = Vec::new();
            for (c, v, o) in &step.values {
                match groups.iter_mut().find(|(g, _)| (*g - *o).abs() < 1e-6) {
                    Some((_, values)) => {
                        values.insert(*c, *v);
                    }
                    None => groups.push((*o, BTreeMap::from([(*c, *v)]))),
                }
            }
            for (o, values) in groups {
                commit_layer(pixels, id, set.size(), &values, source(o), &mut Staging::default());
            }
        }
    }
}

/// Lays one stroke's dabs into `canvas`.
fn lay_stroke(set: &SetMesh, step: &ReplayStep, tip: &Tip, stroke: &ReplayStroke, canvas: &mut StrokeCanvas) {
    match &stroke.space {
        StrokeSpace::Mesh { view, projection } => {
            if let Some(camera) = ViewCamera::new(view, projection, set.center, set.radius) {
                lay_mesh_stroke(set, &step.brush, tip, stroke, &camera, canvas);
            }
        }
        StrokeSpace::Uv => lay_uv_stroke(set, &step.brush, tip, stroke, canvas),
    }
}

/// The fade by the turn from the dab's normal the brush's culling asks for (measured): a smoothstep 0.5 wide in the
/// cosine around the culling angle's; without culling every surface in reach.
fn facing(brush: &ReplayBrush) -> Facing {
    match brush.culling {
        Some(share) => {
            let limit = (share.clamp(0.0, 1.0) * std::f32::consts::PI).cos();
            Facing::Fade { full: limit + CULLING_HALF_WIDTH, none: limit - CULLING_HALF_WIDTH }
        }
        None => Facing::Any,
    }
}

/// TextureCraft's brush settings for `brush`: the file's jitter ranges mapped onto TextureCraft's (size `1 − J·u`
/// becomes a radius scaled by `1 − J/2` with TextureCraft's symmetric jitter `±(J/2)/(1 − J/2)`, the same draw for
/// draw; rotation in turns of a half turn; position in radii); pen pressure on size and flow; the rotation turned
/// counter-clockwise as TextureCraft's angle is.
pub(crate) fn settings(brush: &ReplayBrush, lazy_pixels: f32) -> (BrushSettings, f32) {
    let size_scale = 1.0 - brush.size_jitter * 0.5;
    let settings = BrushSettings {
        min_size: brush.size_pressure.unwrap_or(1.0),
        min_flow: brush.flow_pressure.unwrap_or(1.0),
        flow: brush.flow,
        opacity: 1.0,
        spacing: brush.spacing,
        hardness: 0.5,
        angle: -brush.rotation.to_degrees(),
        follow_path: brush.follow,
        alignment: match brush.alignment {
            TipAlignment::Camera => Alignment::Camera,
            TipAlignment::Tangent | TipAlignment::Uv => Alignment::Tangent,
        },
        lazy: lazy_pixels,
        jitter_size: if size_scale > 0.0 { brush.size_jitter * 0.5 / size_scale } else { 0.0 },
        jitter_angle: brush.rotation_jitter / std::f32::consts::PI,
        jitter_flow: brush.flow_jitter,
        jitter_position: brush.position_jitter * 2.0,
        backface_culling: false,
        ..BrushSettings::default()
    };
    (settings, size_scale.max(0.0))
}

/// A recorded 3D-view camera as an orbit camera with the place of its points on the orbit camera's view.
struct ViewCamera {
    orbit: OrbitCamera,
    viewport: Vec2,
    aspect: f32,
    /// The recorded camera's roll from the orbit camera's up (sine, cosine).
    roll: (f32, f32),
    /// The projection's off-centre shift in normalised device coordinates.
    shift: Vec2,
}

impl ViewCamera {
    /// `None` for matrices that are no camera.
    fn new(view: &[f32; 16], projection: &[f32; 16], center: Vec3, radius: f32) -> Option<ViewCamera> {
        if !view.iter().chain(projection.iter()).all(|v| v.is_finite()) {
            return None;
        }
        let v = Mat4::from_cols_array(view);
        let p = Mat4::from_cols_array(projection);
        if v.determinant().abs() < 1e-12 {
            return None;
        }
        let inv = v.inverse();
        let eye = inv.transform_point3(Vec3::ZERO);
        let forward = inv.transform_vector3(Vec3::NEG_Z).try_normalize()?;
        let up = inv.transform_vector3(Vec3::Y).try_normalize()?;
        let (m00, m11) = (p.x_axis.x, p.y_axis.y);
        if m00.is_nan() || m11.is_nan() || m00 <= 1e-9 || m11 <= 1e-9 {
            return None;
        }
        let aspect = m11 / m00;
        let ortho = p.w_axis.w.abs() > 0.5 && p.z_axis.w.abs() < 0.5;
        let distance = (center - eye).dot(forward).max(radius * 0.05).max(1e-4);
        let (fov, shift) = match ortho {
            true => ((2.0 * (1.0 / m11 / distance).atan()).to_degrees(), Vec2::new(-p.w_axis.x, -p.w_axis.y)),
            false => ((2.0 * (1.0 / m11).atan()).to_degrees(), Vec2::new(p.z_axis.x, p.z_axis.y)),
        };
        if !fov.is_finite() || fov <= 0.0 || !aspect.is_finite() {
            return None;
        }
        let back = -forward;
        let limit = MAX_PITCH_DEGREES.to_radians();
        let pitch = back.y.clamp(-1.0, 1.0).asin().clamp(-limit, limit);
        let yaw = back.x.atan2(back.z);
        let orbit = OrbitCamera { target: eye + forward * distance, yaw, pitch, distance, fov, scene_radius: radius, ortho };
        let f0 = orbit.forward();
        let right0 = f0.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up0 = right0.cross(f0);
        let roll = (-up.dot(right0), up.dot(up0));
        Some(ViewCamera { orbit, viewport: Vec2::new(VIEWPORT_HEIGHT * aspect, VIEWPORT_HEIGHT), aspect, roll, shift })
    }

    /// The pixel on the orbit camera's view of a recorded point (normalised device coordinates).
    fn screen(&self, ndc: [f32; 2]) -> Vec2 {
        let (x, y) = (ndc[0] + self.shift.x, ndc[1] + self.shift.y);
        let (s, c) = self.roll;
        let (a, b) = (x * self.aspect, y);
        let (x0, y0) = ((a * c - b * s) / self.aspect, a * s + b * c);
        Vec2::new((x0 * 0.5 + 0.5) * self.viewport.x, (0.5 - y0 * 0.5) * self.viewport.y)
    }
}

/// What the stroke's seed drives: TextureCraft's brush draws the jitter from it as the file's app does.
fn seeded(brush: &ReplayBrush, stroke: &ReplayStroke) -> SeededJitter {
    SeededJitter { seed: stroke.seed, both_axes: brush.position_both, normal: brush.position_normal }
}

/// The rasteriser's view of the set for `shape`: every surface in a dab's volume, faded as the brush culls.
fn raster_ctx<'a>(set: &'a SetMesh, brush: &ReplayBrush, eye: Vec3, shape: Shape<'a>) -> RasterCtx<'a> {
    RasterCtx {
        mesh: &set.mesh,
        bvh: &set.bvh,
        gbuf: &set.gbuf,
        material: set.material,
        depth: None,
        eye,
        backface_culling: false,
        angle_fade: (90.0, 90.0),
        shape,
        facing: facing(brush),
    }
}

/// Lays mesh-space `dabs` into `canvas`, each with its stamp of the tip's cycle.
fn lay_dabs(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, start: usize, eye: Vec3, dabs: &[Stamp], canvas: &mut StrokeCanvas) {
    for (shape, group) in tip.by_stamp(dabs, start, |d| d.seed) {
        rasterize(canvas, &raster_ctx(set, brush, eye, shape), &group);
    }
}

/// A 3D-view stroke through TextureCraft's [`Stroke`], fed point by point, dabs laid batch by batch.
fn lay_mesh_stroke(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, stroke: &ReplayStroke, camera: &ViewCamera, canvas: &mut StrokeCanvas) {
    let (mut settings, size_scale) = settings(brush, stroke.lazy * VIEWPORT_HEIGHT);
    settings.hardness = tip.hardness;
    settings.alpha = tip.alpha;
    let mut pen = Stroke::new(&settings, &set.mesh).seeded(seeded(brush, stroke));
    pen.radius = brush.radius * size_scale;
    let ctx = PickCtx { mesh: &set.mesh, bvh: &set.bvh, camera: &camera.orbit, viewport: camera.viewport };
    let (eye, start) = (camera.orbit.eye(), tip.start(stroke.seed));
    let copies = symmetry(stroke);
    let mut laid = 0usize;
    for points in stroke.points.chunks(POINTS_PER_BATCH) {
        let mut dabs = Vec::new();
        for p in points {
            dabs.extend(pen.add(&ctx, camera.screen(p.position), p.pressure));
        }
        pen.stamps.clear();
        let originals = dabs.len();
        for m in &copies {
            for i in 0..originals {
                if let Some(d) = dabs.get(i).copied().and_then(|d| copied(&d, m)) {
                    dabs.push(d);
                }
            }
        }
        laid = laid.saturating_add(dabs.len());
        lay_dabs(set, brush, tip, start, eye, &dabs, canvas);
        if laid > MAX_DABS {
            break;
        }
    }
}

/// The symmetry copies' matrices: the recorded step applied once, twice, … `copies` times.
fn symmetry(stroke: &ReplayStroke) -> Vec<Mat4> {
    let step = Mat4::from_cols_array(&stroke.copy_matrix);
    if stroke.copies == 0 || !step.is_finite() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut power = step;
    for _ in 0..stroke.copies {
        out.push(power);
        power = step * power;
    }
    out
}

/// A dab moved by a symmetry matrix: its centre as a point, its axes as directions.
fn copied(d: &Stamp, m: &Mat4) -> Option<Stamp> {
    let center = m.project_point3(d.center);
    let normal = m.transform_vector3(d.normal).try_normalize()?;
    let tangent = m.transform_vector3(d.tangent).try_normalize()?;
    let bitangent = m.transform_vector3(d.bitangent).try_normalize()?;
    center.is_finite().then_some(Stamp { center, normal, tangent, bitangent, ..*d })
}

/// The lazy mouse on a path: the brush stays put until the string of `length` is taut.
fn trail(points: &[(Vec2, f32)], length: f32) -> Vec<(Vec2, f32)> {
    if length.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(points.len());
    let Some(&(mut brush, p0)) = points.first() else { return out };
    out.push((brush, p0));
    for &(p, pressure) in points.iter().skip(1) {
        let d = p - brush;
        let l = d.length();
        if l > length {
            brush += d * ((l - length) / l);
        }
        out.push((brush, pressure));
    }
    out
}

/// A 2D-view stroke: dabs walked along the path in texels, each step `spacing × 2R` at the set's average density
/// (measured; `R` the dab's radius after pressure and jitter), a fixed share of the texture while the path is off the
/// islands. Every spot draws its jitter from the stroke's seed, on the islands or not (measured); a dab moved off them
/// is not laid. Symmetry copies are laid on the mesh.
fn lay_uv_stroke(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, stroke: &ReplayStroke, canvas: &mut StrokeCanvas) {
    let res = set.resolution as f32;
    let points: Vec<(Vec2, f32)> = stroke.points.iter().map(|p| (Vec2::new(p.position[0], p.position[1]) * res, p.pressure)).collect();
    let points = trail(&points, stroke.lazy * res);
    let (settings, _) = settings(brush, 0.0);
    let radius = brush.radius / set.world_per_texel;
    let on_surface = |p: Vec2| {
        let (x, y) = (p.x.floor(), p.y.floor());
        x >= 0.0 && y >= 0.0 && x < res && y < res && set.gbuf.tri.get(y as usize * set.resolution as usize + x as usize).is_some_and(|t| *t != NO_TRI)
    };
    let mut random = StrokeRandom::new(seeded(brush, stroke));
    let jittered =
        Jittered { size: brush.size_jitter > 0.0, flow: brush.flow_jitter > 0.0, rotation: brush.rotation_jitter > 0.0, position: brush.position_jitter > 0.0 };
    let mut dabs: Vec<UvStamp> = Vec::new();
    let mut spots = 0u32;
    let mut lay = |at: Vec2, heading: Vec2, pressure: f32, dabs: &mut Vec<UvStamp>| -> f32 {
        let d = random.draw(jittered);
        let r = radius * pressure_scale(brush.size_pressure.unwrap_or(1.0), pressure) * (1.0 - brush.size_jitter * (1.0 - d.size));
        let flow = brush.flow * pressure_scale(brush.flow_pressure.unwrap_or(1.0), pressure) * (1.0 - brush.flow_jitter * (1.0 - d.flow));
        // moved by shares of the diameter: along both axes in a drawn direction, or across the stroke (rows down)
        let distance = brush.position_jitter * d.distance;
        let (moved, across) = match (jittered.position, brush.position_both) {
            (false, _) => (Vec2::ZERO, 0.0),
            (true, true) => {
                let (sin, cos) = (std::f32::consts::PI * (d.direction - 0.5)).sin_cos();
                (Vec2::new(distance * sin, distance * cos), 0.0)
            }
            (true, false) => (Vec2::ZERO, distance),
        };
        let center = at + (moved + Vec2::new(-heading.y, heading.x) * across) * 2.0 * r;
        // the file's turn is clockwise on the screen, TextureCraft's angle counter-clockwise; y up on the screen, rows down
        let turn = brush.rotation_jitter * (1.0 - 2.0 * d.rotation);
        let follow = if settings.follow_path { (-heading.y).atan2(heading.x).to_degrees() - 90.0 } else { 0.0 };
        let angle = settings.angle - turn.to_degrees() + follow;
        if on_surface(center) {
            dabs.push(UvStamp { center: center / res, radius: r / res, angle, flow: flow.clamp(0.0, 1.0), hardness: tip.hardness, seed: spots });
        }
        spots = spots.wrapping_add(1);
        if on_surface(at) { (brush.spacing.max(0.01) * 2.0 * r * UV_STEP).max(1e-3) } else { OFF_SURFACE_STEP * res }
    };
    let Some(&(first, p0)) = points.first() else { return };
    // the first spot faces along the path's first move
    let heading = points.windows(2).find_map(|w| match w {
        [(a, _), (b, _)] => (*b - *a).try_normalize(),
        _ => None,
    });
    let mut left = lay(first, heading.unwrap_or(Vec2::X), p0, &mut dabs);
    let mut walked = 0usize;
    'walk: for pair in points.windows(2) {
        let &[(a, pa), (b, pb)] = pair else { continue };
        let d = b - a;
        let l = d.length();
        if l.is_nan() || l <= 0.0 {
            continue;
        }
        let u = d / l;
        let mut t = 0.0;
        while l - t >= left {
            t += left;
            let f = t / l;
            left = lay(a + u * t, u, pa + (pb - pa) * f, &mut dabs);
            walked += 1;
            if walked > MAX_DABS {
                break 'walk;
            }
        }
        left -= l - t;
    }
    let start = tip.start(stroke.seed);
    for (shape, group) in tip.by_stamp(&dabs, start, |d| d.seed) {
        rasterize_uv(canvas, &set.gbuf, shape, &group);
    }
    // symmetry: the copies of the dabs' places on the mesh
    let copies = symmetry(stroke);
    if !copies.is_empty() {
        let on_mesh: Vec<Stamp> = dabs.iter().filter_map(|d| d.on_mesh(&set.mesh, &set.gbuf, set.world_per_texel * res)).collect();
        let copied: Vec<Stamp> = copies.iter().flat_map(|m| on_mesh.iter().filter_map(move |d| copied(d, m))).collect();
        lay_dabs(set, brush, tip, start, set.center, &copied, canvas);
    }
}

#[cfg(test)]
mod tests;
