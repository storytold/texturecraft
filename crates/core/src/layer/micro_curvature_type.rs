use serde::{Deserialize, Serialize};

/// How a generator's micro details turn the micro height into curvature
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MicroCurvatureType {
    /// The slope of the height, stronger than Sobel and lifting the whole surface slightly
    Standard,
    /// The slope of the height
    #[default]
    Sobel,
    /// The height against blurred copies of itself, softer and wider
    Smooth,
}

impl MicroCurvatureType {
    pub const ALL: [MicroCurvatureType; 3] = [MicroCurvatureType::Standard, MicroCurvatureType::Sobel, MicroCurvatureType::Smooth];

    pub fn label(self) -> &'static str {
        match self {
            MicroCurvatureType::Standard => "Standard",
            MicroCurvatureType::Sobel => "Sobel",
            MicroCurvatureType::Smooth => "Smooth",
        }
    }

    /// The type named `name` (its label, any case)
    pub fn parse(name: &str) -> Option<MicroCurvatureType> {
        MicroCurvatureType::ALL.into_iter().find(|kind| kind.label().eq_ignore_ascii_case(name))
    }
}
