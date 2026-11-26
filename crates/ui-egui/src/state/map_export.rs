use serde::{Deserialize, Serialize};

use super::FileType;

/// One output map's export settings on a texture set's page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapExport {
    /// False leaves the map out of the export.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub file_type: Option<FileType>,
}

fn enabled_by_default() -> bool {
    true
}

impl Default for MapExport {
    fn default() -> Self {
        MapExport { enabled: true, file_type: None }
    }
}
