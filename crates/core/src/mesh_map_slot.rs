//! The mesh map an imported image file was resampled into

use serde::{Deserialize, Serialize};

use crate::BakedMap;

/// A mesh map of a texture set, and the UV tile one image of it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MeshMapSlot {
    /// Index of the texture set.
    pub texture_set: usize,
    pub map: BakedMap,
    /// The UV tile the image covers (1001 on a set without UV tiles).
    pub tile: u32,
}
