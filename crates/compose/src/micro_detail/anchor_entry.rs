use std::sync::{Arc, Weak};

use texturecraft_core::LayerId;

use super::anchor_heights::AnchorHeights;
use crate::below::StackKey;

/// The anchor heights built for a texture set's stack: the mesh maps they were built beside, the set, the anchor and the stack's key
pub(super) struct AnchorEntry {
    pub(super) baked: Weak<Vec<f32>>,
    pub(super) texture_set: usize,
    pub(super) anchor: (LayerId, usize),
    pub(super) stack: Arc<StackKey>,
    pub(super) heights: Arc<AnchorHeights>,
}
