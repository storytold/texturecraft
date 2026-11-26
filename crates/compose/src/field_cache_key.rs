use texturecraft_core::BlockScales;

use crate::sampled_map::SampledMap;

/// What a field was built from: geometry, atlas layout, baked-map identities and generator settings
#[derive(Clone, PartialEq)]
pub(crate) struct FieldCacheKey {
    pub(crate) geometry: u64,
    pub(crate) block: (u32, u32),
    pub(crate) scales: BlockScales,
    pub(crate) maps: Vec<SampledMap>,
    pub(crate) settings: Vec<u32>,
}
