//! Padding of the viewport composite. Uniform UV coverage has no pixels to copy.

use std::collections::BTreeSet;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use texturecraft_compose::{Composite, pad_tiles};
use texturecraft_geom::{NO_TRI, UvGBuffer};
use texturecraft_texel::DilationMap;

use crate::session::DISPLAY_PADDING;

/// The viewport's padding map. Reading the dense map explicitly also works for uniform UVs.
pub struct DisplayPadding {
    dense: OnceLock<Arc<DilationMap>>,
    size: (u32, u32),
    /// All texels covered or all uncovered. Mixed coverage needs the ordinary dilation map.
    uniform: Option<bool>,
}

impl DisplayPadding {
    pub(crate) fn new(gbuf: &UvGBuffer, has_coverage: bool, block: u32) -> Self {
        let size = (gbuf.width, gbuf.height);
        let uniform = if !has_coverage {
            Some(false)
        } else if !gbuf.tri.contains(&NO_TRI) {
            Some(true)
        } else {
            None
        };
        let dense = if uniform.is_none() {
            Arc::new(DilationMap::in_blocks(&gbuf.coverage_mask(), size.0, size.1, Some(DISPLAY_PADDING), block)).into()
        } else {
            OnceLock::new()
        };
        Self { dense, size, uniform }
    }

    pub(crate) fn apply_to_tiles(&self, output: &mut Composite, tiles: &[(u32, u32)]) -> BTreeSet<(u32, u32)> {
        if self.uniform.is_some() { BTreeSet::new() } else { pad_tiles(output, self, tiles) }
    }

    /// Whether every texel has a surface sample and the display needs no padding.
    pub fn fully_covered(&self) -> bool {
        self.uniform == Some(true)
    }

    /// Share existing island padding without constructing a dense map for uniform coverage.
    pub fn existing_sources(&self) -> Option<Arc<DilationMap>> {
        if self.uniform.is_none() { self.dense.get().cloned() } else { None }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.dense.get().map_or(0, |map| map.src.capacity() * size_of::<u32>())
    }
}

impl Deref for DisplayPadding {
    type Target = Arc<DilationMap>;

    fn deref(&self) -> &Self::Target {
        self.dense.get_or_init(|| {
            let (width, height) = self.size;
            let n = width as usize * height as usize;
            let src = if self.uniform == Some(true) { (0..n as u32).collect() } else { vec![texturecraft_texel::dilate::NONE; n] };
            Arc::new(DilationMap { width, height, src })
        })
    }
}
