//! A paint effect's brush alpha (`sourceTransparent`) as a [`ReplayTip`].
//!
//! - The newer round shape (`shape_v2`) is TextureCraft's round alpha: full inside the hardness, a smoothstep to the rim
//!   (measured).
//! - The older round shape (`shape`): a smoothstep from 1 at 0.2 of the radius to 0 at the rim, divided by
//!   `1 − hardness` and cut at 1 (measured); drawn as an image.
//! - A bitmap alpha: its red × alpha, as an image. A uniform alpha: a full square of its value.
//! - Any other alpha is a compiled graph: [`graph_tip`] renders its grey output with its tweaks at [`TIP_SIDE`] from the
//!   project's decompiled graphs; without them it is a `brush-alpha:<graph>` blocker.
//! - A dynamic alpha (a graph with a `stampIndex` input, or a source wired to the stroke's seed) cycles through stamps:
//!   [`cycle`] renders each.
//! - A stencil ([`stencil`]): a uniform source is its grey everywhere; a bitmap its red × alpha; a graph its grey output
//!   at [`STENCIL_SIDE`].

use std::sync::Arc;

use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{Procedural, ProjectSource, STROKE_WIRE_RANDOM_START, STROKE_WIRE_SEED, Tweak, TweakValue};

use super::brush::refused;
use super::{ReplayTip, StencilImage};
use crate::graph_library::GraphLibrary;
use crate::resources::ProjectImages;

/// Side of a drawn tip image.
pub const TIP_SIDE: u32 = 256;
/// Longest side a bitmap tip keeps (larger ones are box-reduced).
pub const MAX_BITMAP_SIDE: u32 = 1024;
/// Side of a stencil drawn by its graph (it spans the whole view, so finer than a tip), and the longest side a bitmap
/// stencil keeps.
pub const STENCIL_SIDE: u32 = 1024;
pub const MAX_STENCIL_SIDE: u32 = 2048;
/// Most stamps of a dynamic alpha's cycle rendered.
pub const MAX_CYCLE: u32 = 256;
/// A dynamic alpha's inputs (file data: the input names of the alpha graphs): the stamp a dab lays, and how many the
/// cycle has.
const STAMP_INDEX: &str = "stampIndex";
const STAMP_COUNT: &str = "stampCycleCount";
/// The graph input taking the random seed (file data).
const SEED: &str = "$randomseed";
/// The older round shape: the share of the radius where its smoothstep starts (measured).
const OLD_SHAPE_START: f32 = 0.2;
/// Name of the newer and the older round shape graphs (file data: the graph names the file refers to).
const ROUND_SHAPE: &str = "shape_v2";
const OLD_ROUND_SHAPE: &str = "shape";

/// The tip of `alpha` (the first entry is the alpha) and a note when it is approximated; or why it cannot be had.
pub fn tip(alpha: &[ProjectSource], images: &ProjectImages, graphs: Option<&GraphLibrary>) -> Result<(ReplayTip, Option<String>), Blocker> {
    match alpha.first() {
        // no alpha: the older round shape, soft
        None => Ok((old_round(0.0), None)),
        Some(ProjectSource::Procedural(p)) => {
            let q = p.procedural.as_ref().ok_or_else(|| refused("alpha-without-procedural", "brush alpha without a graph"))?;
            match (q.name(), round_hardness(&q.tweaks)) {
                (ROUND_SHAPE, Some(h)) => Ok((ReplayTip::Round { hardness: h }, None)),
                (OLD_ROUND_SHAPE, Some(h)) => Ok((old_round(h), None)),
                _ => Ok((graph_tip(q, graphs)?, None)),
            }
        }
        Some(ProjectSource::Bitmap(b)) => {
            let read = images.peek(&b.url).map_err(|e| refused("alpha-bitmap-unreadable", format!("brush alpha {e}")))?;
            bitmap(&read.image).map(|t| (t, None)).ok_or_else(|| refused("alpha-bitmap-empty", "brush alpha bitmap is empty"))
        }
        Some(ProjectSource::Uniform(u)) => {
            let v = u.color[0] * u.opacity;
            let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            Ok((ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![v]) }, None))
        }
        Some(other) => Err(refused(&format!("alpha-source-{}", other.kind()), format!("brush alpha source {}", other.kind()))),
    }
}

/// The tip a compiled-graph alpha renders: its grey output with the graph's own tweaks at [`TIP_SIDE`]. Without the
/// project's graphs decompiled, or when the graph does not render, the graph's blocker.
pub fn graph_tip(graph: &Procedural, graphs: Option<&GraphLibrary>) -> Result<ReplayTip, Blocker> {
    let name = graph.name();
    let blocked = |why: &str| Blocker::new(BlockerKey::new(BlockerKind::BrushAlpha, name), format!("brush alpha \"{name}\" {why}"));
    let lib = graphs.ok_or_else(|| blocked("is a compiled graph (graphs are kept as placeholders)"))?;
    let values = crate::graph_uses::render_grey(lib, graph, TIP_SIDE).ok_or_else(|| blocked("is a compiled graph that does not render"))?;
    Ok(ReplayTip::Image { width: TIP_SIDE, height: TIP_SIDE, values: Arc::from(values) })
}

/// The stamps a dynamic alpha cycles through, and whether the cycle starts at a stamp drawn from the stroke's seed
/// (measured): as many as the source's `stampCycleCount` says (without one, the graph's own), stamp `i` rendered with
/// `stampIndex` `i` and the count set to that number too (the graph's stored count is not used). A source wired to the
/// stroke's seed gets a cycle too, stamp `i` rendered with seed `i` (inferred: only that the stamps differ is known).
/// Empty for any other alpha, or when a stamp does not render (every dab then lays the tip).
pub fn cycle(alpha: &[ProjectSource], graphs: Option<&GraphLibrary>) -> (Vec<ReplayTip>, bool) {
    let (Some(ProjectSource::Procedural(p)), Some(lib)) = (alpha.first(), graphs) else { return (Vec::new(), false) };
    let Some(q) = p.procedural.as_ref() else { return (Vec::new(), false) };
    if matches!(q.name(), ROUND_SHAPE | OLD_ROUND_SHAPE) && round_hardness(&q.tweaks).is_some() {
        // drawn by its measured curve: the same for every stamp
        return (Vec::new(), false);
    }
    let int = |id: &str| {
        q.tweaks.iter().find(|t| t.identifier == id).map(|t| match &t.value {
            TweakValue::Int(n) => *n,
            TweakValue::Floats(f) => f.first().copied().filter(|v| v.is_finite()).unwrap_or(0.0) as i32,
            _ => 0,
        })
    };
    let indexed = int(STAMP_INDEX).is_some();
    let seeded = p.stroke_wires & STROKE_WIRE_SEED != 0;
    if !indexed && !seeded {
        return (Vec::new(), false);
    }
    let count = match p.stamp_cycle {
        0 => int(STAMP_COUNT).unwrap_or(1),
        n => n.min(MAX_CYCLE) as i32,
    }
    .clamp(1, MAX_CYCLE as i32);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mut stamp = q.clone();
        let mut set = |id: &str, v: i32| match stamp.tweaks.iter_mut().find(|t| t.identifier == id) {
            Some(t) => t.value = TweakValue::Int(v),
            None => stamp.tweaks.push(Tweak { identifier: id.to_string(), value: TweakValue::Int(v) }),
        };
        if indexed {
            set(STAMP_INDEX, i);
            set(STAMP_COUNT, count);
        }
        if seeded {
            set(SEED, i);
        }
        let Some(values) = crate::graph_uses::render_grey(lib, &stamp, TIP_SIDE) else { return (Vec::new(), false) };
        out.push(ReplayTip::Image { width: TIP_SIDE, height: TIP_SIDE, values: Arc::from(values) });
    }
    (out, p.stroke_wires & STROKE_WIRE_RANDOM_START != 0)
}

/// The note for a dynamic alpha whose stamps could not be rendered (`cycled` of them were): every dab lays the first.
pub fn uncycled(alpha: &[ProjectSource], cycled: usize) -> Option<String> {
    let Some(ProjectSource::Procedural(p)) = alpha.first() else { return None };
    let q = p.procedural.as_ref()?;
    (cycled == 0 && p.stamp_cycle > 1).then(|| format!("brush alpha \"{}\" rendered once (its {} stamps do not cycle)", q.name(), p.stamp_cycle))
}

/// A stencil's image from its source; or why it cannot be had.
pub fn stencil(source: &ProjectSource, images: &ProjectImages, graphs: Option<&GraphLibrary>) -> Result<StencilImage, Blocker> {
    match source {
        ProjectSource::Uniform(u) => {
            let v = u.color[0] * u.opacity;
            Ok(StencilImage::Uniform(if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }))
        }
        ProjectSource::Bitmap(b) => {
            let read = images.peek(&b.url).map_err(|e| refused("stencil-bitmap-unreadable", format!("stencil {e}")))?;
            match grey(&read.image, MAX_STENCIL_SIDE) {
                Some((width, height, values)) => Ok(StencilImage::Image { width, height, values: Arc::from(values) }),
                None => Err(refused("stencil-bitmap-empty", "stencil bitmap is empty")),
            }
        }
        ProjectSource::Procedural(p) => {
            let q = p.procedural.as_ref().ok_or_else(|| refused("stencil-without-procedural", "stencil without a graph"))?;
            let name = q.name();
            let blocked = |why: &str| Blocker::new(BlockerKey::new(BlockerKind::BrushAlpha, name), format!("stencil \"{name}\" {why}"));
            let lib = graphs.ok_or_else(|| blocked("is a compiled graph (graphs are kept as placeholders)"))?;
            let values = crate::graph_uses::render_grey(lib, q, STENCIL_SIDE).ok_or_else(|| blocked("is a compiled graph that does not render"))?;
            Ok(StencilImage::Image { width: STENCIL_SIDE, height: STENCIL_SIDE, values: Arc::from(values) })
        }
        other => Err(refused(&format!("stencil-source-{}", other.kind()), format!("stencil source {}", other.kind()))),
    }
}

/// The round shape `alpha` is (the first entry is the alpha), as the hardness of TextureCraft's round tip, and whether
/// that is the same curve: the newer round shape is, the older one is approximated by a falloff starting where its own
/// does. No alpha is the older shape, soft. `None` for any other alpha (bitmaps, other graphs, constants).
pub fn round_tip_hardness(alpha: &[ProjectSource]) -> Option<(f32, bool)> {
    let old = |h: f32| (OLD_SHAPE_START + (1.0 - OLD_SHAPE_START) * h, false);
    match alpha.first() {
        None => Some(old(0.0)),
        Some(ProjectSource::Procedural(p)) => {
            let q = p.procedural.as_ref()?;
            match (q.name(), round_hardness(&q.tweaks)) {
                (ROUND_SHAPE, Some(h)) => Some((h, true)),
                (OLD_ROUND_SHAPE, Some(h)) => Some(old(h)),
                _ => None,
            }
        }
        Some(_) => None,
    }
}

/// The hardness of a round shape with no other setting away from its default; `None` for any other shape.
fn round_hardness(tweaks: &[Tweak]) -> Option<f32> {
    let mut hardness = 0.0;
    let off = |v: &[f32], d: f32| v.iter().any(|r| (r - d).abs() > 1e-3);
    for t in tweaks {
        match (t.identifier.as_str(), &t.value) {
            ("hardness", TweakValue::Floats(v)) => hardness = v.first().copied().filter(|h| h.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0),
            ("roundness" | "size" | "size_ratio", TweakValue::Floats(v)) if off(v, 1.0) => return None,
            ("shape", TweakValue::Int(n)) if *n != 0 => return None,
            ("angle" | "rotation", TweakValue::Floats(v)) if off(v, 0.0) => return None,
            _ => {}
        }
    }
    Some(hardness)
}

/// The older round shape at `hardness` (measured): `min(1, (1 − S((r − 0.2) / 0.8)) / (1 − h))`, `r` the share of the
/// radius.
pub fn old_round_at(r: f32, hardness: f32) -> f32 {
    if r.is_nan() || r >= 1.0 {
        return 0.0;
    }
    let t = ((r - OLD_SHAPE_START) / (1.0 - OLD_SHAPE_START)).clamp(0.0, 1.0);
    let v = 1.0 - t * t * (3.0 - 2.0 * t);
    let keep = (1.0 - hardness.clamp(0.0, 1.0)).max(1e-4);
    (v / keep).min(1.0)
}

/// [`old_round_at`] drawn over the tip's square.
fn old_round(hardness: f32) -> ReplayTip {
    let n = TIP_SIDE as usize;
    let values: Vec<f32> = (0..n * n)
        .map(|i| {
            let x = ((i % n) as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let y = ((i / n) as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            old_round_at((x * x + y * y).sqrt(), hardness)
        })
        .collect();
    ReplayTip::Image { width: TIP_SIDE, height: TIP_SIDE, values: Arc::from(values) }
}

/// A bitmap's red × alpha, box-reduced to at most [`MAX_BITMAP_SIDE`] a side; `None` when empty.
fn bitmap(image: &crate::image::Rgba) -> Option<ReplayTip> {
    grey(image, MAX_BITMAP_SIDE).map(|(width, height, values)| ReplayTip::Image { width, height, values: Arc::from(values) })
}

/// A bitmap's red × alpha, box-reduced to at most `max_side` a side: `(width, height, values)`; `None` when empty.
fn grey(image: &crate::image::Rgba, max_side: u32) -> Option<(u32, u32, Vec<f32>)> {
    let (w, h) = (image.width, image.height);
    if w == 0 || h == 0 {
        return None;
    }
    let step = w.max(h).div_ceil(max_side.max(1)).max(1);
    let (tw, th) = (w.div_ceil(step), h.div_ceil(step));
    let mut values = Vec::with_capacity(tw as usize * th as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let (mut sum, mut n) = (0.0, 0.0);
            for y in ty * step..((ty + 1) * step).min(h) {
                for x in tx * step..((tx + 1) * step).min(w) {
                    let p = image.get(x, y);
                    let g = p[0] * p[3];
                    sum += if g.is_finite() { g.clamp(0.0, 1.0) } else { 0.0 };
                    n += 1.0;
                }
            }
            values.push(if n > 0.0 { sum / n } else { 0.0 });
        }
    }
    Some((tw, th, values))
}
