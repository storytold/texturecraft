/// What a page remembers between frames
#[derive(Clone, Default)]
pub(super) struct PageState {
    /// Where each section's heading starts in the scrolled content
    pub(super) tops: Vec<f32>,
    /// The highlighted tab
    pub(super) active: usize,
    /// A tab clicked on the last frame, whose section the view scrolls to on this one
    pub(super) jump: Option<usize>,
    /// The clicked section stays highlighted while the view stays where the jump left it, even when the page cannot scroll that far
    pub(super) pinned: Option<(usize, f32)>,
}
