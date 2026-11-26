//! PBR Validate (Metallic Roughness): base colors outside the physically plausible ranges shown in red

use serde::{Deserialize, Serialize};

use crate::ValidationMode;
use crate::blend::lum;
use crate::gradient_input::linear_to_srgb;

/// A PBR Validate filter: which texels it checks and the ranges it checks them against
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PbrValidate {
    pub mode: ValidationMode,
    /// Albedo Dark Range Threshold: the darkest valid non-metal base color, in sRGB steps of 255 (30 or 50)
    pub albedo_dark: u8,
    /// Metal Reflectance Range: the dimmest valid metal base color, in percent of sRGB white (60 or 70)
    pub metal_low: u8,
}

impl PbrValidate {
    /// A new PBR Validate: both kinds of texel, the tolerant ranges
    pub const LIBRARY: PbrValidate = PbrValidate { mode: ValidationMode::Combined, albedo_dark: 30, metal_low: 60 };

    /// Albedo Dark Range Threshold's choices
    pub const ALBEDO_DARK: [u8; 2] = [50, 30];

    /// Metal Reflectance Range's choices
    pub const METAL_LOW: [u8; 2] = [70, 60];

    /// The brightest valid non-metal base color, in sRGB steps of 255
    const ALBEDO_BRIGHT: f32 = 240.0;

    /// Metal from this value up counts as metal
    const METAL_THRESHOLD: f32 = 0.5;

    /// Metal Reflectance Range's choices are percent of sRGB white
    const PERCENT: f32 = 100.0;

    /// Base Color Range's choices are sRGB steps of 255
    const SRGB_STEPS: f32 = 255.0;

    /// The color of a texel outside its range
    pub const INVALID: [f32; 3] = [1.0, 0.0, 0.0];

    /// Straight linear base color `color` of a texel with metal `metal`: its own color when valid or not checked, red when not
    pub fn check(&self, color: [f32; 3], metal: f32) -> [f32; 3] {
        let srgb = linear_to_srgb(lum(color).clamp(0.0, 1.0));
        let valid = if metal >= Self::METAL_THRESHOLD {
            !self.mode.checks_metal() || srgb >= f32::from(self.metal_low) / Self::PERCENT
        } else {
            !self.mode.checks_albedo() || (f32::from(self.albedo_dark) / Self::SRGB_STEPS..=Self::ALBEDO_BRIGHT / Self::SRGB_STEPS).contains(&srgb)
        };
        if valid { color } else { Self::INVALID }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gradient_input::srgb_to_linear;

    #[test]
    fn colors_outside_their_range_turn_red() {
        let gray = |srgb: f32| [srgb_to_linear(srgb / 255.0); 3];
        let filter = PbrValidate::LIBRARY;
        assert_eq!(filter.check(gray(40.0), 0.0), gray(40.0), "above 30 sRGB");
        assert_eq!(filter.check(gray(20.0), 0.0), PbrValidate::INVALID, "too dark");
        assert_eq!(filter.check(gray(250.0), 0.0), PbrValidate::INVALID, "too bright");
        assert_eq!(PbrValidate { albedo_dark: 50, ..filter }.check(gray(40.0), 0.0), PbrValidate::INVALID);
        assert_eq!(filter.check(gray(140.0), 1.0), PbrValidate::INVALID, "a dim metal");
        assert_eq!(filter.check(gray(170.0), 1.0), gray(170.0), "60% reflective");
        assert_eq!(PbrValidate { metal_low: 70, ..filter }.check(gray(170.0), 1.0), PbrValidate::INVALID);
        let albedo = PbrValidate { mode: ValidationMode::Albedo, ..filter };
        assert_eq!(albedo.check(gray(140.0), 1.0), gray(140.0), "Albedo leaves metal unchecked");
        let metal = PbrValidate { mode: ValidationMode::Metal, ..filter };
        assert_eq!(metal.check(gray(20.0), 0.0), gray(20.0), "Metal leaves non-metal unchecked");
    }
}
