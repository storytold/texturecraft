use serde::{Deserialize, Serialize};

use super::{FileType, MapExport};

/// One texture set's export overrides (None: the global setting).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SetExport {
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub file_type: Option<FileType>,
    /// Texels per side.
    #[serde(default)]
    pub size: Option<u32>,
    /// Padding type and dilation width.
    #[serde(default)]
    pub padding: Option<(String, u32)>,
    /// The template `maps` belong to: they are dropped when the set's template changes.
    #[serde(default)]
    pub maps_template: String,
    /// Per output map (by index into the template's maps) settings.
    #[serde(default)]
    pub maps: std::collections::BTreeMap<usize, MapExport>,
}

impl SetExport {
    /// Whether anything is overridden (the set is marked "*").
    pub fn any(&self) -> bool {
        self.preset.is_some() || self.file_type.is_some() || self.size.is_some() || self.padding.is_some() || self.maps.values().any(|m| m.file_type.is_some())
    }

    /// Whether the set exports as the global settings say (no override, every map ticked).
    pub fn is_default(&self) -> bool {
        !self.any() && self.maps.values().all(|m| m.enabled)
    }
}
