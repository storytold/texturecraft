//! The quarter a corner mirror copies

use serde::{Deserialize, Serialize};

/// The quarter of each UV tile a corner mirror keeps and copies over the other three
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorCorner {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl MirrorCorner {
    /// Every corner, in the order a list shows them
    pub const ALL: [MirrorCorner; 4] = [MirrorCorner::TopLeft, MirrorCorner::TopRight, MirrorCorner::BottomLeft, MirrorCorner::BottomRight];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            MirrorCorner::TopLeft => "Top Left",
            MirrorCorner::TopRight => "Top Right",
            MirrorCorner::BottomLeft => "Bottom Left",
            MirrorCorner::BottomRight => "Bottom Right",
        }
    }

    /// The id `effect.set {cornerType}` takes
    pub fn id(self) -> &'static str {
        match self {
            MirrorCorner::TopLeft => "topLeft",
            MirrorCorner::TopRight => "topRight",
            MirrorCorner::BottomLeft => "bottomLeft",
            MirrorCorner::BottomRight => "bottomRight",
        }
    }

    /// Whether the kept quarter is on the right half and on the bottom half
    pub fn sides(self) -> (bool, bool) {
        match self {
            MirrorCorner::TopLeft => (false, false),
            MirrorCorner::TopRight => (true, false),
            MirrorCorner::BottomLeft => (false, true),
            MirrorCorner::BottomRight => (true, true),
        }
    }
}
