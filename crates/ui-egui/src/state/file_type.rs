use serde::{Deserialize, Serialize};

/// A file type with its bit depth (an export override).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileType {
    /// The listed name ("png", "targa", ...).
    pub format: String,
    pub bit_depth: u8,
    /// "8 bits + dithering".
    #[serde(default)]
    pub dithering: bool,
}
