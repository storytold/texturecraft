//! How a color becomes the gray a gradient looks up

use serde::{Deserialize, Serialize};

/// The gray a color reads as, before a gradient maps it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrayscaleMode {
    /// Halfway between the lightest and darkest component
    Desaturation,
    /// Weighted by how bright each component looks (what gradients read in projects saved before the modes)
    #[default]
    Luma,
    /// The components' mean
    Average,
    /// The lightest component
    Max,
    /// The darkest component
    Min,
}

impl GrayscaleMode {
    /// Every mode, in the order a list shows them
    pub const ALL: [GrayscaleMode; 5] = [GrayscaleMode::Desaturation, GrayscaleMode::Luma, GrayscaleMode::Average, GrayscaleMode::Max, GrayscaleMode::Min];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            GrayscaleMode::Desaturation => "Desaturation",
            GrayscaleMode::Luma => "Luma",
            GrayscaleMode::Average => "Average",
            GrayscaleMode::Max => "Max",
            GrayscaleMode::Min => "Min",
        }
    }

    /// The id `effect.set {grayscaleMode}` takes
    pub fn id(self) -> &'static str {
        match self {
            GrayscaleMode::Desaturation => "desaturation",
            GrayscaleMode::Luma => "luma",
            GrayscaleMode::Average => "average",
            GrayscaleMode::Max => "max",
            GrayscaleMode::Min => "min",
        }
    }

    /// The gray of linear color `[r, g, b]`
    pub fn gray(self, [r, g, b]: [f32; 3]) -> f32 {
        match self {
            GrayscaleMode::Desaturation => (r.max(g).max(b) + r.min(g).min(b)) / 2.0,
            GrayscaleMode::Luma => crate::blend::lum([r, g, b]),
            GrayscaleMode::Average => (r + g + b) / 3.0,
            GrayscaleMode::Max => r.max(g).max(b),
            GrayscaleMode::Min => r.min(g).min(b),
        }
    }
}
