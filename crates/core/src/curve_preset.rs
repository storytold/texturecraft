//! The shapes Gradient Curve remaps values through

use serde::{Deserialize, Serialize};

use crate::layer::smoothstep;

/// Stairs climbs in this many flat steps
const STAIRS_STEPS: f32 = 4.0;

/// Triangular rises and falls this many times
const TRIANGULAR_TEETH: f32 = 2.0;

/// A remap curve from 0..1 onto 0..1
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CurvePreset {
    /// A quarter circle: rises fast, then flattens
    #[default]
    Round,
    /// A quarter circle the other way: flat, then rises fast
    Pinch,
    /// Four flat steps
    Stairs,
    EaseOut,
    Ease,
    EaseIn,
    /// Up to the middle and back down, rounded at the top
    PeakOut,
    /// Up to the middle and back down in straight lines
    Peak,
    /// Up to the middle and back down, pointed at the top
    PeakIn,
    /// Half a sine wave
    Bump,
    /// Two humps
    DoubleBump,
    /// Two straight teeth
    Triangular,
}

impl CurvePreset {
    /// Every preset, in the order the Preset list shows them
    pub const ALL: [CurvePreset; 12] = [
        CurvePreset::Round,
        CurvePreset::Pinch,
        CurvePreset::Stairs,
        CurvePreset::EaseOut,
        CurvePreset::Ease,
        CurvePreset::EaseIn,
        CurvePreset::PeakOut,
        CurvePreset::Peak,
        CurvePreset::PeakIn,
        CurvePreset::Bump,
        CurvePreset::DoubleBump,
        CurvePreset::Triangular,
    ];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            CurvePreset::Round => "Round",
            CurvePreset::Pinch => "Pinch",
            CurvePreset::Stairs => "Stairs",
            CurvePreset::EaseOut => "Ease Out",
            CurvePreset::Ease => "Ease",
            CurvePreset::EaseIn => "Ease In",
            CurvePreset::PeakOut => "Peak Out",
            CurvePreset::Peak => "Peak",
            CurvePreset::PeakIn => "Peak In",
            CurvePreset::Bump => "Bump",
            CurvePreset::DoubleBump => "Double Bump",
            CurvePreset::Triangular => "Triangular",
        }
    }

    /// The id `effect.set {preset}` takes
    pub fn id(self) -> &'static str {
        match self {
            CurvePreset::Round => "round",
            CurvePreset::Pinch => "pinch",
            CurvePreset::Stairs => "stairs",
            CurvePreset::EaseOut => "easeOut",
            CurvePreset::Ease => "ease",
            CurvePreset::EaseIn => "easeIn",
            CurvePreset::PeakOut => "peakOut",
            CurvePreset::Peak => "peak",
            CurvePreset::PeakIn => "peakIn",
            CurvePreset::Bump => "bump",
            CurvePreset::DoubleBump => "doubleBump",
            CurvePreset::Triangular => "triangular",
        }
    }

    /// The curve at `t` (clamped to 0..1; a non-finite `t` reads 0)
    pub fn at(self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let peak = 1.0 - (2.0 * t - 1.0).abs();
        let y = match self {
            CurvePreset::Round => (1.0 - (1.0 - t) * (1.0 - t)).sqrt(),
            CurvePreset::Pinch => 1.0 - (1.0 - t * t).sqrt(),
            CurvePreset::Stairs => ((t * STAIRS_STEPS).floor() / (STAIRS_STEPS - 1.0)).min(1.0),
            CurvePreset::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            CurvePreset::Ease => smoothstep(0.0, 1.0, t),
            CurvePreset::EaseIn => t * t,
            CurvePreset::PeakOut => peak.sqrt(),
            CurvePreset::Peak => peak,
            CurvePreset::PeakIn => peak * peak,
            CurvePreset::Bump => (std::f32::consts::PI * t).sin(),
            CurvePreset::DoubleBump => (2.0 * std::f32::consts::PI * t).sin().abs(),
            CurvePreset::Triangular => 1.0 - ((2.0 * TRIANGULAR_TEETH * t) % 2.0 - 1.0).abs(),
        };
        y.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples across 0..1 that every curve is read at
    const SAMPLES: usize = 64;

    #[test]
    fn every_preset_stays_in_range_and_reads_a_non_finite_t_as_0() {
        for preset in CurvePreset::ALL {
            let values: Vec<f32> = (0..=SAMPLES).map(|i| preset.at(i as f32 / SAMPLES as f32)).collect();

            assert!(values.iter().all(|value| (0.0..=1.0).contains(value)), "{}", preset.id());
            assert_eq!(preset.at(f32::NAN), preset.at(0.0), "{}", preset.id());
            assert_eq!(preset.at(f32::INFINITY), preset.at(0.0), "{}", preset.id());
        }
    }

    #[test]
    fn stairs_has_four_levels_and_triangular_two_teeth() {
        let mut levels: Vec<f32> = (0..=SAMPLES).map(|i| CurvePreset::Stairs.at(i as f32 / SAMPLES as f32)).collect();
        levels.dedup();

        assert_eq!(levels, [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]);

        let triangular = |t: f32| CurvePreset::Triangular.at(t);
        assert_eq!([triangular(0.0), triangular(0.25), triangular(0.5), triangular(0.75), triangular(1.0)], [0.0, 1.0, 0.0, 1.0, 0.0]);
    }
}
