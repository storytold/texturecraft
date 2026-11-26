//! The projection preview's fixed camera

use texturecraft_engine::geom::glam::Vec3;

use super::{CAMERA_PITCH, CAMERA_YAW};

/// The preview camera's directions: towards the eye, the view direction, and the view's right and up
#[derive(Clone, Copy, Debug)]
pub(super) struct View {
    pub(super) eye: Vec3,
    pub(super) forward: Vec3,
    pub(super) right: Vec3,
    pub(super) up: Vec3,
}

impl View {
    /// The fixed preview camera, turned CAMERA_YAW round and CAMERA_PITCH down
    pub(super) fn new() -> View {
        let (yaw, pitch) = (CAMERA_YAW.to_radians(), CAMERA_PITCH.to_radians());
        let eye = Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos());
        let forward = -eye;
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        View { eye, forward, right, up: right.cross(forward) }
    }
}
