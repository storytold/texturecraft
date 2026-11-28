//! Where a texture set's UV padding takes its texels from

use serde::{Deserialize, Serialize};

/// UV padding mode: what fills the texels just outside a texture set's UV islands
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UvPaddingMode {
    /// Across seams (3D): past a UV seam, the texels continue the surface from the seam's other side
    Space3d,
    /// From island edges (2D): every texel repeats the nearest covered texel in the texture
    #[default]
    Space2d,
}

impl UvPaddingMode {
    pub const ALL: [UvPaddingMode; 2] = [UvPaddingMode::Space3d, UvPaddingMode::Space2d];

    /// Stable id for commands and inspect
    pub fn id(self) -> &'static str {
        match self {
            UvPaddingMode::Space3d => "3d",
            UvPaddingMode::Space2d => "2d",
        }
    }

    pub fn parse(id: &str) -> Option<UvPaddingMode> {
        UvPaddingMode::ALL.into_iter().find(|value| value.id() == id)
    }
}
