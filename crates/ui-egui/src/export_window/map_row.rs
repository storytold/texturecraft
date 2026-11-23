use crate::state::FileType;

/// One Output maps row: the map, its file name pattern and what it is written as without its
/// own override
pub(super) struct MapRow {
    pub(super) map: usize,
    pub(super) pattern: String,
    pub(super) inherited: FileType,
}
