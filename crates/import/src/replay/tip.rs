//! A paint effect's brush alpha (`sourceTransparent`) as a [`ReplayTip`].
//!
//! - The newer round shape (`shape_v2`) is TextureCraft's round alpha: full inside the hardness, a smoothstep to the rim
//!   (measured).
//! - The older round shape (`shape`): a smoothstep from 1 at 0.2 of the radius to 0 at the rim, divided by
//!   `1 − hardness` and cut at 1 (measured); drawn as an image.
//! - A bitmap alpha: its red × alpha, as an image. A uniform alpha: a full square of its value.
//! - Any other alpha is a compiled graph: [`graph_tip`] renders its grey output with its tweaks at [`TIP_SIDE`] from the
//!   project's decompiled graphs; without them it is a `brush-alpha:<graph>` blocker.

use std::sync::Arc;

use texturecraft_import_reader::blockers::{Blocker, BlockerKey, BlockerKind};
use texturecraft_import_reader::model::{Procedural, ProjectSource, Tweak, TweakValue};

use super::ReplayTip;
use super::brush::refused;
use crate::graph_library::GraphLibrary;
use crate::resources::ProjectImages;

/// Side of a drawn tip image.
pub const TIP_SIDE: u32 = 256;
/// Longest side a bitmap tip keeps (larger ones are box-reduced).
pub const MAX_BITMAP_SIDE: u32 = 1024;
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
                _ => {
                    let tip = graph_tip(q, graphs)?;
                    // a dynamic alpha cycles through stamps; rendered once, every dab takes the first
                    let note = (p.stamp_cycle > 1).then(|| format!("brush alpha \"{}\" rendered once (its {} stamps do not cycle)", q.name(), p.stamp_cycle));
                    Ok((tip, note))
                }
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
    let (w, h) = (image.width, image.height);
    if w == 0 || h == 0 {
        return None;
    }
    let step = w.max(h).div_ceil(MAX_BITMAP_SIDE).max(1);
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
    Some(ReplayTip::Image { width: tw, height: th, values: Arc::from(values) })
}
