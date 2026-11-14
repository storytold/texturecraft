use crate::RunningBake;

/// A background bake of one or more texture sets.
pub struct BakeJob {
    pub queue: Vec<usize>,
    pub current: Option<RunningBake>,
    pub total: usize,
    /// The mesh the job was started on: a result for another mesh (changed or reimported meanwhile) is not applied.
    pub mesh_id: u64,
}
