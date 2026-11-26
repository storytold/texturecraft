/// What a row of size columns did this frame
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SizeColumnsResponse {
    /// The column edited and whether the edit continues the previous undo step
    pub(crate) edit: Option<(usize, bool)>,

    /// The column under the pointer or being dragged
    pub(crate) hovered: Option<usize>,
}
