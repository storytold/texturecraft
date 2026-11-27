//! One of Baked Lighting Stylized's extra lights

use serde::{Deserialize, Serialize};

/// A light from `azimuth` around +Y (degrees from +Z) and `elevation` above the horizon (degrees), of strength `intensity` in `color`
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectionalLight {
    pub azimuth: f32,
    pub elevation: f32,
    pub intensity: f32,
    pub color: [f32; 3],
}

impl DirectionalLight {
    /// An unlit white light from `azimuth`, 45 degrees up
    pub const fn off(azimuth: f32) -> DirectionalLight {
        DirectionalLight { azimuth, elevation: 45.0, intensity: 0.0, color: [1.0; 3] }
    }

    /// The unit direction toward the light
    pub fn direction(&self) -> [f32; 3] {
        toward(self.azimuth, self.elevation)
    }
}

/// The unit direction toward a light at `azimuth` around +Y (degrees from +Z) and `elevation` above the horizon (degrees)
fn toward(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (sin_azimuth, cos_azimuth) = azimuth.to_radians().sin_cos();
    let (sin_elevation, cos_elevation) = elevation.to_radians().sin_cos();
    [cos_elevation * sin_azimuth, sin_elevation, cos_elevation * cos_azimuth]
}
