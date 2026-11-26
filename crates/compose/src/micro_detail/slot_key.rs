use std::sync::Weak;

use super::anchor_heights::AnchorHeights;
use crate::store::ImageRes;

/// A slot as a cache key: an image by identity, a value by its bits
#[derive(Clone)]
pub(super) enum SlotKey {
    Uniform([u32; 3]),
    Image(Weak<ImageRes>, bool),
    Anchor(Weak<AnchorHeights>),
}

impl PartialEq for SlotKey {
    fn eq(&self, other: &SlotKey) -> bool {
        match (self, other) {
            (SlotKey::Uniform(a), SlotKey::Uniform(b)) => a == b,
            (SlotKey::Image(a, a_flip), SlotKey::Image(b, b_flip)) => a.strong_count() > 0 && a.ptr_eq(b) && a_flip == b_flip,
            (SlotKey::Anchor(a), SlotKey::Anchor(b)) => a.strong_count() > 0 && a.ptr_eq(b),
            _ => false,
        }
    }
}
