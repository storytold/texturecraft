//! Backface culling of planar and cylindrical projections

use serde::{Deserialize, Serialize};

/// Faces turned more than `angle` degrees from the projection get none of it, faded softer as `hardness` drops
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Backface {
    pub on: bool,
    pub angle: f32,
    pub hardness: f32,
}

impl Backface {
    /// The angle culling may take, in degrees from facing the projector
    pub const ANGLE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=180.0;
}

impl Default for Backface {
    fn default() -> Self {
        Backface { on: false, angle: 90.0, hardness: 0.5 }
    }
}
