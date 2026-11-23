//! Gradient's Input Transformation: how a texel becomes the position it reads on the ramp

use serde::{Deserialize, Serialize};

use crate::grayscale_mode::GrayscaleMode;
use crate::{ChannelKind, finite_unit, lum, per_unit};

/// Keep the transition finite just below full contrast
const MIN_STEEPNESS: f32 = 1e-3;

/// The balance that leaves the gray as it is
pub(crate) const NEUTRAL_BALANCE: f32 = 0.5;

/// The gray, blurred, balanced, contrasted and maybe inverted value a gradient looks up
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GradientInput {
    /// How a color becomes gray (masks and scalar channels are gray already)
    pub mode: GrayscaleMode,
    /// Blurs the gray before the lookup, in texels at 2K (0..16)
    pub blur: f32,
    /// Shifts the gray: above 0.5 lighter, below darker
    pub balance: f32,
    /// Steepens the gray around its middle: 1 is a hard step
    pub contrast: f32,
    pub invert: bool,
}

impl Default for GradientInput {
    /// What gradients read in projects saved before the Input Transformation: luma, untouched
    fn default() -> Self {
        GradientInput { mode: GrayscaleMode::Luma, blur: 0.0, balance: NEUTRAL_BALANCE, contrast: 0.0, invert: false }
    }
}

impl GradientInput {
    /// The library's starting transformation: the average of the components, untouched
    pub const LIBRARY: GradientInput = GradientInput { mode: GrayscaleMode::Average, blur: 0.0, balance: NEUTRAL_BALANCE, contrast: 0.0, invert: false };

    /// Whether it reads gradients as projects saved before it did (skipped when saving)
    pub fn is_unchanged(&self) -> bool {
        *self == GradientInput::default()
    }

    /// The ramp position of gray `x`
    pub fn shape(&self, x: f32) -> f32 {
        // Preserve the exact input when the transformation is neutral
        if self.balance == NEUTRAL_BALANCE && self.contrast == 0.0 && !self.invert {
            return x;
        }

        shape_gray(x, self.balance, self.contrast, self.invert)
    }

    /// Apply a gradient to a channel using its ramp sampler
    pub fn map(&self, kind: ChannelKind, value: [f32; 3], sample: impl Fn(f32) -> Option<[f32; 3]>) -> Option<[f32; 3]> {
        if kind == ChannelKind::Color {
            return sample(self.shape(self.mode.gray(value)));
        }

        // Preserve the signed-channel mapping and leave normals alone
        Some(per_unit(kind, value, |gray| sample(self.shape(gray)).map_or(gray, lum)))
    }
}

/// Gray `x` balanced (0.5 keeps it), contrasted around mid gray (0 keeps it, 1 is a step at 0.5) and maybe inverted, in 0..1
pub fn shape_gray(x: f32, balance: f32, contrast: f32, invert: bool) -> f32 {
    let balanced = finite_unit(x) + finite_unit(balance) - NEUTRAL_BALANCE;
    let contrast = finite_unit(contrast);
    let shaped = if contrast == 1.0 {
        f32::from(balanced >= NEUTRAL_BALANCE)
    } else {
        let steep = (1.0 - contrast).max(MIN_STEEPNESS);
        finite_unit((balanced - NEUTRAL_BALANCE) / steep + NEUTRAL_BALANCE)
    };

    if invert { 1.0 - shaped } else { shaped }
}

/// A ramp stop's place and color
type Stop = (f32, [f32; 3]);

pub(crate) fn ramp_stops<'a>(colors: &'a [[f32; 3]], positions: &'a [f32]) -> impl ExactSizeIterator<Item = Stop> + 'a {
    let last = colors.len().saturating_sub(1);
    colors.iter().copied().enumerate().map(move |(index, color)| {
        let even_position = || if last == 0 { 0.0 } else { index as f32 / last as f32 };
        let position = positions.get(index).copied().filter(|position| position.is_finite()).unwrap_or_else(even_position);
        let position = if position <= 0.0 { 0.0 } else { position.min(1.0) };
        (position, color)
    })
}

/// The color at `t` on a ramp of `colors` placed at `positions` (each 0..1, evenly spaced where missing), blended in linear or sRGB
pub fn ramp(colors: &[[f32; 3]], positions: &[f32], linear: bool, t: f32) -> Option<[f32; 3]> {
    let stops = ramp_stops(colors, positions);
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };

    // Keep the last stop at or before the sample and the first stop after it
    let (mut before, mut after): (Option<Stop>, Option<Stop>) = (None, None);
    for (position, color) in stops {
        if position <= t {
            if before.is_none_or(|(so_far, _)| position.total_cmp(&so_far).is_ge()) {
                before = Some((position, color));
            }
        } else if after.is_none_or(|(so_far, _)| position.total_cmp(&so_far).is_lt()) {
            after = Some((position, color));
        }
    }
    let ((before_place, before_color), (after_place, after_color)) = match (before, after) {
        (Some(before), Some(after)) => (before, after),
        (Some(only), None) | (None, Some(only)) => (only, only),
        (None, None) => return None,
    };

    let encode = |color: [f32; 3]| if linear { color } else { color.map(linear_to_srgb) };
    Some(interpolate((before_place, encode(before_color)), (after_place, encode(after_color)), linear, t))
}

pub(crate) fn interpolate((before_place, before_color): Stop, (after_place, after_color): Stop, linear: bool, t: f32) -> [f32; 3] {
    let share = if after_place > before_place { ((t - before_place) / (after_place - before_place)).clamp(0.0, 1.0) } else { 0.0 };
    let color = std::array::from_fn(|component| before_color[component] + (after_color[component] - before_color[component]) * share);
    if linear { color } else { color.map(srgb_to_linear) }
}

// A copy of texturecraft_texel's: texel and core are both layer 0, so core cannot use it

/// sRGB transfer of a linear component
pub(crate) fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Linear value of an sRGB component
pub(crate) fn srgb_to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ramp_reads_its_stops_in_place_order_whatever_their_list_order() {
        let (black, gray, white) = ([0.0; 3], [0.5; 3], [1.0; 3]);
        let colors = [white, black, gray];
        let positions = [1.0, 0.0, 0.5];

        let quarter = ramp(&colors, &positions, true, 0.25);
        let ends = (ramp(&colors, &positions, true, 0.0), ramp(&colors, &positions, true, 1.0));
        let past_one_stop = ramp(&[gray, white], &[0.5, 0.5], true, 0.25);

        assert_eq!(ramp(&[black, white], &[0.0, -0.0], true, 0.0), Some(white));
        assert_eq!(quarter, Some([0.25; 3]));
        assert_eq!(ends, (Some(black), Some(white)));
        assert_eq!(past_one_stop, Some(gray), "before the first stop, its color");
        assert_eq!(ramp(&[], &[], true, 0.5), None);
    }

    #[test]
    fn full_contrast_separates_neighbors_of_mid_gray() {
        let below = shape_gray(0.49975, NEUTRAL_BALANCE, 1.0, false);
        let above = shape_gray(0.50025, NEUTRAL_BALANCE, 1.0, false);

        assert_eq!((below, above), (0.0, 1.0));
        assert_eq!(shape_gray(0.50025, NEUTRAL_BALANCE, 1.0, true), below);
    }

    #[test]
    fn an_untouched_input_passes_every_gray_on_bit_for_bit() {
        const SAMPLES: u16 = 1000;
        let untouched = [GradientInput::default(), GradientInput::LIBRARY];
        let grays = (0..=SAMPLES).map(|step| f32::from(step) / f32::from(SAMPLES)).chain([-0.25, 1.5]);

        for input in untouched {
            for gray in grays.clone() {
                assert_eq!(input.shape(gray).to_bits(), gray.to_bits(), "{input:?} at {gray}");
            }
        }
    }
}
