pub(super) struct SaveTarget {
    pub(super) project_id: u64,
    pub(super) saved_revision: Option<u64>,
    pub(super) adopt: bool,
    pub(super) path: String,
}
