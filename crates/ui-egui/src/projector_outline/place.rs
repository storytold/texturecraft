use texturecraft_engine::compose::pattern::nonzero_stretch;
use texturecraft_engine::geom::glam::{EulerRot, Mat3, Vec3};

/// A round projector's turn, stretch and centre: a point in its own units lands at `turn × (stretch × point)` (the gizmo's placement)
pub(super) struct Place {
    pub(super) turn: Mat3,
    pub(super) stretch: Vec3,
    /// Its centre in its own units
    pub(super) local: Vec3,
}

impl Place {
    /// `offset` moves the centre by `-offset / per_local` of its own units
    pub(super) fn new(rotation: [f32; 3], scale: [f32; 3], offset: Vec3, per_local: f32) -> Place {
        let [x, y, z] = rotation.map(f32::to_radians);
        Place { turn: Mat3::from_euler(EulerRot::ZYX, z, y, x), stretch: Vec3::from(scale).map(nonzero_stretch), local: -offset / nonzero_stretch(per_local) }
    }

    pub(super) fn world(&self, local: Vec3) -> Vec3 {
        self.turn * (self.stretch * local)
    }

    /// The preview camera's view direction in the projector's own units: the rim is the circle across it
    pub(super) fn facing(&self) -> Vec3 {
        (self.turn.transpose() * crate::projection_preview::view_direction() / self.stretch).try_normalize().unwrap_or(Vec3::Z)
    }
}
