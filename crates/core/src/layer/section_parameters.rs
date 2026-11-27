use serde::{Deserialize, Serialize};

use super::GeneratorKind;
use super::axis_blend::AxisBlend;
use super::axis_layer::AxisLayer;

/// The section controls of the Curvature, Ambient Occlusion, Position and World Space Normals generators, applied before the global ones
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionParameters {
    pub invert: bool,
    /// Blur of the signal ([`SectionParameters::BLUR_RANGE`]; Ambient Occlusion and World Space Normals)
    pub blur: f32,
    /// Shifts the signal darker (below 0.5) or brighter (above 0.5); 0.5 leaves it unchanged (not Curvature)
    pub balance: f32,
    /// Narrows the signal around its middle (0 unchanged, 1 a hard step)
    pub contrast: f32,
    /// Lifts the signal towards white (above 0) or black (below 0) ([`SectionParameters::BRIGHTNESS_RANGE`]; not Ambient Occlusion)
    pub brightness: f32,
    /// Right to left, top to bottom and front to back (Position and World Space Normals)
    pub axes: [AxisLayer; 3],
}

impl SectionParameters {
    pub const BLUR_RANGE: std::ops::RangeInclusive<f32> = 0.0..=16.0;
    pub const BRIGHTNESS_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;
    /// Index in [`SectionParameters::axes`] of each axis
    pub const RIGHT_TO_LEFT: usize = 0;
    pub const TOP_TO_BOTTOM: usize = 1;
    pub const FRONT_TO_BACK: usize = 2;

    /// The reference app's defaults for `kind`, or None for kinds without section controls
    pub fn defaults(kind: GeneratorKind) -> Option<SectionParameters> {
        let axis = |weight: f32| AxisLayer { invert: false, blend: AxisBlend::Add, weight };
        let defaults = SectionParameters { invert: false, blur: 0.0, balance: 0.5, contrast: 0.0, brightness: 0.0, axes: [axis(0.0), axis(1.0), axis(0.0)] };
        match kind {
            GeneratorKind::Curvature | GeneratorKind::AmbientOcclusion | GeneratorKind::Position | GeneratorKind::WorldSpaceNormals => Some(defaults),
            GeneratorKind::EdgeWear
            | GeneratorKind::Dirt
            | GeneratorKind::ThinAreas
            | GeneratorKind::Direction
            | GeneratorKind::FiberGlassEdgeWear
            | GeneratorKind::ThreeDDistance
            | GeneratorKind::ThreeDLinearGradient
            | GeneratorKind::UvBorderDistance
            | GeneratorKind::GrayscaleConversion
            | GeneratorKind::DrippingRust
            | GeneratorKind::UvRandomColor
            | GeneratorKind::UvChecker => None,
        }
    }

    /// Whether a generator of `kind` has the section blur
    pub fn has_blur(kind: GeneratorKind) -> bool {
        matches!(kind, GeneratorKind::AmbientOcclusion | GeneratorKind::WorldSpaceNormals)
    }

    /// Whether a generator of `kind` has the section balance
    pub fn has_balance(kind: GeneratorKind) -> bool {
        matches!(kind, GeneratorKind::AmbientOcclusion | GeneratorKind::Position | GeneratorKind::WorldSpaceNormals)
    }

    /// Whether a generator of `kind` has the section brightness
    pub fn has_brightness(kind: GeneratorKind) -> bool {
        matches!(kind, GeneratorKind::Curvature | GeneratorKind::Position | GeneratorKind::WorldSpaceNormals)
    }

    /// Whether a generator of `kind` combines the three axes of a map
    pub fn has_axes(kind: GeneratorKind) -> bool {
        matches!(kind, GeneratorKind::Position | GeneratorKind::WorldSpaceNormals)
    }

    /// Every value brought into its range, non-finite ones to the defaults of `kind`
    pub fn clamped(self, kind: GeneratorKind) -> SectionParameters {
        let defaults = SectionParameters::defaults(kind).unwrap_or(self);
        let within = |value: f32, default: f32, range: std::ops::RangeInclusive<f32>| {
            if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default }
        };
        let axes = std::array::from_fn(|index| {
            let (axis, default) = (self.axes[index], defaults.axes[index]);
            AxisLayer { weight: within(axis.weight, default.weight, 0.0..=1.0), ..axis }
        });

        SectionParameters {
            invert: self.invert,
            blur: within(self.blur, defaults.blur, SectionParameters::BLUR_RANGE),
            balance: within(self.balance, defaults.balance, 0.0..=1.0),
            contrast: within(self.contrast, defaults.contrast, 0.0..=1.0),
            brightness: within(self.brightness, defaults.brightness, SectionParameters::BRIGHTNESS_RANGE),
            axes,
        }
    }
}
