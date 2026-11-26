use texturecraft_engine::{Session, geom::glam::Vec3};

/// A 3D stroke's endpoint as a point on the model, scoped to its project, mesh and texture set
///
/// Shift+click projects it to the current view, so the line starts at the same spot on the model
/// after the camera moved. A point the model now hides still starts the line at its projected position.
#[derive(Clone, Copy, Debug)]
pub struct SurfacePaintPoint {
    mesh_id: u64,
    texture_set: usize,
    pos: Vec3,
}

impl SurfacePaintPoint {
    pub(crate) fn new(session: &Session, pos: Vec3) -> Self {
        Self { mesh_id: session.mesh_id, texture_set: session.ts(), pos }
    }

    /// The point in model space, or `None` once the mesh or texture set is not the one it was made on
    pub fn position_for(self, session: &Session) -> Option<Vec3> {
        (self.mesh_id == session.mesh_id && self.texture_set == session.ts()).then_some(self.pos)
    }
}
