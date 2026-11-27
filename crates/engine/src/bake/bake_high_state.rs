use serde::Serialize;

/// Where one high poly file of a bake stands, as the Baking Log shows it
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "reason")]
pub enum BakeHighState {
    /// Its texture set has not started baking
    Waiting,
    Loading,
    Loaded,
    /// Its texture set's bake failed while getting the high polys ready; the reason is the error
    Failed(String),
    /// The bake stopped before the file was loaded
    Canceled,
}

impl BakeHighState {
    /// Whether the file can still change in this bake
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Waiting | Self::Loading)
    }

    /// Whether the file counts as an issue in the log
    pub fn is_issue(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Canceled)
    }
}
