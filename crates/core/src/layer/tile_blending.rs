use serde::{Deserialize, Serialize};

/// How a Tile Generator combines shapes that overlap
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TileBlending {
    #[default]
    Add,
    Max,
    /// Adds bright shapes and subtracts dark ones
    AddSub,
}

impl TileBlending {
    pub const ALL: [TileBlending; 3] = [TileBlending::Add, TileBlending::Max, TileBlending::AddSub];

    pub fn label(self) -> &'static str {
        match self {
            TileBlending::Add => "Add",
            TileBlending::Max => "Max",
            TileBlending::AddSub => "Add sub",
        }
    }
}
