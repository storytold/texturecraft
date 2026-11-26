use std::sync::Weak;

use texturecraft_compose::ImageRes;

use super::micro_request::MicroRequest;

/// The cache key of a program's micro maps: the requests, their images, the bake's maps and the
/// layout they are built in
pub(super) struct MicroKey {
    pub(super) requests: Vec<MicroRequest>,
    pub(super) images: Vec<[Option<Weak<ImageRes>>; 2]>,
    pub(super) maps: [Option<Weak<Vec<f32>>>; 2],
    pub(super) size: [u32; 3],
    pub(super) grid: [u32; 2],
    pub(super) levels: Option<Option<usize>>,
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
            && self.levels == other.levels
            && self.maps.iter().zip(&other.maps).all(|(a, b)| same(a, b))
            && self.images.len() == other.images.len()
            && self.images.iter().zip(&other.images).all(|(a, b)| a.iter().zip(b).all(|(a, b)| same(a, b)))
    }
}
