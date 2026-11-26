use super::{MapAction, PendingDrop};

/// What the output map rows asked for in this frame.
#[derive(Default)]
pub(super) struct RowEdit {
    pub(super) action: Option<MapAction>,
    /// A map dropped on a channel swatch, to ask which of its channels to use.
    pub(super) pending: Option<PendingDrop>,
}
