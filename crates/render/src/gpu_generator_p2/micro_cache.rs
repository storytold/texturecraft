use std::sync::Arc;

use super::micro_key::MicroKey;
use super::micro_maps::MicroMaps;

pub(super) struct MicroCache {
    pub(super) key: MicroKey,
    pub(super) maps: Arc<MicroMaps>,
}
