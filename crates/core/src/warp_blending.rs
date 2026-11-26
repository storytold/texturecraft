//! How a Multi Directional Warp combines its directions

use serde::{Deserialize, Serialize};

/// How the reads of a Multi Directional Warp's directions combine
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarpBlending {
    #[default]
    Average,
    /// The largest of them, per component
    Max,
    /// The smallest of them, per component
    Min,
    /// One move after the other, each from where the last one landed
    Chain,
}

impl WarpBlending {
    /// Every choice, in the order a panel lists them
    pub const ALL: [WarpBlending; 4] = [WarpBlending::Average, WarpBlending::Max, WarpBlending::Min, WarpBlending::Chain];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            WarpBlending::Average => "Average",
            WarpBlending::Max => "Max",
            WarpBlending::Min => "Min",
            WarpBlending::Chain => "Chain",
        }
    }

    /// The id `effect.set` takes
    pub fn id(self) -> &'static str {
        match self {
            WarpBlending::Average => "average",
            WarpBlending::Max => "max",
            WarpBlending::Min => "min",
            WarpBlending::Chain => "chain",
        }
    }
}
