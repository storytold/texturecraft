//! An image a filter reads besides the stack it filters (its image inputs), as a source on the filter's effect

use crate::{Filter, Source, SourceColorMode, TargetColorMode, WarpSourceMode};

/// One image input of a filter: the key its source is stored under on the effect, its name and the gray it reads until a source is set
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageInput {
    pub id: &'static str,
    pub label: &'static str,
    /// The gray the input reads while no source is set (what the filter did before it had the input)
    pub default: f32,
    /// Added by a mode of the filter (shown under that mode), not one of its Image inputs
    pub by_mode: bool,
}

impl ImageInput {
    /// Bevel Smooth's and Directional Distance's Distance Map: scales the distance per texel
    pub const DISTANCE_MAP: ImageInput = ImageInput { id: "distanceMap", label: "Distance Map", default: 1.0, by_mode: false };

    /// Warp's and Blur Slope's Custom Noise (Source Type Custom Noise): the source its texels move or read along; mid gray moves nothing
    pub const CUSTOM_NOISE: ImageInput = ImageInput { id: "customNoise", label: "Custom Noise", default: 0.5, by_mode: true };

    /// The Custom Input of Grayscale Conversion (the color it reads instead of the channel) and Anisotropic Kuwahara (its direction's gray); mid gray until set
    pub const CUSTOM_INPUT: ImageInput = ImageInput { id: "customInput", label: "Custom Input", default: 0.5, by_mode: true };

    /// Color Match's Input (Source Color Mode Input): the color each texel moves from; mid gray until set
    pub const SOURCE_INPUT: ImageInput = ImageInput { id: "input", label: "Input", default: 0.5, by_mode: true };

    /// Color Match's Image Input (Target Color Mode Image Input): the color each texel moves to; mid gray until set
    pub const TARGET_INPUT: ImageInput = ImageInput { id: "imageInput", label: "Image Input", default: 0.5, by_mode: true };

    /// The image inputs `filter` reads now (a mode's input only while that mode is on)
    pub fn of(filter: &Filter) -> Vec<ImageInput> {
        Self::all(filter).iter().copied().filter(|input| input.is_on(filter)).collect()
    }

    /// The input of `filter` stored under `id`, also while its mode is off
    pub fn named(filter: &Filter, id: &str) -> Option<ImageInput> {
        Self::all(filter).iter().find(|input| input.id == id).copied()
    }

    /// Every image input `filter` has, its modes' inputs also while those modes are off
    fn all(filter: &Filter) -> &'static [ImageInput] {
        match filter {
            Filter::BevelSmooth(_) | Filter::DirectionalDistance(_) => &[Self::DISTANCE_MAP],
            Filter::SourceWarp(_) | Filter::SourceSlopeBlur(_) => &[Self::CUSTOM_NOISE],
            Filter::GrayscaleConversion(_) | Filter::AnisotropicKuwahara(_) => &[Self::CUSTOM_INPUT],
            Filter::ColorMatch(_) => &[Self::SOURCE_INPUT, Self::TARGET_INPUT],
            _ => &[],
        }
    }

    /// Whether `filter` reads this input now: an input a mode adds only while that mode is on
    fn is_on(&self, filter: &Filter) -> bool {
        match filter {
            Filter::SourceWarp(warp) => warp.source.mode == WarpSourceMode::CustomNoise,
            Filter::SourceSlopeBlur(slope) => slope.source.mode == WarpSourceMode::CustomNoise,
            Filter::GrayscaleConversion(conversion) => conversion.custom_input,
            Filter::AnisotropicKuwahara(kuwahara) => kuwahara.custom_input,
            Filter::ColorMatch(matcher) if *self == Self::SOURCE_INPUT => matcher.source_mode == SourceColorMode::Input,
            Filter::ColorMatch(matcher) => matcher.target_mode == TargetColorMode::ImageInput,
            _ => true,
        }
    }

    /// The source the input reads while none is set
    pub fn default_source(&self) -> Source {
        Source::Uniform([self.default; 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BevelSmooth, SourceWarp, WarpSource};

    #[test]
    fn a_warp_has_its_custom_noise_only_in_that_source_mode() {
        let warp = SourceWarp::LIBRARY;
        assert!(ImageInput::of(&Filter::SourceWarp(warp)).is_empty());

        let custom = SourceWarp { source: WarpSource { mode: WarpSourceMode::CustomNoise, ..warp.source }, ..warp };
        assert_eq!(ImageInput::of(&Filter::SourceWarp(custom)), vec![ImageInput::CUSTOM_NOISE]);
        assert_eq!(ImageInput::named(&Filter::SourceWarp(warp), "customNoise"), Some(ImageInput::CUSTOM_NOISE), "kept while the mode is off");
        assert_eq!(ImageInput::of(&Filter::BevelSmooth(BevelSmooth::LIBRARY)), vec![ImageInput::DISTANCE_MAP]);
        assert_eq!(ImageInput::named(&Filter::BevelSmooth(BevelSmooth::LIBRARY), "customNoise"), None);
        assert_eq!(ImageInput::DISTANCE_MAP.default_source(), Source::Uniform([1.0; 3]));
    }
}
