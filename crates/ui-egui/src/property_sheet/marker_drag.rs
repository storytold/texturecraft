//! Where a gradient's dragged marker moved

/// A marker that was dragged: which color, its new place and whether it continues the drag's undo step
pub(crate) struct MarkerDrag {
    pub(crate) index: usize,
    pub(crate) position: f32,
    pub(crate) merge: bool,
}
