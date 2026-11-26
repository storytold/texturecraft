use texturecraft_geom::glam::Vec3;

/// Where the bake's transfer ray for one low-poly point starts
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RayStart {
    /// On the cage mesh: the offset from the cage to the low-poly point
    CageMesh(Vec3),
    /// Out along `up` by the frontal distance times `scale` (the painted cage amount)
    Distance { up: Vec3, scale: f32 },
    /// Out along `up` by `front` world units and reaching `rear` past the low-poly point: the automatic cage's own distances
    Fitted { up: Vec3, front: f32, rear: f32 },
}

impl RayStart {
    /// The ray origin for the low-poly point `position` at `frontal` world units of max frontal distance
    pub fn origin(&self, position: Vec3, frontal: f32) -> Vec3 {
        match *self {
            RayStart::CageMesh(offset) => position - offset,
            RayStart::Distance { up, scale } => position + up * frontal * scale,
            RayStart::Fitted { up, front, .. } => position + up * front,
        }
    }

    /// The ray's direction, how far it runs to the low-poly point and how far past it, at `frontal` and `rear` world units of the distance settings
    pub fn span(&self, frontal: f32, rear: f32) -> (Vec3, f32, f32) {
        match *self {
            RayStart::CageMesh(offset) => (offset.normalize_or_zero(), offset.length(), rear),
            RayStart::Distance { up, scale } => (-up, frontal * scale, rear),
            RayStart::Fitted { up, front, rear: fitted_rear } => (-up, front, fitted_rear),
        }
    }
}
