//! What happened on an action row this frame

/// What an action row's user did this frame
pub(crate) struct ActionRowResponse {
    /// The button clicked
    pub(crate) clicked: Option<usize>,
    /// Whether the toggle flipped
    pub(crate) toggled: bool,
}
