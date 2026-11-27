//! The shapes the projection preview wraps its checker around

use texturecraft_engine::geom::glam::Vec3;

use super::{CUBE_HALF, CYLINDER_HALF_HEIGHT, CYLINDER_RADIUS, SPHERE_RADIUS};

/// What the preview wraps the checker around
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreviewShape {
    Sphere,
    Cylinder,
    Cube,
}

impl PreviewShape {
    /// Half the shape's size along X, Y and Z, in mesh radii
    pub(super) fn half_size(self) -> Vec3 {
        match self {
            PreviewShape::Sphere => Vec3::splat(SPHERE_RADIUS),
            PreviewShape::Cylinder => Vec3::new(CYLINDER_RADIUS, CYLINDER_HALF_HEIGHT, CYLINDER_RADIUS),
            PreviewShape::Cube => Vec3::splat(CUBE_HALF),
        }
    }
}
