use serde::{Deserialize, Serialize};

use super::GeneratorKind;

/// The controls of the wear generators (Metal Edge Wear, Fiber Glass Edge Wear, Dirt) as the reference app lays them out; each kind reads its own
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WearParameters {
    /// How far the wear spreads (0..1)
    pub level: f32,
    /// How hard the wear's edge is (0 soft, 1 a hard step)
    pub contrast: f32,
    /// How much the grunge breaks the wear up (0..1)
    pub grunge_amount: f32,
    /// The grunge's size: repeats across the UV square ([`WearParameters::GRUNGE_SCALE_RANGE`])
    pub grunge_scale: f32,
    /// The grunge projected from the world's three axes rather than laid out in UV
    pub triplanar: bool,
    /// How sharply the three projections meet (0..1)
    pub triplanar_contrast: f32,
    /// How far the worn edges spread over rounded edges ([`WearParameters::EDGES_SMOOTHNESS_RANGE`]; edge wear kinds)
    pub edges_smoothness: f32,
    /// How much occluded areas are kept clean (0..1; edge wear kinds)
    pub ao_masking: f32,
    /// How much the edges count against the grunge (0..1; edge wear kinds)
    pub curvature_weight: f32,
    /// How much convex edges are kept clean (0..1; Dirt)
    pub edges_masking: f32,
    /// The grunge read from the generator's custom grunge image instead of the kind's own pattern
    #[serde(default)]
    pub custom_grunge: bool,
}

impl WearParameters {
    pub const GRUNGE_SCALE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=16.0;
    pub const EDGES_SMOOTHNESS_RANGE: std::ops::RangeInclusive<f32> = 0.0..=16.0;

    /// The reference app's defaults for `kind`, or None for kinds without wear controls
    pub fn defaults(kind: GeneratorKind) -> Option<WearParameters> {
        let shared = WearParameters {
            level: 0.75,
            contrast: 0.5,
            grunge_amount: 0.3,
            grunge_scale: 4.0,
            triplanar: false,
            triplanar_contrast: 0.5,
            edges_smoothness: 1.0,
            ao_masking: 0.5,
            curvature_weight: 1.0,
            edges_masking: 0.5,
            custom_grunge: false,
        };
        match kind {
            GeneratorKind::EdgeWear | GeneratorKind::Dirt => Some(shared),
            GeneratorKind::FiberGlassEdgeWear => Some(WearParameters { curvature_weight: 0.25, ..shared }),
            GeneratorKind::ThinAreas
            | GeneratorKind::Curvature
            | GeneratorKind::AmbientOcclusion
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

    /// Every value brought into its range, non-finite ones to the defaults of `kind`
    pub fn clamped(self, kind: GeneratorKind) -> WearParameters {
        let defaults = WearParameters::defaults(kind).unwrap_or(self);
        let unit = |value: f32, default: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { default };
        let within = |value: f32, default: f32, range: &std::ops::RangeInclusive<f32>| {
            if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default }
        };

        WearParameters {
            level: unit(self.level, defaults.level),
            contrast: unit(self.contrast, defaults.contrast),
            grunge_amount: unit(self.grunge_amount, defaults.grunge_amount),
            grunge_scale: within(self.grunge_scale, defaults.grunge_scale, &WearParameters::GRUNGE_SCALE_RANGE),
            triplanar: self.triplanar,
            triplanar_contrast: unit(self.triplanar_contrast, defaults.triplanar_contrast),
            edges_smoothness: within(self.edges_smoothness, defaults.edges_smoothness, &WearParameters::EDGES_SMOOTHNESS_RANGE),
            ao_masking: unit(self.ao_masking, defaults.ao_masking),
            curvature_weight: unit(self.curvature_weight, defaults.curvature_weight),
            edges_masking: unit(self.edges_masking, defaults.edges_masking),
            custom_grunge: self.custom_grunge,
        }
    }
}
