/// Only the version is needed before handing the full package to MaterialCraft.
#[derive(serde::Deserialize)]
pub(super) struct VersionMarker {
    pub(super) version: u32,
}
