use super::{CageSettings, CageType, CommonBakeSettings, HighPolySettings, MeshMapSettings, SkewSettings};
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
}

impl BakeSettings {
    /// Whether a painted skew map steers the rays: it needs a separate high poly and a distance-based cage with averaged normals
    pub fn skew_applies(&self) -> bool {
        !self.high_poly.use_low_as_high && self.cage.kind == CageType::DistanceBased && self.cage.average_normals
    }
}
