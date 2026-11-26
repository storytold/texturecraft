use serde::{Deserialize, Deserializer, Serialize};

use super::DecalFlip;

/// The mirrored copy of a decal: reflected through the plane perpendicular to `axis`, across the mesh center (moved by `offset`)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecalSymmetry {
    /// Axis the mirror plane is perpendicular to (0 = X, 1 = Y, 2 = Z)
    #[serde(deserialize_with = "deserialize_axis")]
    pub axis: u8,
    /// Extra flip of the image on the mirrored copy, on top of the reflection itself
    #[serde(default)]
    pub flip: DecalFlip,
    /// Where the mirror plane sits, relative to the mesh center along `axis` (world units, as the brush symmetry's offset)
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f32,
}

fn deserialize_axis<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    // Recover malformed projects with the same Z plane used by the renderer
    Ok(u8::deserialize(deserializer)?.min(2))
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}
