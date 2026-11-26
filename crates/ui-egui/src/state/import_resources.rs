use std::collections::BTreeSet;

use super::{ImportRow, ResourceUsage};

/// File > Import resources: the files added so far, imported by the Import button
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ImportResources {
    pub rows: Vec<ImportRow>,
    /// Ids of the selected rows
    pub selected: BTreeSet<u64>,
    /// The row a Shift click extends from
    pub anchor: Option<u64>,
    /// The usage the list is narrowed to (None: all)
    pub filter: Option<ResourceUsage>,
    /// Whether Enter requested the import in the next frame
    pub confirm: bool,
    /// The project that was open when this window opened
    pub project: u64,
    /// This opening's identity, used to reject late file picker answers
    pub(crate) opening_id: u64,
    pub(crate) next_row: u64,
}
