//! What a slot row offers besides its picture and words

/// A slot row's toggle, ×, ⟲ and whether the row opens the source list
pub(super) struct SlotEnds<'a> {
    /// The on/off pill at the right end, when the slot has one
    pub(super) on: Option<&'a mut bool>,

    /// The × back to a uniform source shows
    pub(super) clear: bool,

    /// The ⟲ shows: `Some(changed)`, faint and inert while unchanged
    pub(super) reset: Option<bool>,

    /// A click on the row opens the source list
    pub(super) pickable: bool,
}
