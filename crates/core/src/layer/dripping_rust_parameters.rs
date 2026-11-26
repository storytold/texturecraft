use serde::{Deserialize, Serialize};

/// The Dripping Rust generator's controls: rust in the occluded areas, running down in drips
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrippingRustParameters {
    /// How far the rust spreads: 0 none, 1 everywhere
    pub spreading: f32,
    /// How hard the rust's edge is (0..1; 1 a hard step)
    pub contrast: f32,
    /// Softens the rust before it drips (0..1)
    pub spreading_smoothness: f32,
    /// How far the drips run (0..1)
    pub drips_intensity: f32,
    /// Softens the drips (0..1)
    pub drips_smoothness: f32,
    /// How many steps each drip is drawn with ([`DrippingRustParameters::MAX_SAMPLES`] at most)
    pub drips_samples: u32,
    /// The axis of the mesh's bounds the drips run down: 0 X, 1 Y, 2 Z
    pub axis: u8,
}

impl DrippingRustParameters {
    pub const MAX_SAMPLES: u32 = 32;

    /// Every value brought into its range, non-finite ones to the defaults
    pub fn clamped(self) -> DrippingRustParameters {
        let defaults = DrippingRustParameters::default();
        let unit = |value: f32, default: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { default };

        DrippingRustParameters {
            spreading: unit(self.spreading, defaults.spreading),
            contrast: unit(self.contrast, defaults.contrast),
            spreading_smoothness: unit(self.spreading_smoothness, defaults.spreading_smoothness),
            drips_intensity: unit(self.drips_intensity, defaults.drips_intensity),
            drips_smoothness: unit(self.drips_smoothness, defaults.drips_smoothness),
            drips_samples: self.drips_samples.min(DrippingRustParameters::MAX_SAMPLES),
            axis: self.axis.min(2),
        }
    }
}

impl Default for DrippingRustParameters {
    /// The reference app's defaults
    fn default() -> DrippingRustParameters {
        DrippingRustParameters {
            spreading: 0.5,
            contrast: 0.5,
            spreading_smoothness: 0.0,
            drips_intensity: 0.1,
            drips_smoothness: 0.05,
            drips_samples: 32,
            axis: 1,
        }
    }
}
