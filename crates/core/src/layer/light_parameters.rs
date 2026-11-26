use serde::{Deserialize, Serialize};

use super::GeneratorKind;

/// The Light generator's controls: where the light comes from, how its highlight spreads and how it fades across the mesh
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightParameters {
    /// Degrees around the up axis (0 from -X, 90 from +Z)
    pub horizontal: f32,
    /// Degrees above the horizon (90 from straight above)
    pub vertical: f32,
    /// How narrow the lit area is ([`LightParameters::GLOSSINESS_RANGE`]; 0.5 a plain diffuse falloff)
    pub glossiness: f32,
    /// How much the lit area is pushed to white (0..1; 1 every lit texel white)
    pub highlight_level: f32,
    /// How much the side away from the light darkens (0..1)
    pub attenuation: f32,
}

impl LightParameters {
    pub const ANGLE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=360.0;
    pub const GLOSSINESS_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.999;

    /// The reference app's defaults for `kind`, or None for kinds other than Light
    pub fn defaults(kind: GeneratorKind) -> Option<LightParameters> {
        (kind == GeneratorKind::Direction).then_some(LightParameters {
            horizontal: 90.0,
            vertical: 45.0,
            glossiness: 0.5,
            highlight_level: 0.0,
            attenuation: 0.25,
        })
    }

    /// Every value brought into its range, non-finite ones to the defaults
    pub fn clamped(self) -> LightParameters {
        let defaults = LightParameters::defaults(GeneratorKind::Direction).unwrap_or(self);
        let within = |value: f32, default: f32, range: std::ops::RangeInclusive<f32>| {
            if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default }
        };

        LightParameters {
            horizontal: within(self.horizontal, defaults.horizontal, LightParameters::ANGLE_RANGE),
            vertical: within(self.vertical, defaults.vertical, LightParameters::ANGLE_RANGE),
            glossiness: within(self.glossiness, defaults.glossiness, LightParameters::GLOSSINESS_RANGE),
            highlight_level: within(self.highlight_level, defaults.highlight_level, 0.0..=1.0),
            attenuation: within(self.attenuation, defaults.attenuation, 0.0..=1.0),
        }
    }
}
