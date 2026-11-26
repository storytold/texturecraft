use serde::{Deserialize, Serialize};

/// Whether a moved texture repeats across its UV tile or leaves empty where it no longer reaches
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepeatMode {
    /// Past the tile's edge reads empty
    None,
    /// The tile repeats both ways, so the result stays seamless
    #[default]
    HorizontallyAndVertically,
}

impl RepeatMode {
    /// Every mode, in the order a list shows them
    pub const ALL: [RepeatMode; 2] = [RepeatMode::None, RepeatMode::HorizontallyAndVertically];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            RepeatMode::None => "None",
            RepeatMode::HorizontallyAndVertically => "Horizontally & Vertically",
        }
    }

    /// The id `effect.set {repeatMode}` takes
    pub fn id(self) -> &'static str {
        match self {
            RepeatMode::None => "none",
            RepeatMode::HorizontallyAndVertically => "horizontallyAndVertically",
        }
    }
}
