use serde::{Deserialize, Serialize};

/// What a generator's output resolution is based on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResolutionMode {
    /// The context: the resolution of the UV tile it is computed for.
    Auto,
    /// The texture set's resolution.
    TextureSet,
    /// The UV tile's resolution.
    UvTile,
    /// A size of its own.
    Custom,
}

impl ResolutionMode {
    pub const ALL: [ResolutionMode; 4] = [ResolutionMode::Auto, ResolutionMode::TextureSet, ResolutionMode::UvTile, ResolutionMode::Custom];

    pub fn label(self) -> &'static str {
        match self {
            ResolutionMode::Auto => "Auto",
            ResolutionMode::TextureSet => "Texture set",
            ResolutionMode::UvTile => "UV tile",
            ResolutionMode::Custom => "Custom",
        }
    }

    pub fn parse(name: &str) -> Option<ResolutionMode> {
        let norm = |text: &str| text.to_ascii_lowercase().replace([' ', '_', '-'], "");
        ResolutionMode::ALL.into_iter().find(|mode| norm(&format!("{mode:?}")) == norm(name) || norm(mode.label()) == norm(name))
    }
}
