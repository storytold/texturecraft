//! The shape a Bevel gives the corners it rounds off

use serde::{Deserialize, Serialize};

/// How a Bevel measures the distance from a shape's edge, which shows at its corners
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BevelCorner {
    /// Straight-line distance: corners the bevel goes around come out round
    #[default]
    Round,
    /// The larger of the horizontal and vertical distance: corners stay square
    Angular,
}

impl BevelCorner {
    /// Every corner type, in the order a switch shows them
    pub const ALL: [BevelCorner; 2] = [BevelCorner::Round, BevelCorner::Angular];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            BevelCorner::Round => "Round",
            BevelCorner::Angular => "Angular",
        }
    }

    /// The id `effect.set {cornerType}` takes
    pub fn id(self) -> &'static str {
        match self {
            BevelCorner::Round => "round",
            BevelCorner::Angular => "angular",
        }
    }
}
