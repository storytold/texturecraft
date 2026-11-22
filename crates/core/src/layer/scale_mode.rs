use serde::{Deserialize, Serialize};

/// What a fill's size is set in: repeats of its tile, or the tile's length on the mesh (stored as tiling either way)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ScaleMode {
    #[default]
    Tiling,
    PhysicalSize,
}

impl ScaleMode {
    pub const ALL: [ScaleMode; 2] = [ScaleMode::Tiling, ScaleMode::PhysicalSize];

    /// Command id
    pub fn id(self) -> &'static str {
        match self {
            ScaleMode::Tiling => "tiling",
            ScaleMode::PhysicalSize => "physicalSize",
        }
    }

    pub fn from_id(id: &str) -> Option<ScaleMode> {
        ScaleMode::ALL.into_iter().find(|mode| mode.id() == id)
    }
}
