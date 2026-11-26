use std::sync::{Arc, Weak};

use texturecraft_engine::{compose::BakedMaps, core::BakedMap};

/// The identity of one baked map read by a linked cook.
pub(super) struct LinkBakeInput {
    map: BakedMap,
    resolution: Option<u32>,
    pixels: Option<Weak<Vec<f32>>>,
}

impl LinkBakeInput {
    pub(super) fn capture(map: BakedMap, baked: Option<&BakedMaps>) -> Self {
        let pixels = baked.and_then(|baked| baked.maps.get(&map)).map(Arc::downgrade);

        Self { map, resolution: baked.filter(|_| pixels.is_some()).map(|baked| baked.resolution), pixels }
    }

    pub(super) fn is_current(&self, baked: Option<&BakedMaps>) -> bool {
        let current = Self::capture(self.map, baked);
        match (&self.pixels, current.pixels) {
            (Some(previous), Some(now)) => self.resolution == current.resolution && previous.ptr_eq(&now),
            (None, None) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_baked_input_at_the_same_resolution_invalidates_its_cook() {
        let earlier = Arc::new(vec![0.25; 4]);
        let mut baked = BakedMaps { resolution: 2, maps: [(BakedMap::AmbientOcclusion, Arc::clone(&earlier))].into() };
        let ambient_occlusion = LinkBakeInput::capture(BakedMap::AmbientOcclusion, Some(&baked));
        let curvature = LinkBakeInput::capture(BakedMap::Curvature, Some(&baked));

        assert!(ambient_occlusion.is_current(Some(&baked)));
        assert_eq!(Arc::strong_count(&earlier), 2);

        baked.maps.insert(BakedMap::AmbientOcclusion, Arc::new(vec![0.75; 4]));

        assert!(!ambient_occlusion.is_current(Some(&baked)));
        assert!(curvature.is_current(Some(&baked)));

        baked.maps.insert(BakedMap::Curvature, Arc::new(vec![0.5; 4]));

        assert!(!curvature.is_current(Some(&baked)));
    }
}
