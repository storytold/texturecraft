//! Pattern settings in commands: the resolution a pattern is computed at and the Tile Generator's settings

use serde_json::{Map, Value};
use texturecraft_core::{
    OutputResolution, Procedural, ProceduralKind, ResolutionMode, TileBlending, TileGenerator, TileInputDistribution, TileShape, TileSizeMode, TileSymmetry,
};

use crate::Result;
use crate::commands::{bad, boolp, f32p, u64p};

/// The pattern's `scale`, `seed`, `contrast`, `balance` and `invert` in `params`, applied to `pattern` in range
pub(crate) fn pattern_basics_from(params: &Value, pattern: &mut Procedural) {
    if let Some(scale) = f32p(params, "scale") {
        pattern.scale = scale.clamp(Procedural::MIN_SCALE, Procedural::MAX_SCALE);
    }
    if let Some(seed) = u64p(params, "seed") {
        pattern.seed = u32::try_from(seed).unwrap_or(u32::MAX);
    }
    if let Some(contrast) = f32p(params, "contrast") {
        pattern.contrast = contrast.clamp(0.0, Procedural::MAX_CONTRAST);
    }
    if let Some(balance) = f32p(params, "balance") {
        pattern.balance = balance.clamp(-Procedural::MAX_BALANCE, Procedural::MAX_BALANCE);
    }
    if let Some(invert) = boolp(params, "invert") {
        pattern.invert = invert;
    }
}

/// Whether `params` holds any pattern setting [`pattern_settings_from`] reads
pub(crate) fn has_pattern_settings(params: &Value) -> bool {
    ["resolution", "customResolution", "resolutionFactor", "tile", "tileDefaults"].iter().any(|key| params.get(key).is_some())
}

/// Apply the resolution and Tile Generator settings in `params` to `pattern`, using `command` in errors
pub(crate) fn pattern_settings_from(command: &str, params: &Value, pattern: &mut Procedural) -> Result<()> {
    resolution_from(command, params, &mut pattern.resolution)?;
    let resets = boolp(params, "tileDefaults") == Some(true);
    let edits = params.get("tile");
    if (resets || edits.is_some()) && pattern.kind != ProceduralKind::TileGenerator {
        return Err(bad(command, "`tile` and `tileDefaults` go with the Tile Generator pattern"));
    }

    if resets {
        pattern.tile = Some(Box::default());
    }
    if let Some(edits) = edits {
        let current = pattern.tile.get_or_insert_with(Box::default);
        **current = tile_from(command, edits, current)?;
    }
    Ok(())
}

/// The `resolution`, `customResolution` and `resolutionFactor` params applied to `resolution`
pub(crate) fn resolution_from(command: &str, params: &Value, resolution: &mut OutputResolution) -> Result<()> {
    if let Some(value) = params.get("resolution") {
        let name = value.as_str().ok_or_else(|| bad(command, "`resolution` must be a string"))?;
        resolution.mode = ResolutionMode::parse(name).ok_or_else(|| bad(command, format!("unknown resolution `{name}`")))?;
    }
    if let Some(custom) = u64p(params, "customResolution") {
        resolution.custom = custom.clamp(1, u64::from(OutputResolution::MAX_SIZE)) as u32;
    }
    if let Some(factor) = params.get("resolutionFactor") {
        let factor = factor.as_i64().ok_or_else(|| bad(command, "`resolutionFactor` must be a whole number"))?;
        resolution.factor = factor.clamp(i64::from(*OutputResolution::FACTORS.start()), i64::from(*OutputResolution::FACTORS.end())) as i8;
    }
    Ok(())
}

/// `current` with the camelCase settings in `edits` applied (menu choices by label or id), in range
fn tile_from(command: &str, edits: &Value, current: &TileGenerator) -> Result<TileGenerator> {
    let edits = edits.as_object().ok_or_else(|| bad(command, "`tile` must be an object"))?;
    let mut merged = serde_json::to_value(current).map_err(|error| bad(command, error.to_string()))?;
    let fields = merged.as_object_mut().ok_or_else(|| bad(command, "the Tile Generator settings are not an object"))?;
    for (key, value) in edits {
        let field = snake_case(key);
        if !fields.contains_key(&field) {
            return Err(bad(command, format!("unknown Tile Generator setting `{key}`")));
        }

        fields.insert(field.clone(), choice_id(command, &field, value)?);
    }
    let tile: TileGenerator = serde_json::from_value(merged).map_err(|error| bad(command, format!("bad Tile Generator setting: {error}")))?;
    Ok(tile.clamped())
}

/// A menu setting's value as its saved id (`"Half bell"` → `"HalfBell"`); other values unchanged
fn choice_id(command: &str, field: &str, value: &Value) -> Result<Value> {
    let Some(text) = value.as_str() else { return Ok(value.clone()) };
    let id = match field {
        "shape" => find(&TileShape::ALL, TileShape::label, text),
        "size_mode" => find(&TileSizeMode::ALL, TileSizeMode::label, text),
        "input_distribution" => find(&TileInputDistribution::ALL, TileInputDistribution::label, text),
        "symmetry" => find(&TileSymmetry::ALL, TileSymmetry::label, text),
        "blending" => find(&TileBlending::ALL, TileBlending::label, text),
        _ => return Ok(value.clone()),
    };

    id.map(Value::String).ok_or_else(|| bad(command, format!("unknown {} `{text}`", camel_case(field))))
}

/// The id of the choice in `all` whose label or id is `text` (any case, spaces optional)
fn find<T: Copy + std::fmt::Debug>(all: &[T], label: fn(T) -> &'static str, text: &str) -> Option<String> {
    let normalize = |name: &str| name.to_ascii_lowercase().replace([' ', '_', '-', '+'], "");
    let wanted = normalize(text);
    all.iter().find(|choice| normalize(label(**choice)) == wanted || normalize(&format!("{choice:?}")) == wanted).map(|choice| format!("{choice:?}"))
}

/// The Tile Generator settings as `project.inspect` reports them: camelCase keys, as the commands take them
pub(crate) fn tile_report(tile: &TileGenerator) -> Value {
    let Ok(Value::Object(fields)) = serde_json::to_value(tile) else { return Value::Null };
    Value::Object(fields.into_iter().map(|(key, value)| (camel_case(&key), value)).collect::<Map<String, Value>>())
}

fn snake_case(key: &str) -> String {
    let mut snake = String::with_capacity(key.len());
    for character in key.chars() {
        if character.is_ascii_uppercase() {
            snake.push('_');
            snake.push(character.to_ascii_lowercase());
        } else {
            snake.push(character);
        }
    }
    snake
}

fn camel_case(field: &str) -> String {
    let mut camel = String::with_capacity(field.len());
    let mut upper = false;
    for character in field.chars() {
        if character == '_' {
            upper = true;
        } else if upper {
            camel.push(character.to_ascii_uppercase());
            upper = false;
        } else {
            camel.push(character);
        }
    }
    camel
}
