use serde::{Deserialize, Serialize};

/// How a fill reads its image sources between texels
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Filtering {
    /// Bilinear on the image level that matches the texel's footprint: smooth when the image is shrunk
    #[default]
    BilinearHq,
    /// Bilinear on the full image: sharp, but may shimmer where the image is shrunk
    BilinearSharp,
    /// The nearest image texel
    Nearest,
}

impl Filtering {
    pub const ALL: [Filtering; 3] = [Filtering::BilinearHq, Filtering::BilinearSharp, Filtering::Nearest];

    /// Command id
    pub fn id(self) -> &'static str {
        match self {
            Filtering::BilinearHq => "bilinearHq",
            Filtering::BilinearSharp => "bilinearSharp",
            Filtering::Nearest => "nearest",
        }
    }

    pub fn from_id(id: &str) -> Option<Filtering> {
        Filtering::ALL.into_iter().find(|f| f.id() == id)
    }
}
