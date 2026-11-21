//! Which side of a mask's edge a Mask Outline draws on

use serde::{Deserialize, Serialize};

/// The side of the threshold edge a Mask Outline's band lies on
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutlinePosition {
    /// Around the shapes, outside them
    #[default]
    Outside,
    /// Inside the shapes, along their edge
    Inside,
    /// On both sides of the edge
    Both,
}

impl OutlinePosition {
    /// Every position, in the order a switch shows them
    pub const ALL: [OutlinePosition; 3] = [OutlinePosition::Outside, OutlinePosition::Inside, OutlinePosition::Both];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            OutlinePosition::Outside => "Outside",
            OutlinePosition::Inside => "Inside",
            OutlinePosition::Both => "Both",
        }
    }

    /// The id `effect.set {maskPosition}` takes
    pub fn id(self) -> &'static str {
        match self {
            OutlinePosition::Outside => "outside",
            OutlinePosition::Inside => "inside",
            OutlinePosition::Both => "both",
        }
    }

    /// Whether the band covers a texel inside the shapes (`inside`)
    pub fn covers(self, inside: bool) -> bool {
        match self {
            OutlinePosition::Outside => !inside,
            OutlinePosition::Inside => inside,
            OutlinePosition::Both => true,
        }
    }
}
