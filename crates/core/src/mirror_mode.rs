//! Which way a mirror filter copies a texture

use serde::{Deserialize, Serialize};

/// The side of an axis, or the corner, a mirror filter copies over the rest of each UV tile
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorMode {
    /// Across a vertical axis, left onto right
    #[default]
    AxisX,
    /// Across a horizontal axis, top onto bottom
    AxisY,
    /// One quarter onto the other three
    Corner,
}

impl MirrorMode {
    /// Every mode, in the order a list shows them
    pub const ALL: [MirrorMode; 3] = [MirrorMode::AxisX, MirrorMode::AxisY, MirrorMode::Corner];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            MirrorMode::AxisX => "Mirror Axis X",
            MirrorMode::AxisY => "Mirror Axis Y",
            MirrorMode::Corner => "Mirror Corner",
        }
    }

    /// The id `effect.set {mode}` takes
    pub fn id(self) -> &'static str {
        match self {
            MirrorMode::AxisX => "axisX",
            MirrorMode::AxisY => "axisY",
            MirrorMode::Corner => "corner",
        }
    }
}
