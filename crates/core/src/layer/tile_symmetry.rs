use serde::{Deserialize, Serialize};

/// The mirrors a Tile Generator's random symmetry picks from
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TileSymmetry {
    #[default]
    Both,
    Horizontal,
    Vertical,
}

impl TileSymmetry {
    pub const ALL: [TileSymmetry; 3] = [TileSymmetry::Both, TileSymmetry::Horizontal, TileSymmetry::Vertical];

    pub fn label(self) -> &'static str {
        match self {
            TileSymmetry::Both => "Horizontal + vertical",
            TileSymmetry::Horizontal => "Horizontal",
            TileSymmetry::Vertical => "Vertical",
        }
    }
}
