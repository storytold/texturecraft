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
//! | projection tool | a mask's strokes paint the grey of the first `sourceColor`, placed on the view by `projMatrix` |
//! | smudge tool | each dab draws what lies at the previous dab's place over its own, values and coverage (measured) |
//! | `stampBlendingMode` ≠ 0, `aspectRatio` ≠ 1, other `alignment` | approximated (noted) |
//!
//! Not replayable (a `brush:` blocker): particle brushes, ribbons, path strokes, projection painting without an image or
//! on a paint layer, a smudge with nothing of its effect before it, brushes painting images, graphs or anchors,
//! unreadable point lists.

use texturecraft_core::Channel;
use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{PaintEffect, PaintTool, ProjectBrush, ProjectSource, ProjectStroke, StampEmitter, StrokeView};

use super::{PaintMode, ReplayBrush, ReplayStencil, ReplayStep, ReplayStroke, StrokePoint, StrokeSpace, TipAlignment, pressure_at, tip};
use crate::channels::SetChannels;
use crate::resources::ProjectImages;

/// The file's brush size is a diameter in the normalised mesh's units (measured on controlled strokes).
pub const SIZE_TO_RADIUS: f32 = 0.5;
/// Most symmetry copies laid per stroke (a guard; the file stores one in the projects seen).
pub const MAX_COPIES: u32 = 64;
/// Most points read per stroke (a guard against hostile files).
pub const MAX_POINTS: usize = 1 << 20;
/// A stencil without its matrix spans the view: normalised device coordinates −1..1 onto the image's 0..1.
const VIEW_STENCIL: [f32; 9] = [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0];

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
            StampEmitter::Physics(_) => {
                return Err(refused("particle", "particle brush: its emitter and receiver scripts are not carried by TextureCraft's particle brush"));
            }
            StampEmitter::Ribbon(_) => return Err(refused("ribbon", "path ribbon brush (a band laid along a path, no stamps)")),
            StampEmitter::Other(class) if class.is_empty() => return Err(refused("no-emitter", "brush without an emitter")),
            StampEmitter::Other(class) => return Err(Blocker::class(class, format!("brush emitter {class}"))),
        }
    }
    if !effect.paths.is_empty() {
        return Err(refused("3d-path", "path strokes (curves along the mesh) are not replayed"));
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

/// `effect` as a step painting into `target`, the declared `channels` of the set; or why it cannot be replayed.
pub fn step(
    effect: &PaintEffect,
    target: StepTarget,
    channels: &SetChannels,
    images: &ProjectImages,
    graphs: Option<&crate::graph_library::GraphLibrary>,
) -> Result<ReplayStep, Blocker> {
    check(effect)?;
    let mut notes = Vec::new();
    let (tip, tip_note) = tip::tip(&effect.alpha, images, graphs)?;
    notes.extend(tip_note);
    let mut brush = map_brush(effect, tip, &mut notes);
    (brush.cycle, brush.cycle_drawn_start) = tip::cycle(&effect.alpha, graphs);
    if let Some(n) = tip::uncycled(&effect.alpha, brush.cycle.len()) {
        notes.push(n);
    }
    let (mode, values, mask_value) = values(effect, target, channels)?;
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
    let projection = match (effect.tool, effect.colors.first()) {
        (PaintTool::Projection, Some(source)) => Some(ReplayStencil { image: tip::stencil(source, images, graphs)?, matrix }),
        _ => None,
    };
    let strokes: Vec<ReplayStroke> = effect.strokes.iter().filter_map(|s| stroke(s, brush.size_pressure.is_some() || brush.flow_pressure.is_some())).collect();
    if mode == PaintMode::Smudge && strokes.iter().any(|s| s.copies > 0) {
        notes.push("symmetry copies of smudge strokes left out".to_string());
    }
    let opacity = if effect.opacity.is_finite() { effect.opacity.clamp(0.0, 1.0) } else { 1.0 };
    Ok(ReplayStep { mode, values, mask_value, opacity, brush, stencil, projection, strokes, notes })
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

/// The tool's mode, values per channel (content) and grey value (mask).
type Values = (PaintMode, Vec<(Channel, [f32; 3], f32)>, f32);

/// What `effect` lays: the brush paints its `sourceColor` uniforms (linear, as fills); the eraser lowers coverage on the
/// channels it works on, or in a mask paints black over what its strokes reach (measured); the smudge moves what is
/// there on the channels it works on; projection painting in a mask paints its image's grey (the step's projection).
fn values(effect: &PaintEffect, target: StepTarget, channels: &SetChannels) -> Result<Values, Blocker> {
    let paints_source = |s: &ProjectSource| refused(&format!("paints-{}", s.kind()), format!("brush paints a {} source", s.kind()));
    match (target, effect.tool) {
        (StepTarget::Content, PaintTool::Projection) => Err(refused("projection", "projection painting on a paint layer")),
        (StepTarget::Mask, PaintTool::Projection) => Ok((PaintMode::Paint, Vec::new(), 1.0)),
        (StepTarget::Mask, PaintTool::Smudge) => Ok((PaintMode::Smudge, Vec::new(), 0.0)),
        (StepTarget::Content, PaintTool::Smudge) => {
            let worked = worked(effect, channels).into_iter().map(|c| (c, [0.0; 3], 1.0)).collect();
            Ok((PaintMode::Smudge, worked, 0.0))
        }
        (StepTarget::Mask, PaintTool::Eraser) => Ok((PaintMode::Paint, Vec::new(), 0.0)),
        (StepTarget::Mask, _) => match effect.colors.first() {
            None => Ok((PaintMode::Paint, Vec::new(), 1.0)),
            Some(ProjectSource::Uniform(u)) => {
                let v = u.color[0] * u.opacity;
                Ok((PaintMode::Paint, Vec::new(), if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }))
            }
            Some(other) => Err(paints_source(other)),
        },
        (StepTarget::Content, PaintTool::Eraser) => {
            let worked = worked(effect, channels).into_iter().map(|c| (c, [0.0; 3], 1.0)).collect();
            Ok((PaintMode::Erase, worked, 0.0))
        }
        (StepTarget::Content, _) => {
            let mut out = Vec::new();
            for c in worked(effect, channels) {
                let pc = channels.project_channel(c);
                match effect.colors.iter().find(|s| s.channels().contains(pc)) {
                    None => {}
                    Some(ProjectSource::Uniform(u)) => {
                        let v = if c.components() == 3 { u.color } else { [u.color[0]; 3] };
                        let opacity = if u.opacity.is_finite() { u.opacity.clamp(0.0, 1.0) } else { 1.0 };
                        out.push((c, v.map(|x| if x.is_finite() { x } else { 0.0 }), opacity));
                    }
                    Some(other) => return Err(paints_source(other)),
                }
            }
            Ok((PaintMode::Paint, out, 0.0))
        }
    }
}

/// The declared channels an effect works on: those its `channelTypes` lists (all for an empty list).
fn worked(effect: &PaintEffect, channels: &SetChannels) -> Vec<Channel> {
    let mask = effect.header.channels;
    channels.all().filter(|c| mask.is_empty() || channels.mask_has(mask, *c)).collect()
}
