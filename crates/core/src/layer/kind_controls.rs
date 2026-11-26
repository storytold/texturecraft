use serde::{Deserialize, Serialize};

use super::{
    BorderDistanceParameters, DistanceParameters, DrippingRustParameters, GeneratorKind, GrayscaleConversionParameters, LinearGradientParameters,
    UvCheckerParameters, UvRandomColorParameters,
};

/// The controls of a kind that has only the reference's own controls, no earlier model: one variant per kind
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KindControls {
    Distance(DistanceParameters),
    LinearGradient(LinearGradientParameters),
    BorderDistance(BorderDistanceParameters),
    Grayscale(GrayscaleConversionParameters),
    DrippingRust(DrippingRustParameters),
    UvRandomColor(UvRandomColorParameters),
    UvChecker(UvCheckerParameters),
}

impl KindControls {
    /// The reference app's defaults for `kind`, or None for kinds with an earlier model or section, wear or light controls
    pub fn defaults(kind: GeneratorKind) -> Option<KindControls> {
        match kind {
            GeneratorKind::ThreeDDistance => Some(KindControls::Distance(DistanceParameters::default())),
            GeneratorKind::ThreeDLinearGradient => Some(KindControls::LinearGradient(LinearGradientParameters::default())),
            GeneratorKind::UvBorderDistance => Some(KindControls::BorderDistance(BorderDistanceParameters::default())),
            GeneratorKind::GrayscaleConversion => Some(KindControls::Grayscale(GrayscaleConversionParameters::default())),
            GeneratorKind::DrippingRust => Some(KindControls::DrippingRust(DrippingRustParameters::default())),
            GeneratorKind::UvRandomColor => Some(KindControls::UvRandomColor(UvRandomColorParameters::default())),
            GeneratorKind::UvChecker => Some(KindControls::UvChecker(UvCheckerParameters::default())),
            GeneratorKind::EdgeWear
            | GeneratorKind::Dirt
            | GeneratorKind::ThinAreas
            | GeneratorKind::Curvature
            | GeneratorKind::AmbientOcclusion
            | GeneratorKind::Position
            | GeneratorKind::Direction
            | GeneratorKind::FiberGlassEdgeWear
            | GeneratorKind::WorldSpaceNormals => None,
        }
    }

    /// Every value brought into its range, non-finite ones to the defaults (saved files are not trusted)
    pub fn clamped(self) -> KindControls {
        match self {
            KindControls::Distance(distance) => KindControls::Distance(distance.clamped()),
            KindControls::LinearGradient(gradient) => KindControls::LinearGradient(gradient.clamped()),
            KindControls::BorderDistance(border) => KindControls::BorderDistance(border.clamped()),
            KindControls::Grayscale(grayscale) => KindControls::Grayscale(grayscale.clamped()),
            KindControls::DrippingRust(rust) => KindControls::DrippingRust(rust.clamped()),
            KindControls::UvRandomColor(random) => KindControls::UvRandomColor(random.clamped()),
            KindControls::UvChecker(checker) => KindControls::UvChecker(checker.clamped()),
        }
    }

    /// The kind these controls belong to
    pub fn kind(&self) -> GeneratorKind {
        match self {
            KindControls::Distance(_) => GeneratorKind::ThreeDDistance,
            KindControls::LinearGradient(_) => GeneratorKind::ThreeDLinearGradient,
            KindControls::BorderDistance(_) => GeneratorKind::UvBorderDistance,
            KindControls::Grayscale(_) => GeneratorKind::GrayscaleConversion,
            KindControls::DrippingRust(_) => GeneratorKind::DrippingRust,
            KindControls::UvRandomColor(_) => GeneratorKind::UvRandomColor,
            KindControls::UvChecker(_) => GeneratorKind::UvChecker,
        }
    }
}
