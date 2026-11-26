use texturecraft_core::LayerId;

/// The layers selected in one texture set together with the active one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiSelection {
    /// The active layer these were picked with; another selected layer ends the selection.
    pub active: LayerId,
    /// Where a Shift+click range starts: the last layer clicked without Shift.
    pub anchor: LayerId,
    /// Every selected layer, the active one included, in the order they were picked.
    pub layers: Vec<LayerId>,
}
