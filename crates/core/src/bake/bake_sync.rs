use super::{BakeSyncGroup, BakedMap};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Which of a texture set's bake settings it shares with every other texture set sharing them; all are shared at first, as in the reference
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BakeSync {
    /// Shares Common Settings: output, high poly, cage and skew correction
    pub common: bool,
    /// The map types whose settings it shares
    pub maps: BTreeSet<BakedMap>,
}

impl Default for BakeSync {
    fn default() -> Self {
        Self { common: true, maps: BakedMap::ALL.into_iter().collect() }
    }
}

impl BakeSync {
    pub fn shares(&self, group: BakeSyncGroup) -> bool {
        match group {
            BakeSyncGroup::Common => self.common,
            BakeSyncGroup::Map(map) => self.maps.contains(&map),
        }
    }

    pub fn set_shares(&mut self, group: BakeSyncGroup, shares: bool) {
        match group {
            BakeSyncGroup::Common => self.common = shares,
            BakeSyncGroup::Map(map) if shares => {
                self.maps.insert(map);
            }
            BakeSyncGroup::Map(map) => {
                self.maps.remove(&map);
            }
        }
    }
}
