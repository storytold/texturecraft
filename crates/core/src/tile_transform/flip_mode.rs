use serde::{Deserialize, Serialize};

/// Which way the Transform filter turns its result over
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlipMode {
    #[default]
    None,
    /// Left and right swap
    Horizontally,
    /// Top and bottom swap
    Vertically,
    HorizontallyAndVertically,
}

impl FlipMode {
    /// Every mode, in the order a list shows them
    pub const ALL: [FlipMode; 4] = [FlipMode::None, FlipMode::Horizontally, FlipMode::Vertically, FlipMode::HorizontallyAndVertically];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            FlipMode::None => "None",
            FlipMode::Horizontally => "Horizontally",
            FlipMode::Vertically => "Vertically",
            FlipMode::HorizontallyAndVertically => "Horizontally & Vertically",
        }
    }

    /// The id `effect.set {flipMode}` takes
    pub fn id(self) -> &'static str {
        match self {
            FlipMode::None => "none",
            FlipMode::Horizontally => "horizontally",
            FlipMode::Vertically => "vertically",
            FlipMode::HorizontallyAndVertically => "horizontallyAndVertically",
        }
    }

    /// Whether it swaps across (left and right) and down (top and bottom)
    pub fn sides(self) -> [bool; 2] {
        match self {
            FlipMode::None => [false, false],
            FlipMode::Horizontally => [true, false],
            FlipMode::Vertically => [false, true],
            FlipMode::HorizontallyAndVertically => [true, true],
        }
    }
}
