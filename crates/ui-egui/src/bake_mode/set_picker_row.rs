/// One texture set listed in the set picker
#[derive(Debug)]
pub(super) struct SetPickerRow {
    pub(super) index: usize,
    pub(super) name: String,
    /// For a set not yet sharing a sync group, the values it would give up
    pub(super) note: Option<String>,
}
