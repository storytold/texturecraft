use serde::{Deserialize, Serialize};

/// Whether a UV-placed fill repeats past its 0..1 tile, across, down, both or neither
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UvWrap {
    #[default]
    Repeat,
    RepeatHorizontally,
    RepeatVertically,
    /// One tile only: outside it the fill does not cover
    None,
}

impl UvWrap {
    pub const ALL: [UvWrap; 4] = [UvWrap::None, UvWrap::RepeatHorizontally, UvWrap::RepeatVertically, UvWrap::Repeat];

    /// Command id
    pub fn id(self) -> &'static str {
        match self {
            UvWrap::Repeat => "repeat",
            UvWrap::RepeatHorizontally => "repeatHorizontally",
            UvWrap::RepeatVertically => "repeatVertically",
            UvWrap::None => "none",
        }
    }

    pub fn from_id(id: &str) -> Option<UvWrap> {
        UvWrap::ALL.into_iter().find(|w| w.id() == id)
    }

    /// Whether the fill repeats across (u) and down (v)
    pub fn repeats(self) -> (bool, bool) {
        match self {
            UvWrap::Repeat => (true, true),
            UvWrap::RepeatHorizontally => (true, false),
            UvWrap::RepeatVertically => (false, true),
            UvWrap::None => (false, false),
        }
    }
}
