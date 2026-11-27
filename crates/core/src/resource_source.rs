//! Where an image resource was imported from, so the Resources Updater can read it again

use serde::{Deserialize, Serialize};

/// The file an image resource was read from and its modification time then.
///
/// The resource's stored bytes are the file's bytes at that time, so a file whose size or
/// contents differ from them has changed on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSource {
    /// Path of the file on the machine that imported it.
    pub path: String,
    /// The file's modification time when it was read, in milliseconds since the Unix epoch
    /// (0 when unknown). An unchanged time and size skip reading the file again.
    #[serde(default)]
    pub modified_ms: u64,
    /// The mesh map the file was imported as (`textureSet.meshMap.import`): an update resamples it
    /// again while the map still holds what this file gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_map: Option<crate::MeshMapSlot>,
}
