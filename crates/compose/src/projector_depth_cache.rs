use std::sync::{Arc, Mutex, Weak};

use texturecraft_core::Projection;
use texturecraft_geom::{Mesh, glam::Vec3};

use crate::{projector_depth::ProjectorDepth, projector_view::ProjectorView};

type ViewDepth = (ProjectorView, Option<Arc<ProjectorDepth>>);

// Each map holds about 3 MiB
const MAX_CACHED_VIEWS: usize = 8;

/// Reuse projector depth maps for one mesh across tile batches.
#[derive(Default)]
pub struct ProjectorDepthCache {
    mesh: Weak<Mesh>,
    maps: Mutex<Vec<ViewDepth>>,
}

impl ProjectorDepthCache {
    pub(crate) fn bind(&mut self, mesh: &Arc<Mesh>) -> bool {
        if !std::ptr::eq(self.mesh.as_ptr(), mesh.as_ref()) {
            self.maps.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
            self.mesh = Arc::downgrade(mesh);
            return true;
        }

        false
    }

    pub(crate) fn get(&self, mesh: &Mesh, bounds: (Vec3, Vec3), projection: &Projection) -> Option<Arc<ProjectorDepth>> {
        let view = ProjectorView::of(projection, bounds)?;

        // Rasterization is serial so tile workers can wait for a single build
        let mut maps = self.maps.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(index) = maps.iter().position(|(cached_view, _)| *cached_view == view) {
            let entry = maps.remove(index);
            let depth = entry.1.clone();
            maps.insert(0, entry);
            return depth;
        }

        let depth = ProjectorDepth::build(view, mesh).map(Arc::new);
        maps.insert(0, (view, depth.clone()));
        maps.truncate(MAX_CACHED_VIEWS);

        depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::{Projector, UvTransform};
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
            let map = cache.get(&mesh, bounds, &projection).unwrap();
            let reused = cache.get(&mesh, bounds, &projection).unwrap();
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
        let last_map = Arc::downgrade(&cache.get(&mesh, mesh.bounds(), &projection).unwrap());

        drop(cache);
        assert!(last_map.upgrade().is_none());
    }
}
