use crate::commands::{CommandSpec, always, bad, boolp, f32p, strp, u64p};
use crate::{Result, Session, cmd};
use serde_json::Value;
use std::sync::Arc;
use texturecraft_bake::BakePaintMap;
use texturecraft_core::{BakePaintTarget, BakedMap};
use texturecraft_geom::glam::Vec2;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("bake.skew.set", "Set Skew Correction", [], None, "{ts?, value?: 0..1 | path: grayscale image}", always, |session, parameters| set(
            session,
            parameters,
            BakePaintTarget::Skew
        )),
        cmd!(
            "bake.skew.paint",
            "Paint Skew Correction",
            [],
            None,
            "{ts?, points: [[u,v],...], radius?: 0.03, value?: 1, hardness?: 0.5, strength?: 1, size?: 1024, continue?: false}",
            always,
            |session, parameters| paint(session, parameters, BakePaintTarget::Skew)
        ),
        cmd!("bake.skew.clear", "Remove Skew Correction", [], None, "{ts?}", always, |session, parameters| clear(session, parameters, BakePaintTarget::Skew)),
        cmd!(
            "bake.cage.paint",
            "Paint Cage",
            [],
            None,
            "{ts?, points: [[u,v],...], radius?: 0.03, value?: 1 (frontal distance multiplier, 0..1), hardness?: 0.5, strength?: 1, size?: 1024, continue?: false}",
            always,
            |session, parameters| paint(session, parameters, BakePaintTarget::Cage)
        ),
        cmd!("bake.cage.reset", "Reset Painted Cage", [], None, "{ts?}", always, |session, parameters| clear(session, parameters, BakePaintTarget::Cage)),
    ]
}

fn texture_set(session: &Session, parameters: &Value) -> Result<usize> {
    let index = match parameters.get("ts") {
        None => session.ts(),
        Some(value) => value.as_u64().and_then(|value| usize::try_from(value).ok()).ok_or_else(|| bad("bake.paint", "ts must be a texture set index"))?,
    };
    session.doc.project.texture_set(index)?;
    Ok(index)
}

fn number(parameters: &Value, name: &str, default: f32, minimum: f32, maximum: f32) -> Result<f32> {
    if parameters.get(name).is_none() {
        return Ok(default);
    }
    f32p(parameters, name)
        .filter(|value| (minimum..=maximum).contains(value))
        .ok_or_else(|| bad("bake.paint", format!("{name} must be in {minimum}..={maximum}")))
}

fn set(session: &mut Session, parameters: &Value, target: BakePaintTarget) -> Result<Value> {
    let texture_set = texture_set(session, parameters)?;
    let map = match strp(parameters, "path") {
        Some(path) => load_image(path)?,
        None => BakePaintMap::constant(number(parameters, "value", 1.0, 0.0, 1.0)?),
    };
    map.validate().map_err(|error| bad("bake.paint", error.to_string()))?;

    session.edit("Set Bake Paint Map", |session| {
        session.doc.bake_paint.insert((texture_set, target), Arc::new(map));
        Ok(())
    })?;
    session.note_bake_paint(texture_set, None);

    Ok(super::meshes_json(session))
}

fn clear(session: &mut Session, parameters: &Value, target: BakePaintTarget) -> Result<Value> {
    let texture_set = texture_set(session, parameters)?;
    if session.doc.bake_paint.contains_key(&(texture_set, target)) {
        session.edit("Clear Bake Paint Map", |session| {
            session.doc.bake_paint.remove(&(texture_set, target));
            Ok(())
        })?;
        session.note_bake_paint(texture_set, None);
    }

    Ok(super::meshes_json(session))
}

/// Most UV points one `bake.paint` call takes
const MAX_STROKE_POINTS: usize = 100_000;
/// Brush radius in UV units: the default, and the smallest that still paints
const DEFAULT_RADIUS: f32 = 0.03;
const MIN_RADIUS: f32 = 0.00001;
/// Brush hardness when a call gives none: halfway between a soft and a hard edge
const DEFAULT_HARDNESS: f32 = 0.5;
/// Side of a new paint map in texels: the default, and the range a call may ask for
const DEFAULT_PAINT_SIZE_TEXELS: u64 = 1024;
const PAINT_SIZE_RANGE_TEXELS: std::ops::RangeInclusive<u64> = 16..=4096;

fn paint(session: &mut Session, parameters: &Value, target: BakePaintTarget) -> Result<Value> {
    let texture_set = texture_set(session, parameters)?;
    let points = parameters
        .get("points")
        .and_then(Value::as_array)
        .filter(|points| !points.is_empty() && points.len() <= MAX_STROKE_POINTS)
        .ok_or_else(|| bad("bake.paint", format!("points must contain 1 to {MAX_STROKE_POINTS} UV coordinates")))?;

    let points = points
        .iter()
        .map(|point| {
            let coordinate = |index| point.get(index).and_then(Value::as_f64).map(|value| value as f32).filter(|value| value.is_finite());
            match (coordinate(0), coordinate(1)) {
                (Some(u), Some(v)) => Ok(Vec2::new(u, v)),
                _ => Err(bad("bake.paint", "each point must contain two finite UV coordinates")),
            }
        })
        .collect::<Result<Vec<_>>>()?;

    let value = number(parameters, "value", 1.0, 0.0, 1.0)?;
    let radius = number(parameters, "radius", DEFAULT_RADIUS, MIN_RADIUS, 1.0)?;
    let hardness = number(parameters, "hardness", DEFAULT_HARDNESS, 0.0, 1.0)?;
    let strength = number(parameters, "strength", 1.0, 0.0, 1.0)?;
    let size = match parameters.get("size") {
        None => DEFAULT_PAINT_SIZE_TEXELS,
        Some(_) => u64p(parameters, "size").ok_or_else(|| bad("bake.paint", "size must be an integer"))?,
    };
    if !PAINT_SIZE_RANGE_TEXELS.contains(&size) {
        return Err(bad("bake.paint", format!("size must be between {} and {}", PAINT_SIZE_RANGE_TEXELS.start(), PAINT_SIZE_RANGE_TEXELS.end())));
    }

    let default = match target {
        BakePaintTarget::Cage => 1.0,
        BakePaintTarget::Skew => 0.0,
    };
    let label = match target {
        BakePaintTarget::Cage => "Paint Cage",
        BakePaintTarget::Skew => "Paint Skew Correction",
    };

    session.edit_merging(label, boolp(parameters, "continue") == Some(true), |session| {
        // Skew painting starts with Auto-rebake on for the Normal map, which shows the skew best, unless the set picked one
        let first_skew = target == BakePaintTarget::Skew && !session.doc.bake_paint.contains_key(&(texture_set, target));
        let set = session.doc.project.texture_set_mut(texture_set)?;
        if first_skew && set.auto_rebake.is_none() {
            set.auto_rebake = Some(BakedMap::Normal);
        }

        let map = session.doc.bake_paint.entry((texture_set, target)).or_insert_with(|| Arc::new(BakePaintMap::constant(default)));
        if map.values.len() == 1 {
            *map = Arc::new(map.resized(size as u32, size as u32).map_err(|error| bad("bake.paint", error.to_string()))?);
        }
        Arc::make_mut(map).paint(&points, radius, value, hardness, strength).map_err(|error| bad("bake.paint", error.to_string()))
    })?;

    // Bilinear sampling extends each dab's reach by one paint texel
    let paint_texel = session.doc.bake_paint.get(&(texture_set, target)).map_or(0.0, |map| 1.0 / map.width.min(map.height).max(1) as f32);
    let reach = Vec2::splat(radius + paint_texel);
    let uv_min = points.iter().fold(Vec2::INFINITY, |min, point| min.min(*point)) - reach;
    let uv_max = points.iter().fold(Vec2::NEG_INFINITY, |max, point| max.max(*point)) + reach;

    session.note_bake_paint(texture_set, Some((uv_min, uv_max)));

    Ok(super::meshes_json(session))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_image(path: &str) -> Result<BakePaintMap> {
    let length = std::fs::metadata(path).map_err(|error| bad("bake.paint", error.to_string()))?.len();
    if length > 256 << 20 {
        return Err(bad("bake.paint", "image exceeds 256 MiB"));
    }
    let bytes = std::fs::read(path).map_err(|error| bad("bake.paint", error.to_string()))?;
    let image = texturecraft_formats::decode_image_within(&bytes, texturecraft_bake::MAX_TEXELS).map_err(|error| bad("bake.paint", error.to_string()))?;
    Ok(BakePaintMap {
        width: image.width,
        height: image.height,
        values: image.rgba.iter().map(|pixel| ((pixel[0] + pixel[1] + pixel[2]) / 3.0).clamp(0.0, 1.0)).collect(),
    })
}

#[cfg(target_arch = "wasm32")]
fn load_image(_: &str) -> Result<BakePaintMap> {
    Err(bad("bake.paint", "file paths are unavailable in the browser"))
}
