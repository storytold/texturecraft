use std::collections::BTreeSet;
use std::sync::Weak;

use texturecraft_compose::ImageRes;
use texturecraft_geom::UvGBuffer;

use super::micro_request::MicroRequest;

/// The cache key of a program's micro maps: the requests, their images, the bake's maps, the
/// layout they are built in, and the coverage and sizes of the micro curvatures' detail sizes
pub(super) struct MicroKey {
    pub(super) requests: Vec<MicroRequest>,
    pub(super) images: Vec<[Option<Weak<ImageRes>>; 2]>,
    pub(super) maps: [Option<Weak<Vec<f32>>>; 2],
    pub(super) size: [u32; 3],
    pub(super) grid: [u32; 2],
    pub(super) level_sets: BTreeSet<usize>,
    pub(super) gbuf: Option<Weak<UvGBuffer>>,
    pub(super) needed: [bool; crate::gpu_curvature::SIZES],
}

impl MicroKey {
    pub(super) fn matches(&self, other: &MicroKey) -> bool {
        fn same<T>(a: &Option<Weak<T>>, b: &Option<Weak<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Weak::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }

        self.requests == other.requests
            && self.size == other.size
            && self.grid == other.grid
            && self.level_sets == other.level_sets
            && self.needed == other.needed
            && same(&self.gbuf, &other.gbuf)
            && self.maps.iter().zip(&other.maps).all(|(a, b)| same(a, b))
            && self.images.len() == other.images.len()
            && self.images.iter().zip(&other.images).all(|(a, b)| a.iter().zip(b).all(|(a, b)| same(a, b)))
    }
}
