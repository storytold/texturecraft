use serde::{Deserialize, Serialize};

use super::Source;
use super::generator_grayscale::GeneratorGrayscale;

/// The Grayscale Conversion generator's controls: an image turned gray, tiled and rotated across the UV square
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrayscaleConversionParameters {
    /// How the image's color becomes gray
    pub grayscale: GeneratorGrayscale,
    /// Shifts the gray darker (below 0.5) or brighter (above 0.5)
    pub balance: f32,
    /// Narrows the gray around its middle (0..1; 1 a hard step)
    pub contrast: f32,
    /// How many times the image repeats across the UV square ([`GrayscaleConversionParameters::TILE_RANGE`])
    pub tile: f32,
    /// Degrees the image turns about the UV square's centre (0..360)
    pub rotation: f32,
    /// Turn in quarter turns only, so the image keeps tiling seamlessly
    pub safe_rotation: bool,
    /// The image (None: an empty slot, read black)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

impl GrayscaleConversionParameters {
    pub const TILE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=16.0;

    /// Every value brought into its range, non-finite ones to the defaults
    pub fn clamped(self) -> GrayscaleConversionParameters {
        let defaults = GrayscaleConversionParameters::default();
        let within = |value: f32, default: f32, range: std::ops::RangeInclusive<f32>| {
            if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default }
        };

        GrayscaleConversionParameters {
            balance: within(self.balance, defaults.balance, 0.0..=1.0),
            contrast: within(self.contrast, defaults.contrast, 0.0..=1.0),
            tile: within(self.tile, defaults.tile, GrayscaleConversionParameters::TILE_RANGE).round(),
            rotation: within(self.rotation, defaults.rotation, 0.0..=360.0),
            ..self
        }
    }
}

impl Default for GrayscaleConversionParameters {
    /// The reference app's defaults
    fn default() -> GrayscaleConversionParameters {
        GrayscaleConversionParameters {
            grayscale: GeneratorGrayscale::Desaturation,
            balance: 0.5,
            contrast: 0.0,
            tile: 1.0,
            rotation: 0.0,
            safe_rotation: true,
            source: None,
        }
    }
}
