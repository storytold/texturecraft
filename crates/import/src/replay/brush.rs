//! One paint effect of the file as a [`ReplayStep`]: what can be replayed ([`check`]), the brush mapped onto neutral
//! quantities with the measured rules, the strokes as recorded, and the values painted.
//!
//! | File | Replay |
//! |---|---|
//! | `brushSize` | dab radius `brushSize / 2` in the mesh's units (measured) |
//! | `spacing`, `flow`, `opacity` | spacing (share of the diameter), flow per dab, opacity per stroke |
//! | `brushSizeJitter`, `flowJitter`, `positionJitter`, `rotationJitter` | jitter (position in diameters, rotation in radians) |
//! | `positionJitterBothAxes`, `positionJitterDistribution` 1 | position jitter along both view axes / normal distance |
//! | `randomSeed` | the stroke's seed: TextureCraft's brush draws the jitter from it as the file's app does |
//! | `rotation`, `followPath` | the tip's turn (clockwise on the screen) and whether it follows the stroke |
//! | `pressureSizeEnabled`/`Minimum`, `pressureFlowEnabled`/`Minimum` | linear in the pen's pressure: the minimum at 0 |
//! | `alignment` 3, 1 / 4 / 2 | tangent plane / facing the view / the texture's axes |
//! | `cullingEnabled`, `cullingAngle` | a fade by the turn between the texel's normal and the dab's (measured) |
//! | `lazyDistance` | the lazy mouse's string, a share of the view's height |
//! | 3D-view points | normalised device coordinates: the file stores 0..1 with y up (measured) |
//! | 2D-view points | texture coordinates: the file stores V up, the mesh's UVs run down (measured) |
//! | `duplicateCount`, `duplicateMatrix` | symmetry copies, the matrix applied once more per copy (both views) |
//! | `sourceStencil`, `projMatrix` | the stencil the strokes go through, placed on the view ([`tip::stencil`]) |
//! | projection tool | a mask's strokes paint the grey of the first `sourceColor`, placed on the view by `projMatrix`; content channels their own source placed the same way (Inferred) |
//! | smudge tool | each dab draws what lies at the previous dab's place over its own, values and coverage (measured) |
//! | `strokes3D` (path strokes) | cubic pieces through the stored anchors and handles in the mesh's space; dabs walked along them ([`ReplayPath`]) |
//! | anchors' `size` | the pen's pressure along a path, linear between anchors |
//! | `upVector` | where an unturned tip's top points along a path (Inferred) |
//! | ribbon brush (`width`, `tilingMode`, `stretchGuides`, `aspectRatioFactor`, `projectionDepthRatio`) | the tip laid along each path as a band ([`ReplayRibbon`]) |
//! | `sourceColor` bitmap or graph | painted per texel ([`super::source`]): a brush painting only images places them on the view by `projMatrix` as projection painting does (measured on a 2D-view dab); a graph laid on texel for texel; both read along paths and ribbons |
//! | black uniform alpha over a material with a coverage output, on paths | a full square tip: the material's coverage is the shape |
//! | particle brush (`urlEmitter`, `urlReceiver`, `tweaksEmitter`, `tweaksReceiver`, `deltaTime`) | dabs of a simulation per stroke ([`super::particles`], approximated) |
//! | `stampBlendingMode` ≠ 0, `aspectRatio` ≠ 1, other `alignment` | approximated (noted) |
//!
//! Not replayable (a `brush:` blocker): projection painting without an image,
//! smudge or projection painting along path strokes, a smudge with nothing of its effect before it, brushes painting
//! anchors or other sources, brushes painting an image into a mask, brushes painting a graph without the project's
//! graphs ([`SourceGraphs`]), unreadable point lists.

use std::sync::Arc;

use texturecraft_core::{Channel, GraphRef};
use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{
    PaintEffect, PaintTool, ProceduralSource, ProjectBrush, ProjectSource, ProjectStroke, RibbonBrush, StampEmitter, StrokePath3d, StrokeView,
};

use super::source::{ReplaySource, ReplayTexels};
use super::{
    PaintMode, PathPiece, ReplayBrush, ReplayPath, ReplayRibbon, ReplayStencil, ReplayStep, ReplayStroke, ReplayTip, RibbonTiling, StrokePoint, StrokeSpace,
    TipAlignment, pressure_at, tip,
};
use crate::channels::SetChannels;
use crate::resources::ProjectImages;

/// The file's brush size is a diameter in the normalised mesh's units (measured on controlled strokes).
pub const SIZE_TO_RADIUS: f32 = 0.5;
/// Most symmetry copies laid per stroke (a guard; the file stores one in the projects seen).
pub const MAX_COPIES: u32 = 64;
/// Most points read per stroke (a guard against hostile files).
pub const MAX_POINTS: usize = 1 << 20;
/// The note a step with path strokes carries: their rules are measured on flat paths and spheres only.
pub const PATHS_APPROXIMATE: &str = "3D path strokes approximated";
/// The note a ribbon step carries.
pub const RIBBON_APPROXIMATE: &str = "3D path ribbons approximated";
/// A stencil without its matrix spans the view: normalised device coordinates −1..1 onto the image's 0..1.
const VIEW_STENCIL: [f32; 9] = [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0];

/// The note a step painting a graph carries.
pub const GRAPH_TEXELS: &str = "brush source graph laid on texel for texel";
/// The note a step painting an image carries.
pub const IMAGE_TEXELS: &str = "brush source image laid on texel for texel";
/// The note a projection step on content carries.
pub const PROJECTED: &str = "projection painting on a paint layer placed as in masks";
/// The note a brush painting images carries: its images are placed on the view by `projMatrix`.
pub const IMAGE_PLACED: &str = "brush source image placed on the view";
/// The note a step painting an image or a graph along paths carries.
pub const SOURCE_ALONG: &str = "brush source read along the paths";

/// The graphs a brush painting a compiled graph paints: what a fill of the same source runs.
pub trait SourceGraphs {
    /// The graph a fill of `source` runs for each of `channels` it has an output for (one render shared by all; a
    /// channel without one is not painted); or why the graph cannot run.
    fn content(&self, source: &ProceduralSource, channels: &[Channel]) -> Result<Vec<(Channel, GraphRef)>, Blocker>;
    /// The graph of `source`'s grey output, as a mask's generator runs it; or why it cannot run.
    fn mask(&self, source: &ProceduralSource) -> Result<GraphRef, Blocker>;
}

/// What a step paints into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepTarget {
    /// A layer's content: its paint pixels or a content paint effect.
    Content,
    /// A mask's paint effect.
    Mask,
}

/// A replay refusal: `what` names it in the key (`brush:<what>`), `text` says it.
pub fn refused(what: &str, text: impl Into<String>) -> Blocker {
    Blocker::new(BlockerKey::new(BlockerKind::Brush, what), text)
}

/// Why `effect` cannot be replayed, if it cannot (its tip and values are checked by [`step`]).
pub fn check(effect: &PaintEffect) -> Result<(), Blocker> {
    match effect.tool {
        PaintTool::Brush | PaintTool::Eraser | PaintTool::Smudge => {}
        PaintTool::Projection if effect.colors.is_empty() => return Err(refused("projection", "projection painting without an image")),
        PaintTool::Projection => {}
    }
    if !effect.paths.is_empty() && effect.tool == PaintTool::Projection {
        return Err(refused("3d-path", "3D path strokes with projection painting"));
    }
    if !effect.paths.is_empty() && effect.tool == PaintTool::Smudge {
        return Err(refused("3d-path", "3D path strokes with the smudge tool"));
    }
    if let Some(b) = &effect.brush {
        match &b.emitter {
            StampEmitter::Stroke(_) | StampEmitter::Stroke3D(_) => {}
            StampEmitter::Physics(_) => {}
            StampEmitter::Ribbon(_) => {}
            StampEmitter::Other(class) if class.is_empty() => return Err(refused("no-emitter", "brush without an emitter")),
            StampEmitter::Other(class) => return Err(Blocker::class(class, format!("brush emitter {class}"))),
        }
    }
    if effect.paths.iter().any(|p| !p.complete) {
        return Err(refused("unreadable-points", "path points not fully readable"));
    }
    if effect.strokes.iter().any(|s| !s.complete) {
        return Err(refused("unreadable-points", "stroke points not fully readable"));
    }
    Ok(())
}

/// A smudge replayed as a job of its own moves only what its own effect holds, which is nothing; what it moves in the
/// stack below is not replayed.
pub fn alone(effect: &PaintEffect) -> Result<(), Blocker> {
    match effect.tool {
        PaintTool::Smudge => Err(refused("smudge-alone", "a smudge on its own (it moves what lies below its effect)")),
        _ => Ok(()),
    }
}

/// `effect` as a step painting into `target`, the declared `channels` of the set; or why it cannot be replayed. A brush
/// painting a compiled graph needs `sources` (the graphs a fill of it runs).
pub fn step(
    effect: &PaintEffect,
    target: StepTarget,
    channels: &SetChannels,
    images: &ProjectImages,
    graphs: Option<&crate::graph_library::GraphLibrary>,
    sources: Option<&dyn SourceGraphs>,
) -> Result<ReplayStep, Blocker> {
    check(effect)?;
    let mut notes = Vec::new();
    let (tip, tip_note) = match (own_alpha_tip(effect), super::particles::emitter(effect)) {
        (Some(full), _) => (full, None),
        (None, particle) => match (tip::tip(&effect.alpha, images, graphs), particle) {
            // particles are small: a soft round dab stands in for a tip that cannot be made
            (Err(b), Some(_)) => (ReplayTip::Round { hardness: 0.5 }, Some(format!("particle tip laid as a soft round dab ({})", b.text))),
            (tip, _) => tip?,
        },
    };
    notes.extend(tip_note);
    let mut brush = map_brush(effect, tip, &mut notes);
    (brush.cycle, brush.cycle_drawn_start) = tip::cycle(&effect.alpha, images, graphs);
    if let Some(n) = tip::uncycled(&effect.alpha, brush.cycle.len()) {
        notes.push(n);
    }
    let Values { mode, values, mask_value, sources, mask_source } = self::values(effect, target, channels, images, sources)?;
    // the stencil and the projected image share `projMatrix`
    let matrix = effect.stencil_matrix.filter(|m| m.iter().all(|v| v.is_finite())).unwrap_or(VIEW_STENCIL);
    let stencil = match &effect.stencil {
        Some(_) if mode == PaintMode::Smudge => {
            notes.push("a smudge's stencil left out".to_string());
            None
        }
        Some(source) => Some(ReplayStencil { image: tip::stencil(source, images, graphs)?, matrix }),
        None => None,
    };
    let projection = match (effect.tool, effect.colors.first(), target) {
        (PaintTool::Projection, Some(source), StepTarget::Mask) => Some(ReplayStencil { image: tip::stencil(source, images, graphs)?, matrix }),
        _ => None,
    };
    let placed = placement(effect.tool, target, &sources, matrix);
    let mut strokes: Vec<ReplayStroke> =
        effect.strokes.iter().filter_map(|s| stroke(s, brush.size_pressure.is_some() || brush.flow_pressure.is_some())).collect();
    if mode == PaintMode::Smudge && strokes.iter().any(|s| s.copies > 0) {
        notes.push("symmetry copies of smudge strokes left out".to_string());
    }
    let paths: Vec<ReplayPath> = effect.paths.iter().filter_map(path).collect();
    let ribbon = match effect.brush.as_ref().map(|b| &b.emitter) {
        Some(StampEmitter::Ribbon(r)) => Some(ribbon(r)),
        _ => None,
    };
    if ribbon.is_some() {
        notes.push(format!("{RIBBON_APPROXIMATE}: {} paths", paths.len()));
        if !strokes.is_empty() {
            notes.push(format!("{} brush strokes of a ribbon brush left out", strokes.len()));
            strokes.clear();
        }
    } else if !paths.is_empty() {
        notes.push(format!("{PATHS_APPROXIMATE}: {} paths{}", paths.len(), if stencil.is_some() { ", their stencil left out" } else { "" }));
    }
    let painted = sources.iter().map(|(_, s, _)| s).chain(&mask_source);
    let (mut graph, mut image) = (false, false);
    for s in painted {
        match s {
            ReplaySource::Texels(t) if matches!(**t, ReplayTexels::Image(_)) => image = true,
            _ => graph = true,
        }
    }
    if !strokes.is_empty() && placed.is_none() {
        notes.extend(graph.then(|| GRAPH_TEXELS.to_string()));
        notes.extend(image.then(|| IMAGE_TEXELS.to_string()));
    }
    if (graph || image) && !paths.is_empty() {
        notes.push(SOURCE_ALONG.to_string());
    }
    let particles = super::particles::emitter(effect).map(|p| {
        let (particles, note) = super::particles::particles(p);
        notes.push(note);
        particles
    });
    let opacity = if effect.opacity.is_finite() { effect.opacity.clamp(0.0, 1.0) } else { 1.0 };
    if placed.is_some() {
        notes.push(if effect.tool == PaintTool::Projection { PROJECTED } else { IMAGE_PLACED }.to_string());
    }
    Ok(ReplayStep { mode, values, sources, mask_value, mask_source, opacity, brush, stencil, projection, placed, strokes, paths, ribbon, particles, notes })
}

/// Where content sources are read on the stroke's view through `matrix` (`projMatrix`) instead of texel for texel:
/// projection painting, and a brush painting images only (a bitmap material is placed on the view as the projected
/// image is: a single 2D-view dab of an image brush lands its centre on the image's centre through `projMatrix`,
/// measured on a feather card). A brush painting a graph keeps laying it texel for texel.
pub(super) fn placement(tool: PaintTool, target: StepTarget, sources: &[(Channel, ReplaySource, f32)], matrix: [f32; 9]) -> Option<[f32; 9]> {
    if target != StepTarget::Content {
        return None;
    }
    let images_only = !sources.is_empty() && sources.iter().all(|(_, s, _)| matches!(s, ReplaySource::Texels(t) if matches!(**t, ReplayTexels::Image(_))));
    match tool {
        PaintTool::Projection => Some(matrix),
        PaintTool::Brush if images_only => Some(matrix),
        _ => None,
    }
}

/// A full square tip where a path brush paints a material with a coverage output under a black uniform alpha: the
/// material's coverage is then the shape (Inferred from the material presets: the reference app paints them, and a black
/// alpha would paint nothing). `None` otherwise.
fn own_alpha_tip(effect: &PaintEffect) -> Option<ReplayTip> {
    let own = effect.colors.iter().any(|c| matches!(c, ProjectSource::Procedural(p) if p.procedural.as_ref().is_some_and(|q| !q.alpha_output.is_empty())));
    let black =
        matches!(effect.alpha.first(), Some(ProjectSource::Uniform(u)) if (u.color[0] * u.opacity).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater));
    (own && black && !effect.paths.is_empty()).then(|| ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![1.0]) })
}

/// The path stroke as recorded; `None` for one without a finite anchor.
pub fn path(p: &StrokePath3d) -> Option<ReplayPath> {
    let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
    let size = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
    let pieces: Vec<PathPiece> = p
        .segments()
        .into_iter()
        .take(MAX_POINTS)
        .filter(|s| s.points.iter().chain(&s.normals).all(|v| finite(&v[..])))
        .map(|s| PathPiece { points: s.points, normals: s.normals, size: s.size.map(size) })
        .collect();
    let anchor = match pieces.is_empty() {
        true => Some(p.anchors().find(|a| finite(&a.position[..]) && finite(&a.normal[..])).map(|a| (a.position, a.normal, size(a.size)))?),
        false => None,
    };
    Some(ReplayPath { pieces, anchor, seed: p.random_seed, copies: p.duplicate_count.min(MAX_COPIES), copy_matrix: p.duplicate_matrix })
}

/// A ribbon brush in neutral quantities: `tilingMode` 1 one copy over the path (Inferred, panels), 2 capped by the
/// stretch guides, otherwise repeated; the band reaches `projectionDepthRatio` half widths along the surface's normal,
/// at least half its width (Inferred from the name).
pub fn ribbon(r: &RibbonBrush) -> ReplayRibbon {
    let finite = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
    let width = finite(r.width, 0.0, 1e3, 0.0);
    let half = width * 0.5;
    let tiling = match r.tiling_mode {
        1 => RibbonTiling::Whole,
        2 => RibbonTiling::Caps { guides: r.stretch_guides.map(|g| finite(g, 0.0, 1.0, 0.5)) },
        _ => RibbonTiling::Repeat,
    };
    ReplayRibbon {
        width,
        copy: (width * finite(r.aspect_ratio, 1e-3, 1e3, 1.0)).max(1e-6),
        tiling,
        depth: (finite(r.projection_depth_ratio, 0.0, 1e3, 1.0) * half).max(half),
    }
}

/// The brush of `effect` (its defaults when it has none).
pub(crate) fn map_brush(effect: &PaintEffect, tip: super::ReplayTip, notes: &mut Vec<String>) -> ReplayBrush {
    let default = ProjectBrush::default();
    let b = effect.brush.as_ref().unwrap_or(&default);
    let default_emitter = texturecraft_import_reader::model::StrokeEmitter::default();
    let e = match &b.emitter {
        StampEmitter::Stroke(e) | StampEmitter::Stroke3D(e) => e,
        _ => &default_emitter,
    };
    let finite = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
    let alignment = match b.alignment {
        3 | 1 => TipAlignment::Tangent,
        4 => TipAlignment::Camera,
        2 => {
            notes.push("a tip aligned to the texture's axes laid on the tangent plane".to_string());
            TipAlignment::Uv
        }
        n => {
            notes.push(format!("brush alignment {n} laid on the tangent plane"));
            TipAlignment::Tangent
        }
    };
    if effect.stamp_blending != 0 {
        notes.push(format!("stamp blending mode {} laid as stamps over each other", effect.stamp_blending));
    }
    if (e.aspect_ratio - 1.0).abs() > 1e-3 && e.aspect_ratio.is_finite() {
        notes.push(format!("tip aspect ratio {} left out", e.aspect_ratio));
    }
    if b.size_space != 1 {
        notes.push(format!("brush size space {} read as the mesh's units", b.size_space));
    }
    let minimum = |p: &texturecraft_import_reader::model::PressureResponse| p.enabled.then(|| finite(p.minimum, 0.0, 1.0, 0.0));
    ReplayBrush {
        radius: finite(b.size, 0.0, 1e3, 0.1) * SIZE_TO_RADIUS,
        spacing: finite(e.spacing, 0.0, 100.0, 0.2),
        flow: finite(b.flow, 0.0, 1.0, 1.0),
        size_pressure: minimum(&e.pressure_size),
        flow_pressure: minimum(&e.pressure_flow),
        size_jitter: finite(b.size_jitter, 0.0, 1.0, 0.0),
        flow_jitter: finite(b.flow_jitter, 0.0, 1.0, 0.0),
        position_jitter: finite(e.position_jitter, 0.0, 10.0, 0.0),
        position_both: e.position_jitter_both_axes,
        position_normal: e.position_jitter_distribution == 1,
        rotation: finite(e.rotation, -1e3, 1e3, 0.0),
        rotation_jitter: finite(e.rotation_jitter, 0.0, std::f32::consts::PI, 0.0),
        follow: e.follow_path,
        alignment,
        culling: b.culling.map(|c| finite(c, 0.0, 1.0, 0.5)),
        tip,
        cycle: Vec::new(),
        cycle_drawn_start: false,
        up: e.up_vector.filter(|v| v.iter().all(|x| x.is_finite())),
    }
}

/// The stroke as recorded; `None` for a stroke without points. `pressure`: whether the brush reads pressure.
pub fn stroke(s: &ProjectStroke, pressure: bool) -> Option<ReplayStroke> {
    if s.points.is_empty() {
        return None;
    }
    let space = match s.view {
        StrokeView::Uv => StrokeSpace::Uv,
        StrokeView::Mesh | StrokeView::Other(_) => StrokeSpace::Mesh { view: s.view_matrix, projection: s.projection_matrix },
    };
    let points = s
        .points
        .iter()
        .take(MAX_POINTS)
        .filter(|p| p.position.iter().all(|v| v.is_finite()))
        .map(|p| StrokePoint {
            position: match space {
                // the file stores 0..1 with y up
                StrokeSpace::Mesh { .. } => [2.0 * p.position[0] - 1.0, 2.0 * p.position[1] - 1.0],
                // the file stores V up; the mesh's UVs run down
                StrokeSpace::Uv => [p.position[0], 1.0 - p.position[1]],
            },
            pressure: if pressure { pressure_at(&s.pressure, p.time) } else { 1.0 },
        })
        .collect();
    let lazy = if s.lazy_distance.is_finite() && s.lazy_distance > 0.0 { s.lazy_distance } else { 0.0 };
    Some(ReplayStroke { space, points, seed: s.random_seed, lazy, copies: s.duplicate_count.min(MAX_COPIES), copy_matrix: s.duplicate_matrix })
}

/// What a step lays: the tool's mode, values per channel (content), sources per channel (content painted with an
/// image or a graph), grey value and source (mask).
struct Values {
    mode: PaintMode,
    values: Vec<(Channel, [f32; 3], f32)>,
    mask_value: f32,
    sources: Vec<(Channel, ReplaySource, f32)>,
    mask_source: Option<ReplaySource>,
}

impl Values {
    fn of(mode: PaintMode, values: Vec<(Channel, [f32; 3], f32)>, mask_value: f32) -> Values {
        Values { mode, values, mask_value, sources: Vec::new(), mask_source: None }
    }
}

/// What `effect` lays: the brush paints its `sourceColor` uniforms (linear, as fills), images and graphs (per texel,
/// [`super::source`]); the eraser lowers coverage on the channels it works on, or in a mask paints black over what its
/// strokes reach (measured); the smudge moves what is there on the channels it works on; projection painting in a mask
/// paints its image's grey (the step's projection), on content each channel's source placed on the view.
fn values(
    effect: &PaintEffect,
    target: StepTarget,
    channels: &SetChannels,
    images: &ProjectImages,
    graphs: Option<&dyn SourceGraphs>,
) -> Result<Values, Blocker> {
    let paints_source = |s: &ProjectSource| refused(&format!("paints-{}", s.kind()), format!("brush paints a {} source", s.kind()));
    let library = |p: &ProceduralSource| graphs.ok_or_else(|| paints_source(&ProjectSource::Procedural(p.clone())));
    match (target, effect.tool) {
        (StepTarget::Mask, PaintTool::Projection) => Ok(Values::of(PaintMode::Paint, Vec::new(), 1.0)),
        (StepTarget::Mask, PaintTool::Smudge) => Ok(Values::of(PaintMode::Smudge, Vec::new(), 0.0)),
        (StepTarget::Content, PaintTool::Smudge) => {
            let worked = worked(effect, channels).into_iter().map(|c| (c, [0.0; 3], 1.0)).collect();
            Ok(Values::of(PaintMode::Smudge, worked, 0.0))
        }
        (StepTarget::Mask, PaintTool::Eraser) => Ok(Values::of(PaintMode::Paint, Vec::new(), 0.0)),
        (StepTarget::Mask, _) => match effect.colors.first() {
            None => Ok(Values::of(PaintMode::Paint, Vec::new(), 1.0)),
            Some(ProjectSource::Uniform(u)) => {
                let v = u.color[0] * u.opacity;
                Ok(Values::of(PaintMode::Paint, Vec::new(), if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }))
            }
            // a brush (not an eraser or smudge) painting a graph lays its grey output
            Some(ProjectSource::Procedural(p)) if effect.tool == PaintTool::Brush => {
                let source = ReplaySource::Graph(Box::new(library(p)?.mask(p)?));
                Ok(Values { mask_source: Some(source), ..Values::of(PaintMode::Paint, Vec::new(), 1.0) })
            }
            Some(other) => Err(paints_source(other)),
        },
        (StepTarget::Content, PaintTool::Eraser) => {
            let worked = worked(effect, channels).into_iter().map(|c| (c, [0.0; 3], 1.0)).collect();
            Ok(Values::of(PaintMode::Erase, worked, 0.0))
        }
        (StepTarget::Content, _) => {
            let mut out = Values::of(PaintMode::Paint, Vec::new(), 0.0);
            // each image read once, each graph run once for every channel it paints
            let mut read: Vec<(&str, Arc<ReplayTexels>)> = Vec::new();
            let mut by_graph: Vec<(&ProceduralSource, Vec<Channel>)> = Vec::new();
            for c in worked(effect, channels) {
                let pc = channels.project_channel(c);
                match effect.colors.iter().find(|s| s.channels().contains(pc)) {
                    None => {}
                    Some(ProjectSource::Uniform(u)) => {
                        let v = if c.components() == 3 { u.color } else { [u.color[0]; 3] };
                        let opacity = if u.opacity.is_finite() { u.opacity.clamp(0.0, 1.0) } else { 1.0 };
                        out.values.push((c, v.map(|x| if x.is_finite() { x } else { 0.0 }), opacity));
                    }
                    Some(ProjectSource::Procedural(p)) => match by_graph.iter_mut().find(|(q, _)| std::ptr::eq(*q, p)) {
                        Some((_, cs)) => cs.push(c),
                        None => by_graph.push((p, vec![c])),
                    },
                    Some(ProjectSource::Bitmap(b)) => {
                        let texels = match read.iter().find(|(url, _)| *url == b.url) {
                            Some((_, t)) => t.clone(),
                            None => {
                                let image = images.peek(&b.url).map_err(|e| refused("bitmap-unreadable", format!("brush paints an image: {e}")))?;
                                if image.image.width == 0 || image.image.height == 0 {
                                    return Err(refused("bitmap-empty", "brush paints an empty image"));
                                }
                                let t = Arc::new(ReplayTexels::Image(Arc::new(image.image.image_res(!image.float))));
                                read.push((&b.url, t.clone()));
                                t
                            }
                        };
                        out.sources.push((c, ReplaySource::Texels(texels), 1.0));
                    }
                    Some(other) => return Err(paints_source(other)),
                }
            }
            for (p, cs) in by_graph {
                // a channel the graph has no output for is not painted
                for (c, g) in library(p)?.content(p, &cs)? {
                    out.sources.push((c, ReplaySource::Graph(Box::new(g)), 1.0));
                }
            }
            out.sources.sort_by_key(|(c, ..)| *c);
            Ok(out)
        }
    }
}

/// The declared channels an effect works on: those its `channelTypes` lists (all for an empty list).
fn worked(effect: &PaintEffect, channels: &SetChannels) -> Vec<Channel> {
    let mask = effect.header.channels;
    channels.all().filter(|c| mask.is_empty() || channels.mask_has(mask, *c)).collect()
}
