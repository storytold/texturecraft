use serde::{Deserialize, Serialize};
use texturecraft_engine::core::BakeSettings;

/// Bake settings saved under a name. Loading one keeps each texture set's own Include in Bake flag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakePreset {
    pub name: String,
    pub settings: BakeSettings,
}
