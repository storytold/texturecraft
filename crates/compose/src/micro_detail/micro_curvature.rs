use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_core::{BakedMap, GeneratorKind, MicroDetails};

use super::cache_key::CacheKey;
use super::cached_bytes::CachedBytes;
use super::layout_map::LayoutMap;
use super::slot::Slot;
use super::{AnchorHeights, CURVATURE_CACHE, by_blocks, curvature_term, layout};
use crate::eval::Inputs;

/// The curvature a generator's micro details add
pub struct MicroCurvature {
    /// What they add to the curvature input
    pub term: LayoutMap,
    /// The baked curvature with the term added, which the detail sizes blur
    pub curvature: Arc<Vec<f32>>,
}

impl MicroCurvature {
    /// The curvature `micro` adds on a `kind` generator, in the baked curvature map's layout (None without a baked curvature map)
    pub fn of(inp: &Inputs, kind: GeneratorKind, micro: &MicroDetails, anchor: Option<Arc<AnchorHeights>>) -> Option<Arc<MicroCurvature>> {
        let (curvature, size, block) = layout(inp, BakedMap::Curvature)?;
        let height = micro.height.then(|| Slot::of(inp, micro.height_source.as_ref(), anchor));
        let normal = micro.normal.then(|| Slot::of(inp, micro.normal_source.as_ref(), None));
        let settings = vec![micro.curvature_type as u32, micro.curvature_intensity.to_bits(), micro.height_intensity.to_bits(), kind as u32];
        let key =
            CacheKey { map: Arc::downgrade(&curvature), height: height.as_ref().map(Slot::key), normal: normal.as_ref().map(Slot::key), settings, size, block };

        Some(CURVATURE_CACHE.get_or_build(key, || {
            let same_in_every_block = height.iter().chain(&normal).all(Slot::same_in_every_block);
            let term = by_blocks(size, block, same_in_every_block, |origin, block| {
                let height = height.as_ref().map(|height| height.in_block(origin, size));
                let normal = normal.as_ref().map(|normal| normal.in_block(origin, size));
                curvature_term(micro, height.as_ref(), normal.as_ref(), block)
            });
            let adjusted = curvature.par_iter().zip(term.par_iter()).map(|(curvature, term)| curvature + term).collect();
            MicroCurvature { term: LayoutMap { data: term, size }, curvature: Arc::new(adjusted) }
        }))
    }
}

impl CachedBytes for MicroCurvature {
    fn bytes(&self) -> usize {
        self.term.bytes().saturating_add(self.curvature.len().saturating_mul(size_of::<f32>()))
    }
}
