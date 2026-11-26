//! What a uniform channel's ramp row did

/// What a uniform channel's ramp row did this frame
pub(crate) struct RampEdit {
    /// `Some(merge)` when the value changed, `merge` true for a drag's later steps
    pub(crate) changed: Option<bool>,

    /// The invert glyph was clicked
    pub(crate) invert: bool,
}
