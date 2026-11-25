use serde::{Deserialize, Serialize};

use super::axis_blend::AxisBlend;

/// One axis of a World Space Normals or Position generator: its channel, inverted or not, blended over the axes before it
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AxisLayer {
    /// The axis read from its far side
    pub invert: bool,
    pub blend: AxisBlend,
    /// How much of the blend shows (0..1)
    pub weight: f32,
}
