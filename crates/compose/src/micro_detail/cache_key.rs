use std::sync::Weak;

use super::slot_key::SlotKey;

/// What a micro map was built from: the mesh map giving its layout, its slots, its settings, atlas dimensions and UV tile block
#[derive(Clone)]
pub(super) struct CacheKey {
    pub(super) map: Weak<Vec<f32>>,
    pub(super) height: Option<SlotKey>,
    pub(super) normal: Option<SlotKey>,
    pub(super) settings: Vec<u32>,
    pub(super) size: (u32, u32),
    pub(super) block: (u32, u32),
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &CacheKey) -> bool {
        self.map.strong_count() > 0
            && self.map.ptr_eq(&other.map)
            && self.height == other.height
            && self.normal == other.normal
            && self.settings == other.settings
            && self.size == other.size
            && self.block == other.block
    }
}
