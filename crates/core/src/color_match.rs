//! Color Match's settings: the colors moved so a source color lands on a target color

use serde::{Deserialize, Serialize};

use crate::{SourceColorMode, TargetColorMode, oklab};

/// A Color Match filter: every color moved, in OKLab, by the step from its source color to its target color
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorMatch {
    /// Source Color Mode: the color moved from
    pub source_mode: SourceColorMode,
    /// Source Color (linear), read in Source Color Mode Parameter
    pub source_color: [f32; 3],
    /// Target Color Mode: the color moved to
    pub target_mode: TargetColorMode,
    /// Target Color (linear), read in Target Color Mode Parameter
    pub target_color: [f32; 3],
    /// Custom Color Variation: the colors keep at most Hue, Chroma and Luma of their difference from the source
    pub custom_variation: bool,
    /// Hue (degrees): how far a hue may stay from the target's
    pub hue: f32,
    /// Chroma: how far a chroma may stay from the target's
    pub chroma: f32,
    /// Luma: how far a lightness may stay from the target's
    pub luma: f32,
}

impl ColorMatch {
    /// Mid gray (128 sRGB) in linear
    const GRAY: [f32; 3] = [0.215_860_5; 3];

    /// The most Hue keeps (degrees): two hues are at most half a turn apart
    pub const MAX_HUE_DEGREES: f32 = 180.0;
    /// The most Chroma keeps (OKLab chroma)
    pub const MAX_CHROMA: f32 = 1.0;
    /// The most Luma keeps (OKLab lightness, 0..1)
    pub const MAX_LUMA: f32 = 1.0;

    /// A new Color Match: the texture's mean moves to the target and every texel keeps all of its difference from the mean (Color Variation at its widest), so the layer keeps its pattern until a variation is lowered
    pub const LIBRARY: ColorMatch = ColorMatch {
        source_mode: SourceColorMode::Average,
        source_color: Self::GRAY,
        target_mode: TargetColorMode::Parameter,
        target_color: Self::GRAY,
        custom_variation: true,
        hue: Self::MAX_HUE_DEGREES,
        chroma: Self::MAX_CHROMA,
        luma: Self::MAX_LUMA,
    };

    /// Linear `color` moved by the step from `source` to `target`; without Custom Color Variation it keeps all of its difference from the source, with it at most Hue, Chroma and Luma of it (all 0: the target color itself). Hostile values give the color back
    pub fn apply(&self, color: [f32; 3], source: [f32; 3], target: [f32; 3]) -> [f32; 3] {
        let lab = |color: [f32; 3]| oklab::from_linear(color.map(|component| component.max(0.0)));
        let [lightness, a, b] = lab(color);
        let [source_lightness, source_a, source_b] = lab(source);
        let [target_lightness, target_a, target_b] = lab(target);

        let moved = if self.custom_variation {
            self.move_polar(polar([lightness, a, b]), polar([source_lightness, source_a, source_b]), polar([target_lightness, target_a, target_b]))
        } else {
            [target_lightness + lightness - source_lightness, target_a + a - source_a, target_b + b - source_b]
        };

        matched_linear(color, moved)
    }

    #[inline]
    pub(super) fn move_polar(
        &self,
        [lightness, chroma, hue]: [f32; 3],
        [source_lightness, source_chroma, source_hue]: [f32; 3],
        [target_lightness, target_chroma, target_hue]: [f32; 3],
    ) -> [f32; 3] {
        let limit = |value: f32, max: f32| if value.is_finite() { value.clamp(0.0, max) } else { 0.0 };
        let keep = |difference: f32, max: f32| difference.clamp(-max, max);
        let turn = (hue - source_hue + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let out_lightness = target_lightness + keep(lightness - source_lightness, limit(self.luma, Self::MAX_LUMA));
        let out_chroma = (target_chroma + keep(chroma - source_chroma, limit(self.chroma, Self::MAX_CHROMA))).max(0.0);
        let out_hue = target_hue + keep(turn, limit(self.hue, Self::MAX_HUE_DEGREES).to_radians());
        [out_lightness, out_chroma * out_hue.cos(), out_chroma * out_hue.sin()]
    }
}

pub(super) fn polar([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    [lightness, a.hypot(b), b.atan2(a)]
}

pub(super) fn matched_linear(original: [f32; 3], lab: [f32; 3]) -> [f32; 3] {
    let out = oklab::to_linear(lab);
    if out.iter().all(|value| value.is_finite()) { out.map(|value| value.clamp(0.0, 1.0)) } else { original }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn the_source_color_lands_on_the_target_and_variation_is_kept_as_asked() {
        let red = [0.6, 0.1, 0.05];
        let blue = [0.05, 0.1, 0.6];
        let lighter_red = [0.8, 0.2, 0.1];
        let lightness = |color: [f32; 3]| oklab::from_linear(color)[0];

        // Without Custom Color Variation every difference from the source is kept
        let keep_all = ColorMatch { custom_variation: false, ..ColorMatch::LIBRARY };
        let moved = keep_all.apply(lighter_red, red, blue);
        assert!(near(keep_all.apply(red, red, blue), blue), "the source itself becomes the target");
        assert!(lightness(moved) > lightness(blue) + 0.05, "a lighter texel stays lighter than the target");
        assert!(moved[2] > moved[0], "and turns blue");

        // Custom Color Variation at 0: every color is the target
        let flat = ColorMatch { luma: 0.0, chroma: 0.0, hue: 0.0, ..ColorMatch::LIBRARY };
        assert!(near(flat.apply(lighter_red, red, blue), blue));

        // A new Color Match's widest limits keep the lightness difference again
        let library = ColorMatch::LIBRARY;
        assert!(lightness(library.apply(lighter_red, red, blue)) > lightness(blue) + 0.05, "a new Color Match keeps the variation");

        // Luma alone: the lightness difference kept up to it
        let luma_only = ColorMatch { luma: 0.02, ..flat };
        let limited = lightness(luma_only.apply(lighter_red, red, blue)) - lightness(blue);
        assert!((limited - 0.02).abs() < 1e-3, "lightness kept up to Luma: {limited}");

        let hostile = ColorMatch { hue: f32::NAN, chroma: f32::INFINITY, luma: -1.0, ..flat };
        assert!(hostile.apply([f32::NAN, 0.5, 0.5], red, blue).iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }
}
