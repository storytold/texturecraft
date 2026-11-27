use serde::{Deserialize, Serialize};

/// What changed since a mesh map was baked, which makes it out of date
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BakeChange {
    LowPoly,
    HighPoly,
    Cage,
    Settings,
    MapSettings,
}

impl BakeChange {
    /// Identifier in automation replies
    pub fn id(self) -> &'static str {
        match self {
            BakeChange::LowPoly => "lowPoly",
            BakeChange::HighPoly => "highPoly",
            BakeChange::Cage => "cage",
            BakeChange::Settings => "settings",
            BakeChange::MapSettings => "mapSettings",
        }
    }
}
