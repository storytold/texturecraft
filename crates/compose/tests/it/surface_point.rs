use texturecraft_geom::glam::Vec3;

/// A covered texel with its UV, world position and normal
pub(super) struct SurfacePoint {
    pub(super) texel: (u32, u32),
    pub(super) uv: [f32; 2],
    pub(super) position: Vec3,
    pub(super) normal: Vec3,
}
