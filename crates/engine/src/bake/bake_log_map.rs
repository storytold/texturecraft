use texturecraft_core::BakedMap;

use super::BakeMapState;

/// One row of the Baking Log's process list: a map of a texture set in the latest bake
#[derive(Clone, Debug, PartialEq)]
pub struct BakeLogMap {
    pub texture_set: usize,
    pub set_name: String,
    pub map: BakedMap,
    pub state: BakeMapState,
}
