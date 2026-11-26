use texturecraft_geom::glam::Vec3;

/// A point on a triangle with the 3D directions of one UV unit along U and along V
pub(super) struct Frame {
    pub(super) pos: Vec3,
    pub(super) along_u: Vec3,
    pub(super) along_v: Vec3,
    /// Surface length of one UV unit, for the distance a copy may be from the mirrored point
    pub(super) scale: f32,
}
