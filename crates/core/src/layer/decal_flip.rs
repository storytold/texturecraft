use serde::{Deserialize, Serialize};

/// How the mirrored decal's image is turned over
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecalFlip {
    /// Only the reflection: the copy is the decal's mirror image
    #[default]
    None,
    /// The image's left and right swapped
    U,
    /// The image's top and bottom swapped
    V,
}
