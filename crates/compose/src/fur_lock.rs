use texturecraft_geom::glam::Vec2;

/// One lock of fur, with its root relative to its cell.
pub(crate) struct FurLock {
    pub(crate) root: Vec2,
    /// Length along the axis, in cells.
    pub(crate) length: f32,
    pub(crate) angle_sin: f32,
    pub(crate) angle_cos: f32,
    /// Total turn of the axis, in radians.
    pub(crate) curl: f32,
    pub(crate) strength: f32,
}
