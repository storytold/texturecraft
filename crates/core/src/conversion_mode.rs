//! How Grayscale Conversion turns a color into gray

use serde::{Deserialize, Serialize};

use crate::GrayscaleMode;

/// The gray Grayscale Conversion reads from a color: a [`GrayscaleMode`], or its own weights per component
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversionMode {
    /// Halfway between the lightest and darkest component
    Desaturation,
    /// Red, Green, Blue and Alpha times their weights, summed
    #[default]
    ChannelsWeights,
    /// The components' mean
    Average,
    /// The lightest component
    Max,
    /// The darkest component
    Min,
}

impl ConversionMode {
    /// Every mode, in the order a list shows them
    pub const ALL: [ConversionMode; 5] =
        [ConversionMode::Desaturation, ConversionMode::ChannelsWeights, ConversionMode::Average, ConversionMode::Max, ConversionMode::Min];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            ConversionMode::ChannelsWeights => "Channels Weights",
            other => other.mode().map_or("", GrayscaleMode::label),
        }
    }

    /// The id `effect.set {mode}` takes
    pub fn id(self) -> &'static str {
        match self {
            ConversionMode::ChannelsWeights => "channelsWeights",
            other => other.mode().map_or("", GrayscaleMode::id),
        }
    }

    /// The gray of straight color `rgb` with coverage `alpha`, `weights` for Red, Green, Blue and Alpha
    pub fn gray(self, rgb: [f32; 3], alpha: f32, weights: [f32; 4]) -> f32 {
        match self.mode() {
            Some(mode) => mode.gray(rgb),
            None => rgb[0] * weights[0] + rgb[1] * weights[1] + rgb[2] * weights[2] + alpha * weights[3],
        }
    }

    /// The gradient's grayscale mode this one matches (None for Channels Weights)
    fn mode(self) -> Option<GrayscaleMode> {
        match self {
            ConversionMode::Desaturation => Some(GrayscaleMode::Desaturation),
            ConversionMode::ChannelsWeights => None,
            ConversionMode::Average => Some(GrayscaleMode::Average),
            ConversionMode::Max => Some(GrayscaleMode::Max),
            ConversionMode::Min => Some(GrayscaleMode::Min),
        }
    }
}
