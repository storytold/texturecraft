use serde::{Deserialize, Serialize};

/// How a Tile Generator sizes its shapes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TileSizeMode {
    /// Each shape fills its tile minus the interstice (the gap between shapes)
    #[default]
    NormalInterstice,
    /// Each shape takes a size relative to its tile, whatever the gap
    NormalSize,
    /// Each shape is the largest square its tile holds; the size settings are not read
    KeepRatio,
    /// A size relative to the whole pattern, whatever the tile count
    Absolute,
    /// A size in pixels of a 2048 pixel output, whatever the output's resolution
    Pixel,
}

impl TileSizeMode {
    pub const ALL: [TileSizeMode; 5] =
        [TileSizeMode::NormalInterstice, TileSizeMode::NormalSize, TileSizeMode::KeepRatio, TileSizeMode::Absolute, TileSizeMode::Pixel];

    pub fn label(self) -> &'static str {
        match self {
            TileSizeMode::NormalInterstice => "Normal - interstice",
            TileSizeMode::NormalSize => "Normal - size",
            TileSizeMode::KeepRatio => "Keep ratio",
            TileSizeMode::Absolute => "Absolute",
            TileSizeMode::Pixel => "Pixel",
        }
    }
}
