//! Where Color Match takes the color it moves to

use serde::{Deserialize, Serialize};

/// Color Match's Target Color Mode: its Target Color or its Image Input
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetColorMode {
    #[default]
    Parameter,
    ImageInput,
}

impl TargetColorMode {
    /// Every mode, in the order a panel shows them
    pub const ALL: [TargetColorMode; 2] = [TargetColorMode::Parameter, TargetColorMode::ImageInput];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            TargetColorMode::Parameter => "Parameter",
            TargetColorMode::ImageInput => "Image Input",
        }
    }

    /// The id `effect.set {targetColorMode}` takes
    pub fn id(self) -> &'static str {
        match self {
            TargetColorMode::Parameter => "parameter",
            TargetColorMode::ImageInput => "imageInput",
        }
    }
}
