//! Which side of a shape's edge a Bevel Smooth ramps on

use serde::{Deserialize, Serialize};

/// The side of the edge a Bevel Smooth's ramp lies on
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BevelDirection {
    /// From the edge outwards, the shapes staying white
    #[default]
    Out,
    /// From the edge inwards, around the shapes staying black
    In,
    /// Across the edge, mid gray on it
    InOut,
}

impl BevelDirection {
    /// Every direction, in the order a switch shows them
    pub const ALL: [BevelDirection; 3] = [BevelDirection::Out, BevelDirection::In, BevelDirection::InOut];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            BevelDirection::Out => "Out",
            BevelDirection::In => "In",
            BevelDirection::InOut => "In/Out",
        }
    }

    /// The id `effect.set {direction}` takes
    pub fn id(self) -> &'static str {
        match self {
            BevelDirection::Out => "out",
            BevelDirection::In => "in",
            BevelDirection::InOut => "inOut",
        }
    }
}
