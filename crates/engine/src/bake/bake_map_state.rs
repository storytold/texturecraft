use serde::Serialize;

/// Where one map of a bake stands, as the Baking Log shows it
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "reason")]
pub enum BakeMapState {
    Waiting,
    Baking,
    Baked,
    /// The bake of its texture set failed; the reason is the error
    Failed(String),
    /// The bake stopped before this map was kept
    Canceled,
}

impl BakeMapState {
    /// Whether the map can still change in this bake
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Waiting | Self::Baking)
    }

    /// Whether the map counts as an issue in the log
    pub fn is_issue(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Canceled)
    }
}
