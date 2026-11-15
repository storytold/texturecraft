use crate::store::ImageOrigin;

/// Keeps an image's allocation identity stable without retaining its bytes or pixels after resource removal, and
/// without decoding it: images decoded on first read and released again keep their identity.
#[derive(Debug)]
pub(crate) struct ImageIdentity(ImageOrigin);

impl ImageIdentity {
    pub(crate) fn new(origin: ImageOrigin) -> Self {
        Self(origin)
    }
}

impl PartialEq for ImageIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
