//! Whose mask a new mask effect goes in

/// Whose mask a mask effect goes in: an instance without a mask of its own shares its source's, unless the effect starts the instance's own
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum MaskOwner {
    Shared,
    Own,
}
