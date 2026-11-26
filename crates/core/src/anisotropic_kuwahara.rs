//! Anisotropic Kuwahara's settings: a painterly smoothing that follows the direction of the strokes

use serde::{Deserialize, Serialize};

/// An Anisotropic Kuwahara filter: each texel takes the mean of the calmest sector of an ellipse turned along the local direction
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnisotropicKuwahara {
    /// Extract Direction Custom Input: the direction comes from the filter's Custom Input instead of the channel
    pub custom_input: bool,
    /// Texels at 2K the ellipse reaches (0..10)
    pub radius: f32,
    /// How much neighbouring sectors overlap (0 = hard sector edges)
    pub smoothness: f32,
    /// How strongly the calmest sector wins over the others (0.25..1)
    pub sharpness: f32,
    /// How far around a texel its direction is averaged, as a share of the radius
    pub tensor_smoothness: f32,
    /// How much the ellipse stretches along the direction where the direction is clear (0 = a circle)
    pub anisotropy: f32,
    /// Degrees the ellipse turns from the extracted direction
    pub angle: f32,
}

impl AnisotropicKuwahara {
    /// A new Anisotropic Kuwahara
    pub const LIBRARY: AnisotropicKuwahara =
        AnisotropicKuwahara { custom_input: false, radius: 5.0, smoothness: 0.25, sharpness: 0.25, tensor_smoothness: 0.1, anisotropy: 1.0, angle: 0.0 };

    /// The largest Radius, in texels at 2K
    pub const MAX_RADIUS: f32 = 10.0;

    /// The least Sharpness
    pub const MIN_SHARPNESS: f32 = 0.25;

    /// Sectors the ellipse is split into
    pub const SECTORS: usize = 8;

    /// The sector weighing exponent at a Sharpness of 0 and of 1 (Sharpness stops at [`Self::MIN_SHARPNESS`], so the least it weighs with is 5.5)
    const SHARPNESS_EXPONENT_MIN: f32 = 2.0;
    const SHARPNESS_EXPONENT_MAX: f32 = 16.0;

    /// The sample falloff exponent at no Smoothness (1 at full Smoothness)
    const SECTOR_EXPONENT_MAX: f32 = 16.0;

    /// How far around a texel its direction is read, as a share of the radius (Tensor Smoothness, 0..1)
    pub fn tensor_reach(&self) -> f32 {
        if self.tensor_smoothness.is_finite() { self.tensor_smoothness.clamp(0.0, 1.0) } else { Self::LIBRARY.tensor_smoothness }
    }

    /// The exponent a sector's spread is weighed with: higher picks the calmest sector more sharply
    pub fn sharpness_exponent(&self) -> f32 {
        let sharpness = if self.sharpness.is_finite() { self.sharpness.clamp(Self::MIN_SHARPNESS, 1.0) } else { Self::LIBRARY.sharpness };
        Self::SHARPNESS_EXPONENT_MIN + sharpness * (Self::SHARPNESS_EXPONENT_MAX - Self::SHARPNESS_EXPONENT_MIN)
    }

    /// How sharply a sample's weight falls off away from a sector's middle: smoother sectors overlap more
    pub fn sector_exponent(&self) -> f32 {
        let smoothness = if self.smoothness.is_finite() { self.smoothness.clamp(0.0, 1.0) } else { Self::LIBRARY.smoothness };
        1.0 + (1.0 - smoothness) * (Self::SECTOR_EXPONENT_MAX - 1.0)
    }

    /// The ellipse's semi-axes (along, across the direction) for `radius` texels where the direction is `clear` (0..1)
    pub fn axes(&self, radius: f32, clear: f32) -> (f32, f32) {
        let stretch = if self.anisotropy.is_finite() { self.anisotropy.clamp(0.0, 1.0) } else { Self::LIBRARY.anisotropy } * clear.clamp(0.0, 1.0);
        (radius * (1.0 + stretch), radius / (1.0 + stretch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clear_direction_stretches_the_ellipse() {
        let filter = AnisotropicKuwahara::LIBRARY;
        assert_eq!(filter.axes(4.0, 0.0), (4.0, 4.0));
        assert_eq!(filter.axes(4.0, 1.0), (8.0, 2.0));
        assert_eq!(AnisotropicKuwahara { anisotropy: 0.0, ..filter }.axes(4.0, 1.0), (4.0, 4.0));
        assert!(AnisotropicKuwahara { sharpness: 1.0, ..filter }.sharpness_exponent() > filter.sharpness_exponent());
        assert!(AnisotropicKuwahara { smoothness: 1.0, ..filter }.sector_exponent() < filter.sector_exponent());
    }

    #[test]
    fn sharpness_weighs_on_a_line_from_its_exponent_at_0_to_its_exponent_at_1() {
        let at = |sharpness: f32| AnisotropicKuwahara { sharpness, ..AnisotropicKuwahara::LIBRARY }.sharpness_exponent();

        assert_eq!(at(AnisotropicKuwahara::MIN_SHARPNESS), 5.5, "the least Sharpness, a quarter of the way along");
        assert_eq!(at(0.0), 5.5, "below the least, the least");
        assert_eq!(at(1.0), 16.0, "the most Sharpness weighs with the most exponent");
    }
}
