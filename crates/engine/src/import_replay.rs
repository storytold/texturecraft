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
//! - **Projection painting** (masks): the grey laid at each texel is the projected image's where the texel lies on the
//!   stroke's view, placed as a stencil is ([`texturecraft_paint::commit_mask_texels`]).
//! - **Smudge**: dab by dab, in order, each dab's texels (as the brush lays them, times the effect's opacity) draw what
//!   lies at the previous dab's centre over their own, in place in the pixels the job's earlier steps laid, values and
//!   coverage alike ([`texturecraft_paint::smudge_in_place`], the rule measured on the file's app). A dab further than
//!   [`SMUDGE_JUMP`] radii from the previous one on the mesh, or [`SMUDGE_SEAM`] radii away in the texture (a seam),
//!   moves nothing; symmetry copies are not smudged.
//! - **Path strokes** (curves on the mesh, no view): dabs walked along the stored cubic curve by its length in the
//!   mesh's space, the first on the first anchor, each next `spacing × diameter` further, none forced onto the last
//!   (measured to 0.02 texels on flat paths); each dab at the surface point nearest its place on the curve, the tip
//!   along the path (or its top towards the brush's up direction), jitter drawn from the path's seed as for strokes,
//!   the anchors' size as the pen's pressure. Where the curve passes under or over a curved surface, its dabs shrink
//!   with the depth ([`PATH_DEPTH`], fitted on a sphere). Laid after the step's strokes, each path on its own.
//! - **Ribbons**: a ribbon brush lays its tip along each path as a band ([`texturecraft_paint::rasterize_ribbon`]).
//! - **Brushes painting an image or a graph** ([`texturecraft_import::replay::source`]): a stroke's texels take the
//!   source's value texel for texel; a path's and a ribbon's take it at their place on the band, or in the tip's square
//!   of the last dab reaching them, with the source's coverage ([`texturecraft_paint::commit_layer_texels`]).
//!   Projection painting on content reads each channel's source where the texel lies on the stroke's view, placed as a
//!   mask's projected image is.
//! - Dabs reach every surface in their volume whatever the view sees (measured: nothing occludes), at the texel's
//!   distance along the surface, a surface lying off the dab's surface fading out ([`Reach::Surface`]), faded by the turn
//!   from the dab's normal as the brush's culling says ([`Facing::Fade`]); each stroke is laid on its own and committed
//!   with the effect's opacity ("over" for paint, coverage lowered for the eraser).
//!
//! Everything runs on rayon; no files, threads or clocks, so it works in the web build.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_compose::PixelStore;
use texturecraft_core::{Channel, ChannelKind};
use texturecraft_geom::glam::{Mat3, Mat4, Vec2, Vec3};
use texturecraft_geom::{Bvh, Mesh, NO_TRI, OrbitCamera, SubMesh, UvGBuffer};
use texturecraft_import::replay::{
    PaintMode, ReplayBrush, ReplayJob, ReplayPath, ReplayRibbon, ReplayScene, ReplaySource, ReplayStencil, ReplayStep, ReplayStroke, ReplayTarget,
    ReplayTexels, ReplayTip, ReplayedPixels, RibbonTiling as ReplayTiling, StencilImage, StrokeReplayer, StrokeSpace, TipAlignment,
};
use texturecraft_paint::alphas::AlphaImage;
use texturecraft_paint::jitter::{Jittered, StrokeRandom, cycle_start};
use texturecraft_paint::{
    Alignment, BrushAlpha, BrushSettings, ChannelTexels, CommitSource, Facing, PickCtx, RasterCtx, Reach, Ribbon, RibbonTiling, SeededJitter, Shape,
    SpaceCurve, SpacePiece, Staging, Stamp, Stroke, StrokeCanvas, UvStamp, ViewStencil, commit_layer, commit_layer_texels, commit_mask, commit_mask_texels,
    footprint, footprint_uv, pressure_scale, rasterize, rasterize_ribbon_at, rasterize_uv, smudge_in_place,
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
/// A smudge dab further than this many radii from the previous one on the mesh draws nothing (the pen jumped; measured
/// rule of the file's app).
pub const SMUDGE_JUMP: f32 = 4.0;
/// A smudge dab whose previous one lies further than this many radii away in the texture draws nothing (a seam).
pub const SMUDGE_SEAM: f32 = 16.0;

/// How deep under (or over) the surface a path stroke may pass and still lay a dab, in radii of the dab: a dab `d` deep
/// lays a footprint of radius `r·√(1 − (d / (PATH_DEPTH·r))²)` (fitted on a path through a sphere at spacing 200 %: dabs
/// 0.025 … 0.08 deep with radius 0.05 shrink from full size to about 0.4).
pub const PATH_DEPTH: f32 = 1.75;
/// A path's dab looks for the surface within this many radii of its place on the curve (at least [`PATH_REACH_TEXELS`]).
const PATH_REACH: f32 = 4.0;
const PATH_REACH_TEXELS: f32 = 8.0;

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

    /// The index of texel `(x, y)` in the set's buffers.
    fn index(&self, x: u32, y: u32) -> usize {
        y as usize * self.resolution as usize + x as usize
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

    /// The shape dab `seed` lays: stamp `seed + start` of the cycle, or the tip.
    fn shape_of(&self, seed: u32, start: usize) -> Shape<'_> {
        match self.cycle.len() {
            0 => self.shape(),
            n => Shape { library: self.cycle.get((seed as usize).wrapping_add(start) % n), ..self.shape() },
        }
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

/// Where a stroke's texels lie on its view.
enum OnView<'a> {
    /// Through the recorded camera (view times projection).
    Mesh(&'a SetMesh, Mat4),
    /// The 2D view: the texture spans the view.
    Texture(f32),
}

impl OnView<'_> {
    fn of<'a>(set: &'a SetMesh, stroke: &ReplayStroke) -> OnView<'a> {
        match &stroke.space {
            StrokeSpace::Mesh { view, projection } => OnView::Mesh(set, Mat4::from_cols_array(projection) * Mat4::from_cols_array(view)),
            StrokeSpace::Uv => OnView::Texture(set.resolution as f32),
        }
    }

    /// Texel `(x, y)` on the view in normalised device coordinates (y up); `None` off the set or behind the camera.
    fn at(&self, x: u32, y: u32) -> Option<Vec2> {
        match self {
            OnView::Mesh(set, view_projection) => {
                let surface = set.gbuf.surface(&set.mesh, y as usize * set.resolution as usize + x as usize)?;
                let clip = *view_projection * surface.pos.extend(1.0);
                (clip.w > 1e-6).then(|| Vec2::new(clip.x / clip.w, clip.y / clip.w))
            }
            OnView::Texture(r) => Some(Vec2::new((x as f32 + 0.5) / r * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / r * 2.0)),
        }
    }
}

/// Multiplies a laid stroke's coverage by `stencil` where each texel lies on the stroke's view (nothing behind the
/// camera).
fn through(set: &SetMesh, stroke: &ReplayStroke, stencil: &ViewStencil, canvas: &mut StrokeCanvas) {
    let on = OnView::of(set, stroke);
    canvas.scale(|x, y| on.at(x, y).map_or(0.0, |p| stencil.at(p)));
}

/// Lays every step of `job` into its target.
fn replay_job(set: &SetMesh, job: &ReplayJob, stop: &(dyn Fn() -> bool + Sync)) -> Result<ReplayedPixels, String> {
    let mut pixels = PixelStore::default();
    let id = job.target.id();
    for step in &job.steps {
        let tip = Tip::of(&step.brush)?;
        if step.mode == PaintMode::Smudge {
            smudge_step(set, step, &tip, job.target, &mut pixels, stop)?;
            continue;
        }
        let sources = Sources::of(step)?;
        let stencil = step.stencil.as_ref().map(view_stencil).transpose()?;
        let projection = step.projection.as_ref().map(view_stencil).transpose()?;
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
            match (&projection, job.target) {
                (Some(image), ReplayTarget::Mask(id)) => {
                    let on = OnView::of(set, stroke);
                    let grey = |x: u32, y: u32| Some((on.at(x, y).map_or(0.0, |p| image.at(p)), 1.0));
                    commit_mask_texels(&mut pixels, id, set.size(), &grey, source(step, &canvas, 1.0));
                }
                _ => {
                    // projection on content: each channel's image where the texel lies on the view; else texel for texel
                    let reading = match step.placed {
                        Some(m) => Reading::View(OnView::of(set, stroke), Mat3::from_cols_array(&m)),
                        None => Reading::Places(HashMap::new()),
                    };
                    commit(&mut pixels, set, job.target, step, &canvas, &sources, &reading)
                }
            }
        }
        for path in &step.paths {
            if stop() {
                return Err("cancelled".into());
            }
            let mut canvas = StrokeCanvas::new(set.resolution, set.resolution);
            // a source painted along the path is read at each texel's place on the band or in its last dab's square
            let mut places = HashMap::new();
            match &step.ribbon {
                Some(ribbon) => {
                    let laid = lay_ribbon(set, ribbon, &tip, path, &mut canvas);
                    if !sources.is_empty() {
                        places.extend(laid.into_iter().map(|(x, y, at)| (set.index(x, y), at)));
                    }
                }
                None => {
                    let dabs = path_dabs(set, &step.brush, &tip, path);
                    lay_dabs(set, &step.brush, &tip, 0, set.center, &dabs, &mut canvas);
                    if !sources.is_empty() {
                        places = dab_places(set, &step.brush, &tip, &dabs);
                    }
                }
            }
            if !canvas.dirty.is_empty() {
                commit(&mut pixels, set, job.target, step, &canvas, &sources, &Reading::Places(places));
            }
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

/// A step's images and graphs, ready to sample: per channel with its opacity (content), or the mask's grey.
#[derive(Default)]
struct Sources {
    channels: Vec<(Channel, Arc<ReplayTexels>, f32)>,
    mask: Option<Arc<ReplayTexels>>,
}

impl Sources {
    /// `step`'s sources; an error for one that did not render.
    fn of(step: &ReplayStep) -> Result<Sources, String> {
        let ready = |s: &ReplaySource| match s {
            ReplaySource::Texels(t) => Ok(t.clone()),
            ReplaySource::Failed(why) => Err(why.clone()),
            ReplaySource::Graph(g) => Err(format!("brush source \"{}\" was not rendered", g.name)),
        };
        let mut out = Sources { mask: step.mask_source.as_ref().map(ready).transpose()?, ..Sources::default() };
        for (c, s, o) in &step.sources {
            out.channels.push((*c, ready(s)?, *o));
        }
        Ok(out)
    }

    fn is_empty(&self) -> bool {
        self.channels.is_empty() && self.mask.is_none()
    }
}

/// Where a stroke's texels read the step's sources.
enum Reading<'a> {
    /// At a place of the source's own image by texel index (a path's texels: the place on the band or in the tip's
    /// square of the last dab reaching the texel, 0..1 rows down); texels without one read it texel for texel.
    Places(HashMap<usize, [f32; 2]>),
    /// Projection painting on content: where the texel lies on the stroke's view, through the matrix to the image's
    /// place (y up); nothing outside the image or behind the camera.
    View(OnView<'a>, Mat3),
}

impl Reading<'_> {
    /// `source` at texel `(x, y)` as a channel of `kind` holds it, with its coverage factor.
    fn read(&self, set: &SetMesh, source: &ReplayTexels, kind: ChannelKind, x: u32, y: u32) -> Option<([f32; 3], f32)> {
        match self {
            Reading::Places(places) => match places.get(&set.index(x, y)) {
                Some(at) => source.at_uv(kind, *at),
                None => source.at_texel(kind, x, y, set.size()),
            },
            Reading::View(on, to_image) => {
                let ndc = on.at(x, y)?;
                let p = *to_image * Vec3::new(ndc.x, ndc.y, 1.0);
                // the image's rows run down
                let uv = [p.x, 1.0 - p.y];
                uv.iter().all(|v| (0.0..=1.0).contains(v)).then(|| source.at_uv(kind, uv)).flatten()
            }
        }
    }
}

/// The places in the tip's square of path `dabs` (laid in order: a later dab's place wins) for the texels each reaches:
/// `0.5 + offset·x / 2r` along the tip's columns, `0.5 + offset·y / 2r` down its rows (measured replay rule).
fn dab_places(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, dabs: &[Stamp]) -> HashMap<usize, [f32; 2]> {
    let mut places = HashMap::new();
    for d in dabs {
        if d.radius.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            continue;
        }
        let texels = footprint(&raster_ctx(set, brush, set.center, tip.shape_of(d.seed, 0)), d);
        for (x, y, _) in texels {
            let i = set.index(x, y);
            let Some(surface) = set.gbuf.surface(&set.mesh, i) else { continue };
            let off = surface.pos - d.center;
            let (u, v) = (0.5 + off.dot(d.tangent) / (2.0 * d.radius), 0.5 - off.dot(d.bitangent) / (2.0 * d.radius));
            if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
                places.insert(i, [u, v]);
            }
        }
    }
    places
}

/// A laid stroke as a commit source with the step's opacity times `opacity`.
fn source<'a>(step: &ReplayStep, canvas: &'a StrokeCanvas, opacity: f32) -> CommitSource<'a> {
    CommitSource {
        alpha: &canvas.alpha,
        tiles: &canvas.dirty,
        opacity: (step.opacity * opacity).clamp(0.0, 1.0),
        erase: step.mode == PaintMode::Erase,
        per_texel: None,
        carried: None,
    }
}

/// A channel's value and coverage factor read at a texel.
type TexelRead<'a> = dyn Fn(u32, u32) -> Option<([f32; 3], f32)> + Sync + 'a;

/// Commits one stroke's coverage into the target with the step's values and opacity, its sources read as `places` says.
fn commit(pixels: &mut PixelStore, set: &SetMesh, target: ReplayTarget, step: &ReplayStep, canvas: &StrokeCanvas, sources: &Sources, places: &Reading) {
    let source = |opacity: f32| source(step, canvas, opacity);
    match target {
        ReplayTarget::Mask(id) => match &sources.mask {
            Some(m) => {
                let grey = |x: u32, y: u32| places.read(set, m, ChannelKind::Scalar, x, y).map(|(v, a)| (v[0], a));
                commit_mask_texels(pixels, id, set.size(), &grey, source(1.0));
            }
            None => commit_mask(pixels, id, set.size(), step.mask_value, source(1.0), &mut Staging::default()),
        },
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
            if sources.channels.is_empty() {
                return;
            }
            let reads: Vec<(Channel, f32, Box<TexelRead>)> = sources
                .channels
                .iter()
                .map(|(c, texels, o)| {
                    let (kind, texels) = (c.kind(), texels.as_ref());
                    let read: Box<TexelRead> = Box::new(move |x, y| places.read(set, texels, kind, x, y));
                    (*c, *o, read)
                })
                .collect();
            let mut opacities: Vec<f32> = Vec::new();
            for (_, o, _) in &reads {
                if !opacities.iter().any(|g| (g - o).abs() < 1e-6) {
                    opacities.push(*o);
                }
            }
            for o in opacities {
                let channels: Vec<(Channel, ChannelTexels)> =
                    reads.iter().filter(|(_, g, _)| (g - o).abs() < 1e-6).map(|(c, _, f)| (*c, f.as_ref() as ChannelTexels)).collect();
                commit_layer_texels(pixels, id, set.size(), &channels, source(o));
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

/// The rasteriser's view of the set for `shape`: every surface in a dab's reach along the surface (measured: a surface
/// lying off the dab's fades out), faded as the brush culls.
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
        reach: Reach::Surface,
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
    let (eye, start) = (camera.orbit.eye(), tip.start(stroke.seed));
    mesh_dabs(set, brush, tip, stroke, camera, true, |dabs| lay_dabs(set, brush, tip, start, eye, dabs, canvas));
}

/// A 3D-view stroke's dabs through TextureCraft's [`Stroke`], fed point by point, handed to `lay` in order batch by
/// batch; each batch's symmetry copies after its originals when `copies`.
fn mesh_dabs(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, stroke: &ReplayStroke, camera: &ViewCamera, copies: bool, mut lay: impl FnMut(&[Stamp])) {
    let (mut settings, size_scale) = settings(brush, stroke.lazy * VIEWPORT_HEIGHT);
    settings.hardness = tip.hardness;
    settings.alpha = tip.alpha;
    let mut pen = Stroke::new(&settings, &set.mesh).seeded(seeded(brush, stroke));
    pen.radius = brush.radius * size_scale;
    let ctx = PickCtx { mesh: &set.mesh, bvh: &set.bvh, camera: &camera.orbit, viewport: camera.viewport };
    let copies = if copies { symmetry(stroke) } else { Vec::new() };
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
        lay(&dabs);
        if laid > MAX_DABS {
            break;
        }
    }
}

/// The symmetry copies' matrices of a stroke: the recorded step applied once, twice, … `copies` times.
fn symmetry(stroke: &ReplayStroke) -> Vec<Mat4> {
    copy_matrices(stroke.copies, &stroke.copy_matrix)
}

/// `step` applied once, twice, … `copies` times.
fn copy_matrices(copies: u32, step: &[f32; 16]) -> Vec<Mat4> {
    let step = Mat4::from_cols_array(step);
    if copies == 0 || !step.is_finite() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut power = step;
    for _ in 0..copies {
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
    let dabs = uv_dabs(set, brush, tip, stroke);
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

/// A 2D-view stroke's dabs in order (see [`lay_uv_stroke`]), without symmetry copies.
fn uv_dabs(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, stroke: &ReplayStroke) -> Vec<UvStamp> {
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
    let Some(&(first, p0)) = points.first() else { return dabs };
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
    dabs
}

/// A path stroke's curve in the mesh's space.
fn path_curve(path: &ReplayPath) -> SpaceCurve {
    let v = Vec3::from_array;
    let pieces: Vec<SpacePiece> = path.pieces.iter().map(|p| SpacePiece { points: p.points.map(v), normals: p.normals.map(v), size: p.size }).collect();
    SpaceCurve::new(&pieces, path.anchor.map(|(p, n, size)| (v(p), v(n), size)))
}

/// The surface point of the set nearest `p` within `reach`, with the surface's normal there.
fn surface_near(set: &SetMesh, p: Vec3, reach: f32) -> Option<(Vec3, Vec3)> {
    let hit = set.bvh.nearest(p, reach.max(1e-6), &|_| true)?;
    let [a, b, c] = set.mesh.tri(hit.tri as usize)?;
    let w = 1.0 - hit.u - hit.v;
    let at = |v: &[Vec3]| Some(*v.get(a)? * w + *v.get(b)? * hit.u + *v.get(c)? * hit.v);
    let (point, normal) = (at(&set.mesh.positions)?, at(&set.mesh.normals)?.try_normalize()?);
    point.is_finite().then_some((point, normal))
}

/// Axes `x`, `y` (right and down on a view) turned clockwise by `angle` radians.
fn turned(x: Vec3, y: Vec3, angle: f32) -> (Vec3, Vec3) {
    let (s, c) = angle.sin_cos();
    (x * c + y * s, y * c - x * s)
}

/// A path stroke's dabs (see the module's notes), symmetry copies after the originals. Dab `k` lays stamp `k` of a
/// dynamic alpha's cycle.
fn path_dabs(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, path: &ReplayPath) -> Vec<Stamp> {
    let curve = path_curve(path);
    let mut random = StrokeRandom::new(SeededJitter { seed: path.seed, both_axes: brush.position_both, normal: brush.position_normal });
    let jittered =
        Jittered { size: brush.size_jitter > 0.0, flow: brush.flow_jitter > 0.0, rotation: brush.rotation_jitter > 0.0, position: brush.position_jitter > 0.0 };
    let up = brush.up.map(Vec3::from_array);
    let mut out: Vec<Stamp> = Vec::new();
    curve.walk(MAX_DABS, |p, heading, size| {
        let d = random.draw(jittered);
        let r = brush.radius * pressure_scale(brush.size_pressure.unwrap_or(1.0), size) * (1.0 - brush.size_jitter * (1.0 - d.size));
        let flow = brush.flow * pressure_scale(brush.flow_pressure.unwrap_or(1.0), size) * (1.0 - brush.flow_jitter * (1.0 - d.flow));
        if !(r > 0.0 && r.is_finite()) {
            // a dab of no size lays nothing and ends the walk
            return f32::MAX;
        }
        let step = brush.spacing.max(0.01) * 2.0 * r;
        let reach = (PATH_REACH * r).max(PATH_REACH_TEXELS * set.world_per_texel);
        let Some((on, n)) = surface_near(set, p, reach) else { return step };
        // the path's frame: x along it on the tangent plane, y a quarter turn clockwise (right and down on a view)
        let along = (heading - n * heading.dot(n)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
        let (x0, y0) = (along, along.cross(n));
        // the tip's: along the path, or with its top towards the brush's up direction when it does not follow the path
        let up = up.map(|v| v - n * v.dot(n)).and_then(|v| (v.length() > 1e-4).then_some(v)).and_then(Vec3::try_normalize);
        let (tx, ty) = match (brush.follow, up) {
            (false, Some(up)) => (n.cross(-up), -up),
            _ => (x0, y0),
        };
        let turn = if jittered.rotation { brush.rotation_jitter * (1.0 - 2.0 * d.rotation) } else { 0.0 };
        let follow = if brush.follow { std::f32::consts::FRAC_PI_2 } else { 0.0 };
        let (x, y) = turned(tx, ty, brush.rotation + turn + follow);
        // position jitter in shares of the diameter: both axes in a drawn direction, or across the path
        let distance = brush.position_jitter * d.distance;
        let moved = match (jittered.position, brush.position_both) {
            (false, _) => Vec3::ZERO,
            (true, true) => {
                let (sin, cos) = (std::f32::consts::PI * (d.direction - 0.5)).sin_cos();
                x0 * (distance * sin) + y0 * (distance * cos)
            }
            (true, false) => -y0 * distance,
        } * (2.0 * r);
        let placed = if moved == Vec3::ZERO { Some((on, n)) } else { surface_near(set, on + moved, reach + 2.0 * r) };
        // a curve off the surface lays smaller dabs the deeper it is
        let shrink = 1.0 - ((p - on).length() / (PATH_DEPTH * r)).powi(2);
        if let (Some((center, normal)), true) = (placed, shrink > 0.0) {
            let seed = u32::try_from(out.len()).unwrap_or(u32::MAX);
            out.push(Stamp {
                center,
                normal,
                tangent: x,
                bitangent: -y,
                radius: r * shrink.sqrt(),
                flow: flow.clamp(0.0, 1.0),
                hardness: tip.hardness,
                depth: 1.0,
                seed,
                source: None,
            });
        }
        step
    });
    let originals = out.len();
    for m in copy_matrices(path.copies, &path.copy_matrix) {
        for i in 0..originals {
            if let Some(d) = out.get(i).and_then(|d| copied(d, &m)) {
                out.push(d);
            }
        }
    }
    out
}

/// A ribbon along one path: the tip as a band (see [`texturecraft_paint::ribbon`]); every texel laid with its place on
/// the band.
fn lay_ribbon(set: &SetMesh, ribbon: &ReplayRibbon, tip: &Tip, path: &ReplayPath, canvas: &mut StrokeCanvas) -> Vec<(u32, u32, [f32; 2])> {
    let tiling = match ribbon.tiling {
        ReplayTiling::Repeat => RibbonTiling::Repeat,
        ReplayTiling::Whole => RibbonTiling::Whole,
        ReplayTiling::Caps { guides } => RibbonTiling::Caps { guides },
    };
    let band = Ribbon { width: ribbon.width, copy: ribbon.copy, tiling, depth: ribbon.depth };
    // the tip's detail for a copy's shorter side in texels
    let radius = ribbon.width.min(ribbon.copy) * 0.5 / set.world_per_texel;
    rasterize_ribbon_at(canvas, &set.mesh, &set.gbuf, &path_curve(path), &band, tip.shape().at_radius(radius), tip.hardness)
}

/// A smudge dab: its texels' coverage (the brush's shape, flow and culling), its centre in texels, and its centre and
/// radius on the mesh.
struct SmudgeDab {
    texels: Vec<(u32, u32, f32)>,
    texel: Vec2,
    center: Vec3,
    radius: f32,
}

/// The texel (continuous, texel centres at halves) under a point on the set's surface; `None` off it.
fn texel_under(set: &SetMesh, p: Vec3, reach: f32) -> Option<Vec2> {
    let hit = set.bvh.nearest(p, reach.max(1e-4), &|_| true)?;
    let [a, b, c] = set.mesh.tri(hit.tri as usize)?;
    let (ua, ub, uc) = (set.mesh.uvs.get(a)?, set.mesh.uvs.get(b)?, set.mesh.uvs.get(c)?);
    let uv = *ua * (1.0 - hit.u - hit.v) + *ub * hit.u + *uc * hit.v;
    let t = uv * set.resolution as f32;
    t.is_finite().then_some(t)
}

/// One smudge step: each stroke's dabs in order, every dab after the first moving what lies at the previous dab's
/// centre over its own texels, in place in the target's images the job's earlier steps laid (a step's channels with
/// their own opacity, or the mask's grey). Images that hold nothing yet have nothing to move.
fn smudge_step(
    set: &SetMesh,
    step: &ReplayStep,
    tip: &Tip,
    target: ReplayTarget,
    pixels: &mut PixelStore,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    // the images moved, each with the share of a dab's coverage it takes
    let mut images: Vec<(Option<Channel>, f32, TileImage)> = match target {
        ReplayTarget::Content(id) => {
            step.values.iter().filter_map(|(c, _, o)| pixels.layers.remove(&(id, *c)).map(|img| (Some(*c), step.opacity * o.clamp(0.0, 1.0), img))).collect()
        }
        ReplayTarget::Mask(id) => pixels.masks.remove(&id).map(|img| (None, step.opacity, img)).into_iter().collect(),
    };
    let mut result = Ok(());
    if !images.is_empty() {
        for stroke in &step.strokes {
            if stop() {
                result = Err("cancelled".into());
                break;
            }
            smudge_stroke(set, step, tip, stroke, &mut images);
        }
    }
    for (c, _, img) in images {
        match (target, c) {
            (ReplayTarget::Content(id), Some(c)) => {
                pixels.layers.insert((id, c), img);
            }
            (ReplayTarget::Mask(id), _) => {
                pixels.masks.insert(id, img);
            }
            (ReplayTarget::Content(_), None) => {}
        }
    }
    result
}

/// Smudges one stroke into `images` (see [`smudge_step`]).
fn smudge_stroke(set: &SetMesh, step: &ReplayStep, tip: &Tip, stroke: &ReplayStroke, images: &mut [(Option<Channel>, f32, TileImage)]) {
    let brush = &step.brush;
    let start = tip.start(stroke.seed);
    let res = set.resolution as f32;
    let mut last: Option<(Vec2, Vec3)> = None;
    let mut smudge = |dab: Option<SmudgeDab>| {
        let Some(dab) = dab else { return };
        let previous = last.replace((dab.texel, dab.center));
        let Some((from, center)) = previous else { return };
        let shift = from - dab.texel;
        if (center - dab.center).length() > SMUDGE_JUMP * dab.radius || shift.length() * set.world_per_texel > SMUDGE_SEAM * dab.radius {
            return;
        }
        images.par_iter_mut().for_each(|(c, share, img)| {
            let texels: Vec<(u32, u32, f32)> = dab.texels.iter().map(|&(x, y, a)| (x, y, a * *share)).collect();
            smudge_in_place(img, c.as_ref().map_or(texturecraft_core::ChannelKind::Scalar, |c| c.kind()), &texels, shift.to_array());
        });
    };
    match &stroke.space {
        StrokeSpace::Mesh { view, projection } => {
            let Some(camera) = ViewCamera::new(view, projection, set.center, set.radius) else { return };
            let eye = camera.orbit.eye();
            mesh_dabs(set, brush, tip, stroke, &camera, false, |dabs| {
                for d in dabs {
                    let texels = footprint(&raster_ctx(set, brush, eye, tip.shape_of(d.seed, start)), d);
                    smudge(texel_under(set, d.center, 2.0 * d.radius).map(|texel| SmudgeDab { texels, texel, center: d.center, radius: d.radius }));
                }
            });
        }
        StrokeSpace::Uv => {
            for d in uv_dabs(set, brush, tip, stroke) {
                let texel = d.center * res;
                let (x, y) = (texel.x.floor(), texel.y.floor());
                let Some(surface) = set.gbuf.surface(&set.mesh, y as usize * set.resolution as usize + x as usize) else { continue };
                let texels = footprint_uv(&set.gbuf, tip.shape_of(d.seed, start), &d);
                smudge(Some(SmudgeDab { texels, texel, center: surface.pos, radius: d.radius * res * set.world_per_texel }));
            }
        }
    }
}

#[cfg(test)]
mod tests;
