use serde::{Deserialize, Serialize};

/// The shape a Tile Generator draws in each tile
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TileShape {
    /// One of the pattern's input images
    Image,
    Square,
    Disc,
    Paraboloid,
    Bell,
    Gaussian,
    Thorn,
    Pyramid,
    #[default]
    Brick,
    Gradation,
    Waves,
    HalfBell,
    RidgedBell,
    Crescent,
    Capsule,
    Cone,
}

impl TileShape {
    /// In the reference app's menu order
    pub const ALL: [TileShape; 16] = [
        TileShape::Image,
        TileShape::Square,
        TileShape::Disc,
        TileShape::Paraboloid,
        TileShape::Bell,
        TileShape::Gaussian,
        TileShape::Thorn,
        TileShape::Pyramid,
        TileShape::Brick,
        TileShape::Gradation,
        TileShape::Waves,
        TileShape::HalfBell,
        TileShape::RidgedBell,
        TileShape::Crescent,
        TileShape::Capsule,
        TileShape::Cone,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TileShape::Image => "Custom image input",
            TileShape::Square => "Square",
            TileShape::Disc => "Disc",
            TileShape::Paraboloid => "Paraboloid",
            TileShape::Bell => "Bell",
            TileShape::Gaussian => "Gaussian",
            TileShape::Thorn => "Thorn",
            TileShape::Pyramid => "Pyramid",
            TileShape::Brick => "Brick",
            TileShape::Gradation => "Gradation",
            TileShape::Waves => "Waves",
            TileShape::HalfBell => "Half bell",
            TileShape::RidgedBell => "Ridged bell",
            TileShape::Crescent => "Crescent",
            TileShape::Capsule => "Capsule",
            TileShape::Cone => "Cone",
        }
    }
}
