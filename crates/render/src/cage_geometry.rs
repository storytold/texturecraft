//! A texture set's cage as the renderer takes it

use texturecraft_geom::glam::Vec3;

/// A cage for [`crate::Renderer::set_bake_cage`]: three corners per low-poly triangle, and lines as corner pairs
#[derive(Clone, Copy, Debug)]
pub struct CageGeometry<'a> {
    pub corners: &'a [Vec3],
    /// The low poly's polygon edges, for the wireframe
    pub edges: &'a [[u32; 2]],
    /// The low poly's hard edges without a UV seam
    pub missing_seams: &'a [[u32; 2]],
}
