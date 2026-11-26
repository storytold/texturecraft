//! What drives a Warp

use serde::{Deserialize, Serialize};

/// The values a Warp's moves follow
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarpSourceMode {
    /// Fractal noise of the Warp's seed
    #[default]
    DefaultNoise,
    /// The values the Warp filters (their luminance)
    PreviousInput,
    /// An image input (until one is linked: flat, so nothing moves)
    CustomNoise,
}

impl WarpSourceMode {
    /// Every choice, in the order a panel lists them
    pub const ALL: [WarpSourceMode; 3] = [WarpSourceMode::DefaultNoise, WarpSourceMode::PreviousInput, WarpSourceMode::CustomNoise];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            WarpSourceMode::DefaultNoise => "Default Noise",
            WarpSourceMode::PreviousInput => "Previous Input",
            WarpSourceMode::CustomNoise => "Custom Noise",
        }
    }

    /// The id `effect.set` takes
    pub fn id(self) -> &'static str {
        match self {
            WarpSourceMode::DefaultNoise => "defaultNoise",
            WarpSourceMode::PreviousInput => "previousInput",
            WarpSourceMode::CustomNoise => "customNoise",
        }
    }
}
