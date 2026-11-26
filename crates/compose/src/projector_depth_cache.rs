use std::sync::{Arc, Mutex, Weak};

use texturecraft_geom::Mesh;

use crate::fill_depth::{FillDepth, FillDepthKey};
use crate::micro_detail::AnchorHeightCache;

pub(crate) type ViewDepth = (FillDepthKey, Option<Arc<FillDepth>>);

// Each fill's grids share about 3 MiB
const MAX_CACHED_VIEWS: usize = 8;

/// Reuse projector depth maps for one mesh across tile batches.
#[derive(Default)]
pub struct ProjectorDepthCache {
    mesh: Weak<Mesh>,
    maps: Mutex<Vec<ViewDepth>>,
    pub(crate) micro_heights: AnchorHeightCache,
}

impl ProjectorDepthCache {
    pub(crate) fn bind(&mut self, mesh: &Arc<Mesh>) {
        if !std::ptr::eq(self.mesh.as_ptr(), mesh.as_ref()) {
            self.maps.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
            self.micro_heights = AnchorHeightCache::default();
            self.mesh = Arc::downgrade(mesh);
        }
    }

    pub(crate) fn get(&self, mesh: &Mesh, key: FillDepthKey) -> Option<Arc<FillDepth>> {
        // Rasterization is serial so tile workers can wait for a single build
        let mut maps = self.maps.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(index) = maps.iter().position(|(cached_key, _)| *cached_key == key) {
            let entry = maps.remove(index);
            let depth = entry.1.clone();
            maps.insert(0, entry);
            return depth;
        }

        let depth = FillDepth::build(mesh, key).map(Arc::new);
        maps.insert(0, (key, depth.clone()));
        maps.truncate(MAX_CACHED_VIEWS);

        depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projector_view::ProjectorView;
    use texturecraft_core::{Projection, Projector, UvTransform};
    use texturecraft_geom::primitives;

    #[test]
    fn eviction_and_mesh_replacement_release_maps_after_the_last_reader() {
        let mut cache = ProjectorDepthCache::default();
        let mut mesh = Arc::new(primitives::plane(2.0, 2));
        let mesh_reference = Arc::downgrade(&mesh);
        let bounds = mesh.bounds();
        let mut maps = Vec::new();
        let mut first_reader = None;

        cache.bind(&mesh);
        for angle_degrees in 0..MAX_CACHED_VIEWS + 4 {
            let mut projector = Projector::depth_culled();
            projector.rotation[2] = angle_degrees as f32;
            let projection = Projection::Planar { scale: 1.0, axis: 1, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: None };
            let view = ProjectorView::of(&projection, bounds).unwrap();
            let map = cache.get(&mesh, (view, None)).unwrap();
            let reused = cache.get(&mesh, (view, None)).unwrap();

            assert!(Arc::ptr_eq(&map, &reused));
            maps.push(Arc::downgrade(&map));
            if angle_degrees == 0 {
                first_reader = Some(map);
            }
        }

        assert_eq!(maps.iter().filter(|map| map.upgrade().is_some()).count(), MAX_CACHED_VIEWS + 1);
        drop(first_reader);
        assert_eq!(maps.iter().filter(|map| map.upgrade().is_some()).count(), MAX_CACHED_VIEWS);

        // Mutating a uniquely held Arc disassociates its weak references
        Arc::make_mut(&mut mesh).positions[0].y += 1.0;
        assert!(mesh_reference.upgrade().is_none());
        cache.bind(&mesh);
        assert!(maps.iter().all(|map| map.upgrade().is_none()));

        let projection =
            Projection::Planar { scale: 1.0, axis: 1, offset: [0.0; 3], uv: UvTransform::default(), projector: Projector::depth_culled(), placed: None };
        let view = ProjectorView::of(&projection, mesh.bounds()).unwrap();
        let last_map = Arc::downgrade(&cache.get(&mesh, (view, None)).unwrap());

        drop(cache);
        assert!(last_map.upgrade().is_none());
    }
}
