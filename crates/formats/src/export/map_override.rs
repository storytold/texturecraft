use serde::{Deserialize, Serialize};

use super::FileFormat;

/// One preset map's own export settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapOverride {
    /// False leaves the map out (its file name is still reserved, so the others keep theirs).
    pub enabled: bool,
    /// Instead of the preset's and the export's file type.
    pub format: Option<FileFormat>,
    /// Instead of the preset's and the export's bit depth.
    pub bit_depth: Option<u8>,
    /// Instead of the export's dithering of 8-bit files.
    #[serde(default)]
    pub dither: Option<bool>,
}

impl Default for MapOverride {
    fn default() -> Self {
        MapOverride { enabled: true, format: None, bit_depth: None, dither: None }
    }
}
