use std::sync::Weak;

use crate::Texels;

/// A mesh map a whole-map field samples: its allocation and the size it was baked at (one allocation read as another layout reads other texels)
#[derive(Clone)]
pub(crate) struct SampledMap {
    pub(crate) data: Weak<Texels>,
    pub(crate) baked_size: (u32, u32),
}

impl PartialEq for SampledMap {
    fn eq(&self, other: &SampledMap) -> bool {
        self.data.strong_count() > 0 && self.data.ptr_eq(&other.data) && self.baked_size == other.baked_size
    }
}
