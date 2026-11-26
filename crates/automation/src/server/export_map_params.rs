use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Overrides for one output map.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportMapParams {
    /// Index of the map in the preset.
    pub map: usize,
    /// False leaves this map out of the export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// File format override, such as png, pgm or tiff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Bit depth override (8, 16 or 32, depending on the format).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_depth: Option<u8>,
    /// Override dithering for this map's 8-bit outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dithering: Option<bool>,
}
