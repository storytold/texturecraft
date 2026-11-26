use serde::{Deserialize, Serialize};

use super::Source;
use super::generator_grayscale::GeneratorGrayscale;
use super::texture_blend::TextureBlend;

/// A texture a generator blends over its signal (the reference's Use Texture block): read as gray, adjusted, then blended before the global controls
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureLayer {
    /// Use Texture: off keeps the settings but leaves the signal as it is
    pub enabled: bool,
    /// What the texture reads (None: an empty slot, uniform black)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// How much of the blend shows (0..1)
    pub opacity: f32,
    pub invert: bool,
    pub grayscale: GeneratorGrayscale,
    pub blend: TextureBlend,
    /// Repeats across the UV square, about its centre ([`TextureLayer::SCALE_RANGE`])
    pub scale: f32,
    /// Narrows the texture around its middle (above 0) or flattens it towards gray (below 0) (−1..1)
    pub contrast: f32,
    /// Lifts the texture towards white (above 0) or black (below 0) (−1..1)
    pub brightness: f32,
    /// Projected from the world's three axes rather than laid out in UV
    pub triplanar: bool,
    /// How sharply the three projections meet ([`TextureLayer::TRIPLANAR_CONTRAST_RANGE`])
    pub triplanar_contrast: f32,
}

impl Default for TextureLayer {
    /// The reference app's defaults: off, and at opacity 0 even once turned on
    fn default() -> Self {
        TextureLayer {
            enabled: false,
            source: None,
            opacity: 0.0,
            invert: false,
            grayscale: GeneratorGrayscale::default(),
            blend: TextureBlend::default(),
            scale: 1.0,
            contrast: 0.0,
            brightness: 0.0,
            triplanar: false,
            triplanar_contrast: 0.5,
        }
    }
}

impl TextureLayer {
    pub const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.01..=10.0;
    pub const TRIPLANAR_CONTRAST_RANGE: std::ops::RangeInclusive<f32> = 0.001..=1.0;

    /// Whether the texture changes the signal: turned on with some opacity (an empty slot blends black)
    pub fn in_use(&self) -> bool {
        self.enabled && self.opacity > 0.0
    }

    /// Every value brought into its range, non-finite ones to the defaults
    pub fn clamped(self) -> TextureLayer {
        let defaults = TextureLayer::default();
        let within = |value: f32, default: f32, range: std::ops::RangeInclusive<f32>| {
            if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default }
        };

        TextureLayer {
            opacity: within(self.opacity, defaults.opacity, 0.0..=1.0),
            scale: within(self.scale, defaults.scale, TextureLayer::SCALE_RANGE),
            contrast: within(self.contrast, defaults.contrast, -1.0..=1.0),
            brightness: within(self.brightness, defaults.brightness, -1.0..=1.0),
            triplanar_contrast: within(self.triplanar_contrast, defaults.triplanar_contrast, TextureLayer::TRIPLANAR_CONTRAST_RANGE),
            ..self
        }
    }
}
