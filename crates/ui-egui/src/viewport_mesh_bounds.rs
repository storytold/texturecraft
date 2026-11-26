use std::cell::RefCell;
use std::sync::{Arc, Weak};

use texturecraft_engine::geom::{Mesh, glam::Vec3};

/// Axis-aligned bounds shared by projection controls for the current mesh
#[derive(Default)]
pub(crate) struct ViewportMeshBounds {
    mesh: RefCell<Weak<Mesh>>,
    bounds: std::cell::Cell<(Vec3, Vec3)>,
}

impl ViewportMeshBounds {
    pub(crate) fn get(&self, mesh: &Arc<Mesh>) -> (Vec3, Vec3) {
        let mut previous = self.mesh.borrow_mut();
        if previous.as_ptr() != Arc::as_ptr(mesh) {
            self.bounds.set(mesh.bounds());
            *previous = Arc::downgrade(mesh);
        }

        self.bounds.get()
    }
}
