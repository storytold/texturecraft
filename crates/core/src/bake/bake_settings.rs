use super::{BakeSync, BakeSyncGroup, CageSettings, CageType, CommonBakeSettings, HighPolySettings, MeshMapSettings, SkewSettings};
use serde::{Deserialize, Serialize};

/// Persisted clean-room bake settings, grouped by the controls they belong to
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BakeSettings {
    pub common: CommonBakeSettings,
    pub high_poly: HighPolySettings,
    pub cage: CageSettings,
    pub skew: SkewSettings,
    pub maps: MeshMapSettings,
    /// What this texture set shares with the others
    pub sync: BakeSync,
}

impl BakeSettings {
    /// Whether a painted skew map steers the rays: it needs a separate high poly and a distance-based cage with averaged normals
    pub fn skew_applies(&self) -> bool {
        !self.high_poly.use_low_as_high && self.cage.kind == CageType::DistanceBased && self.cage.average_normals
    }

    /// Take `group`'s settings from `source`, keeping whether this set is baked and which maps it bakes
    pub fn copy_group(&mut self, source: &BakeSettings, group: BakeSyncGroup) {
        match group {
            BakeSyncGroup::Common => {
                self.common = CommonBakeSettings { is_enabled: self.common.is_enabled, ..source.common };
                self.high_poly = source.high_poly.clone();
                self.cage = source.cage;
                self.skew = source.skew;
            }
            BakeSyncGroup::Map(map) => self.maps.copy_parameters(map, &source.maps),
        }
    }
}
