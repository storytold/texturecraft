use std::collections::BTreeSet;
use std::sync::Arc;

use crate::below::StackKey;
use crate::eval::Fields;

/// Neighbourhood-filter fields kept between the composites of one pass.
pub(crate) struct HeldFields {
    /// Everything the fields read.
    pub key: Arc<StackKey>,
    /// The working size they were built at.
    pub size: (u32, u32),
    /// The tiles every layer's fields hold.
    pub stack_tiles: BTreeSet<(u32, u32)>,
    /// The tiles the hot layer's fields hold (the same tiles when the stack was not split).
    pub hot_tiles: BTreeSet<(u32, u32)>,
    pub fields: Arc<Fields>,
}

impl std::fmt::Debug for HeldFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldFields").field("stack_tiles", &self.stack_tiles.len()).field("hot_tiles", &self.hot_tiles.len()).finish_non_exhaustive()
    }
}
