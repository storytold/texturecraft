//! `effect.set` on a Transform filter: repeatMode, flipMode, scaling, offset, tileSafeRotation and rotation

use serde_json::Value;
use texturecraft_core::{FlipMode, RepeatMode, TileTransform};

use crate::Result;
use crate::commands::{bad, boolp, f32p, strp};
use crate::layer_cmds::FULL_TURN_DEGREES;

/// Set the Transform settings given in `p`; a value of the wrong kind is refused and changes nothing
pub(crate) fn apply(p: &Value, transform: &mut TileTransform, cmd: &str) -> Result<()> {
    let mut edited = *transform;

    if p.get("repeatMode").is_some() {
        let id = strp(p, "repeatMode").unwrap_or_default();
        edited.repeat =
            RepeatMode::ALL.into_iter().find(|mode| mode.id() == id).ok_or_else(|| bad(cmd, "`repeatMode` must be none or horizontallyAndVertically"))?;
    }
    if p.get("flipMode").is_some() {
        let id = strp(p, "flipMode").unwrap_or_default();
        edited.flip = FlipMode::ALL
            .into_iter()
            .find(|mode| mode.id() == id)
            .ok_or_else(|| bad(cmd, "`flipMode` must be none, horizontally, vertically or horizontallyAndVertically"))?;
    }

    let (low, high) = TileTransform::SCALING;
    if let Some(scaling) = pair(p, "scaling", cmd)? {
        edited.scaling = scaling.map(|scale| scale.clamp(low, high));
    }
    if let Some(offset) = pair(p, "offset", cmd)? {
        edited.offset = offset.map(|along| along.clamp(-TileTransform::MAX_OFFSET, TileTransform::MAX_OFFSET));
    }

    if p.get("tileSafeRotation").is_some() {
        edited.tile_safe = boolp(p, "tileSafeRotation").ok_or_else(|| bad(cmd, "`tileSafeRotation` must be true or false"))?;
    }
    if p.get("rotation").is_some() {
        let degrees = f32p(p, "rotation").ok_or_else(|| bad(cmd, "`rotation` must be a number of degrees"))?;
        edited.rotation = degrees.rem_euclid(FULL_TURN_DEGREES);
    }

    *transform = edited;
    Ok(())
}

/// `[x, y]` (or one number for both) under `key`, if given; anything else is refused
fn pair(p: &Value, key: &str, cmd: &str) -> Result<Option<[f32; 2]>> {
    let Some(value) = p.get(key) else { return Ok(None) };
    let number = |item: &Value| item.as_f64().map(|value| value as f32).filter(|value| value.is_finite());
    let parsed = match value {
        Value::Array(items) if items.len() == 2 => items.first().and_then(number).zip(items.get(1).and_then(number)).map(|(x, y)| [x, y]),
        other => number(other).map(|both| [both, both]),
    };

    parsed.map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be [x, y] or one number")))
}
