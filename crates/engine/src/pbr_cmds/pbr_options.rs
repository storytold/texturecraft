use std::collections::BTreeMap;
use texturecraft_core::{NormalFormat, Projection};
use texturecraft_formats::pbr_maps::MapKind;

/// How to import.
#[derive(Clone, Debug, Default)]
pub struct PbrOptions {
    pub name: Option<String>,
    /// Forces every normal map's convention; `None` reads it from the file name (default OpenGL).
    pub normal_format: Option<NormalFormat>,
    pub projection: Projection,
    /// Files chosen by the caller where several name one map.
    pub choose: BTreeMap<MapKind, String>,
}
