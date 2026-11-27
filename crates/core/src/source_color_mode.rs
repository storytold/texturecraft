//! Where Color Match takes the color it moves from

use serde::{Deserialize, Serialize};

/// Color Match's Source Color Mode: the mean color of the whole texture, its Source Color, or its Input image
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceColorMode {
    #[default]
    Average,
    Parameter,
    Input,
}

impl SourceColorMode {
    /// Every mode, in the order a panel shows them
    pub const ALL: [SourceColorMode; 3] = [SourceColorMode::Average, SourceColorMode::Parameter, SourceColorMode::Input];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            SourceColorMode::Average => "Average",
            SourceColorMode::Parameter => "Parameter",
            SourceColorMode::Input => "Input",
        }
    }

    /// The id `effect.set {sourceColorMode}` takes
    pub fn id(self) -> &'static str {
        match self {
            SourceColorMode::Average => "average",
            SourceColorMode::Parameter => "parameter",
            SourceColorMode::Input => "input",
        }
    }
}
