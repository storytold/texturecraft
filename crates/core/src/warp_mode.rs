//! How a Warp moves the texels it reads

use serde::{Deserialize, Serialize};

/// Which way a Warp moves texels: along its source's slope, along one direction, or along several
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarpMode {
    /// Along the source's slope, each texel its own way
    #[default]
    Warp,
    /// Along the Angle, forwards or backwards by the source
    Directional,
    /// Along Directions spread evenly from the Angle, blended
    MultiDirectional,
}

impl WarpMode {
    /// Every choice, in the order a panel lists them
    pub const ALL: [WarpMode; 3] = [WarpMode::Warp, WarpMode::Directional, WarpMode::MultiDirectional];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            WarpMode::Warp => "Warp",
            WarpMode::Directional => "Directional Warp",
            WarpMode::MultiDirectional => "Multi Directional Warp",
        }
    }

    /// The id `effect.set` takes
    pub fn id(self) -> &'static str {
        match self {
            WarpMode::Warp => "warp",
            WarpMode::Directional => "directional",
            WarpMode::MultiDirectional => "multiDirectional",
        }
    }
}
