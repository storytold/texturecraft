/// The mesh's size as the projections see it: the bounds radius (3D projections) and the mesh units one UV unit covers (UV)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshSize {
    pub radius: f32,
    pub uv_density: f32,
}
