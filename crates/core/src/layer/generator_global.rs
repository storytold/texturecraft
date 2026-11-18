use serde::{Deserialize, Serialize};

/// Adjustments every generator applies to its finished value: a blur, then balance and contrast.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratorGlobal {
    /// Blur of the generated value (0..1 of [`GeneratorGlobal::MAX_BLUR`]).
    pub blur: f32,
    /// Shifts the value darker (below 0.5) or brighter (above 0.5); 0.5 leaves it unchanged.
    pub balance: f32,
    /// Narrows the ramp around its middle (0 unchanged, 1 a hard step).
    pub contrast: f32,
}

impl GeneratorGlobal {
    /// The widest blur, as a radius in texels at [`GeneratorGlobal::BLUR_RESOLUTION`].
    pub const MAX_BLUR: f32 = 64.0;
    /// Narrower than this, the balanced range is a hard step.
    pub const MIN_RANGE: f32 = 1e-6;
    /// The texels per side the blur radius is measured at (2K).
    pub const BLUR_RESOLUTION: f32 = 2048.0;

    pub fn is_neutral(&self) -> bool {
        *self == GeneratorGlobal::default()
    }

    /// The blur as a radius in texels at [`GeneratorGlobal::BLUR_RESOLUTION`].
    pub fn blur_radius(&self) -> f32 {
        self.blur.clamp(0.0, 1.0) * GeneratorGlobal::MAX_BLUR
    }

    /// The blur as a radius in texels at `resolution` texels per side.
    pub fn blur_radius_at(&self, resolution: u32) -> f32 {
        self.blur_radius() * resolution as f32 / GeneratorGlobal::BLUR_RESOLUTION
    }

    /// `value` after balance and contrast: balance picks the input range stretched to 0..1, contrast narrows it around its middle.
    pub fn apply(&self, value: f32) -> f32 {
        if self.balance == 0.5 && self.contrast == 0.0 {
            return value;
        }
        let balance = self.balance.clamp(0.0, 1.0);
        let (low, high) = ((1.0 - 2.0 * balance).max(0.0), (2.0 - 2.0 * balance).min(1.0));
        let middle = (low + high) * 0.5;
        let keep = 1.0 - self.contrast.clamp(0.0, 1.0);
        let (low, high) = (middle - (middle - low) * keep, middle + (high - middle) * keep);
        if high - low < GeneratorGlobal::MIN_RANGE {
            return if value >= middle { 1.0 } else { 0.0 };
        }
        ((value - low) / (high - low)).clamp(0.0, 1.0)
    }
}

impl Default for GeneratorGlobal {
    fn default() -> Self {
        GeneratorGlobal { blur: 0.0, balance: 0.5, contrast: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_settings_leave_the_value_unchanged() {
        let global = GeneratorGlobal::default();
        for value in [0.0, 0.1, 0.5, 0.93, 1.0] {
            assert_eq!(global.apply(value), value);
        }
    }

    #[test]
    fn balance_darkens_below_the_middle_and_brightens_above() {
        let darker = GeneratorGlobal { balance: 0.25, ..GeneratorGlobal::default() };
        assert_eq!(darker.apply(0.4), 0.0, "the lower half goes black");
        assert!((darker.apply(0.75) - 0.5).abs() < 1e-6, "the upper half stretches over the whole range");

        let brighter = GeneratorGlobal { balance: 0.75, ..GeneratorGlobal::default() };
        assert!((brighter.apply(0.25) - 0.5).abs() < 1e-6);
        assert_eq!(brighter.apply(0.6), 1.0, "the upper half goes white");

        assert_eq!(GeneratorGlobal { balance: 0.0, ..GeneratorGlobal::default() }.apply(0.99), 0.0);
        assert_eq!(GeneratorGlobal { balance: 1.0, ..GeneratorGlobal::default() }.apply(0.01), 1.0);
    }

    #[test]
    fn full_contrast_is_a_step_at_the_middle_of_the_balanced_range() {
        let step = GeneratorGlobal { contrast: 1.0, ..GeneratorGlobal::default() };
        assert_eq!(step.apply(0.49), 0.0);
        assert_eq!(step.apply(0.51), 1.0);

        let half = GeneratorGlobal { contrast: 0.5, ..GeneratorGlobal::default() };
        assert_eq!(half.apply(0.25), 0.0);
        assert!((half.apply(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(half.apply(0.75), 1.0);
    }
}
