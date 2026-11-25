use serde::{Deserialize, Serialize};

/// Which of a Tile Generator's input images each tile draws
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TileInputDistribution {
    /// A random input per tile
    #[default]
    Random,
    /// The inputs in turn, tile after tile
    ByNumber,
}

impl TileInputDistribution {
    pub const ALL: [TileInputDistribution; 2] = [TileInputDistribution::Random, TileInputDistribution::ByNumber];

    pub fn label(self) -> &'static str {
        match self {
            TileInputDistribution::Random => "Random",
            TileInputDistribution::ByNumber => "By pattern number",
        }
    }
}
