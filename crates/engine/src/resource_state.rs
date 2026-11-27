//! One row of the Resources Updater: an image resource, its file and how they compare

use crate::resource_status::ResourceStatus;

/// An image resource, the file it came from and whether that file changed on disk
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceState {
    /// The resource's name in the project.
    pub name: String,
    /// How the resource compares with its file now.
    pub status: ResourceStatus,
    /// The file it was imported from or relinked to (None: made in the app or imported without a path).
    pub path: Option<String>,
    /// Why the file could not be checked (with [`ResourceStatus::Missing`]).
    pub error: Option<String>,
}
