/// The blocks of one size in a map: the first origin, the step between origins, how many across
/// and down, and their size
pub(super) struct BlockGroup {
    pub(super) origin: [u32; 2],
    pub(super) step: [u32; 2],
    pub(super) count: [u32; 2],
    pub(super) size: [u32; 2],
}
