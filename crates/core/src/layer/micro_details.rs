use serde::{Deserialize, Serialize};

use super::micro_curvature_type::MicroCurvatureType;
use super::{GeneratorKind, LayerId, Source};
use crate::Channel;

/// A generator's micro details: a height or normal image adding fine curvature or occlusion to its mesh maps
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroDetails {
    /// Use Micro Details (Curvature and Ambient Occlusion; the wear kinds use them once Micro Height or Micro Normal is on)
    pub enabled: bool,
    /// Read the micro height image
    pub height: bool,
    /// Read the micro normal image
    pub normal: bool,
    pub curvature_type: MicroCurvatureType,
    /// How strongly the micro details add to the curvature (0..5 on Curvature, 0..1 on the wear kinds)
    pub curvature_intensity: f32,
    /// How steep the micro height reads ([`MicroDetails::HEIGHT_INTENSITY_RANGE`])
    pub height_intensity: f32,
    /// How far the micro height occludes (0..1)
    pub ao_radius: f32,
    /// How deep the micro height reads for occlusion (0..1)
    pub ao_depth: f32,
    /// The micro height image (None: an empty slot)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_source: Option<Source>,
    /// The micro normal image (None: an empty slot)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_source: Option<Source>,
    /// Saved as the micro detail before Micro Details (an anchor's height times an amount): a generator still on the earlier model adds it that way, see [`super::Generator::earlier_micro`]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub earlier: bool,
}

impl MicroDetails {
    pub const HEIGHT_INTENSITY_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;

    /// Height Details Intensity per unit of an anchor's micro detail amount (0..1): the earlier default amount, 0.5, reads at the default intensity
    pub const HEIGHT_INTENSITY_PER_AMOUNT: f32 = 2.0;

    /// The micro detail saved before Micro Details, anchor point `anchor`'s height at `amount` (0..1), as Micro Details on a `kind` generator (None for kinds without them): the anchor is the Micro Height, `amount` its intensity
    pub fn of_anchor(kind: GeneratorKind, anchor: LayerId, amount: f32) -> Option<MicroDetails> {
        let amount = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
        let source = Source::Anchor { anchor, channel: Channel::Height, ramp: Vec::new() };
        let micro = MicroDetails {
            enabled: true,
            height: true,
            height_intensity: amount * MicroDetails::HEIGHT_INTENSITY_PER_AMOUNT,
            height_source: Some(source),
            earlier: true,
            ..MicroDetails::defaults(kind)?
        };
        Some(micro.clamped(kind))
    }

    /// The reference app's defaults for `kind`, or None for kinds without micro details
    pub fn defaults(kind: GeneratorKind) -> Option<MicroDetails> {
        let shared = MicroDetails {
            enabled: false,
            height: true,
            normal: true,
            curvature_type: MicroCurvatureType::Sobel,
            curvature_intensity: 1.0,
            height_intensity: 1.0,
            ao_radius: 0.3,
            ao_depth: 0.05,
            height_source: None,
            normal_source: None,
            earlier: false,
        };
        match kind {
            GeneratorKind::Curvature | GeneratorKind::AmbientOcclusion => Some(shared),
            GeneratorKind::EdgeWear | GeneratorKind::Dirt | GeneratorKind::FiberGlassEdgeWear => {
                Some(MicroDetails { height: false, normal: false, curvature_intensity: 0.5, ..shared })
            }
            GeneratorKind::ThinAreas
            | GeneratorKind::Position
            | GeneratorKind::Direction
            | GeneratorKind::WorldSpaceNormals
            | GeneratorKind::ThreeDDistance
            | GeneratorKind::ThreeDLinearGradient
            | GeneratorKind::UvBorderDistance
            | GeneratorKind::GrayscaleConversion
            | GeneratorKind::DrippingRust
            | GeneratorKind::UvRandomColor
            | GeneratorKind::UvChecker => None,
        }
    }

    /// Whether a generator of `kind` has the Use Micro Details switch (the wear kinds have only Micro Height and Micro Normal)
    pub fn has_switch(kind: GeneratorKind) -> bool {
        matches!(kind, GeneratorKind::Curvature | GeneratorKind::AmbientOcclusion)
    }

    /// Whether a generator of `kind` adds micro curvature
    pub fn has_curvature(kind: GeneratorKind) -> bool {
        MicroDetails::defaults(kind).is_some() && kind != GeneratorKind::AmbientOcclusion
    }

    /// Whether a generator of `kind` adds micro occlusion
    pub fn has_occlusion(kind: GeneratorKind) -> bool {
        MicroDetails::defaults(kind).is_some() && kind != GeneratorKind::Curvature
    }

    /// The range of the curvature intensity on `kind`
    pub fn curvature_intensity_range(kind: GeneratorKind) -> std::ops::RangeInclusive<f32> {
        if kind == GeneratorKind::Curvature { 0.0..=5.0 } else { 0.0..=1.0 }
    }

    /// Whether the micro details change a generator of `kind`: switched on with a slot to read (an empty slot reads a flat black height, which some curvature types still lift)
    pub fn in_use(&self, kind: GeneratorKind) -> bool {
        (self.enabled || !MicroDetails::has_switch(kind)) && (self.height || self.normal)
    }

    /// The images the micro details read: their height, then their normal
    pub fn sources(&self) -> impl Iterator<Item = &Source> {
        self.height_source.iter().chain(self.normal_source.as_ref())
    }

    /// Every value brought into its range, non-finite ones to the defaults of `kind`
    pub fn clamped(self, kind: GeneratorKind) -> MicroDetails {
        let Some(defaults) = MicroDetails::defaults(kind) else { return self };
        let within = |value: f32, default: f32, range: std::ops::RangeInclusive<f32>| {
            if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default }
        };

        MicroDetails {
            curvature_intensity: within(self.curvature_intensity, defaults.curvature_intensity, MicroDetails::curvature_intensity_range(kind)),
            height_intensity: within(self.height_intensity, defaults.height_intensity, MicroDetails::HEIGHT_INTENSITY_RANGE),
            ao_radius: within(self.ao_radius, defaults.ao_radius, 0.0..=1.0),
            ao_depth: within(self.ao_depth, defaults.ao_depth, 0.0..=1.0),
            ..self
        }
    }
}
