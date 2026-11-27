//! A remembered comparison of a resource with its file, so an unchanged file is not read again

use std::sync::{Arc, Weak};

use crate::resource_status::ResourceStatus;

/// What reading a resource's file found, kept while the file's size and time stay the same
#[derive(Clone, Debug)]
pub(crate) struct ResourceCheck {
    pub path: String,
    pub size: u64,
    pub modified_ms: u64,
    /// The resource bytes the file was compared with (an update or undo replaces them)
    pub bytes: Weak<Vec<u8>>,
    pub status: ResourceStatus,
}

impl ResourceCheck {
    /// Whether this verdict still holds for the file and the resource's bytes
    pub(crate) fn holds_for(&self, path: &str, size: u64, modified_ms: u64, bytes: &Arc<Vec<u8>>) -> bool {
        self.path == path && self.size == size && self.modified_ms == modified_ms && std::ptr::eq(self.bytes.as_ptr(), Arc::as_ptr(bytes))
    }
}
