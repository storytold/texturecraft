use serde::{Deserialize, Serialize};

/// The Log panel's channels, one tab each
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogChannel {
    /// Everything that is not about assets: projects, exports, bakes, the UI
    #[default]
    General,
    /// Loading and importing resources, templates, presets and materials
    Assets,
}

impl LogChannel {
    pub const ALL: [LogChannel; 2] = [LogChannel::General, LogChannel::Assets];

    pub fn name(self) -> &'static str {
        match self {
            LogChannel::General => "GENERAL",
            LogChannel::Assets => "ASSETS",
        }
    }
}
