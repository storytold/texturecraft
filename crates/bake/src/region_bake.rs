use std::collections::BTreeMap;
use std::sync::Arc;
use texturecraft_core::BakedMap;

use crate::TexelBox;

/// A bake that samples only `texels` and takes every other texel from `previous`, a whole bake of the same size
#[derive(Clone, Copy, Debug)]
pub struct RegionBake<'a> {
    pub texels: TexelBox,
    pub previous: &'a BTreeMap<BakedMap, Arc<Vec<f32>>>,
}

/// Whether a region bake gives `map` a whole bake's pixels: not for maps scaled by their whole range or read from neighbours
pub fn region_keeps_pixels(map: BakedMap) -> bool {
    !matches!(map, BakedMap::Curvature | BakedMap::Height | BakedMap::Thickness)
}
