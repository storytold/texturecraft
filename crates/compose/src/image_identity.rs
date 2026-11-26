use std::sync::{Arc, Weak};

use crate::store::ImageRes;

/// Keeps an image's allocation identity stable without retaining pixels after resource removal.
#[derive(Debug)]
pub(crate) struct ImageIdentity(Weak<ImageRes>);

impl ImageIdentity {
    pub(crate) fn new(image: &Arc<ImageRes>) -> Self {
        Self(Arc::downgrade(image))
    }
}

impl PartialEq for ImageIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
