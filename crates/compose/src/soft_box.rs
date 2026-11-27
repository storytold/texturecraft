//! A soft rectangular light of the built-in environments

use texturecraft_geom::glam::Vec3;

/// A soft rectangular light: azimuth/elevation center (degrees), half sizes (degrees)
pub(crate) struct SoftBox {
    pub azimuth: f32,
    pub elevation: f32,
    pub half_width: f32,
    pub half_height: f32,
    pub color: Vec3,
}

/// Where the light starts to fade, as a share of its half size, and over how much of it
const EDGE_FADE_START: f32 = 0.8;
const EDGE_FADE_WIDTH: f32 = 0.4;

impl SoftBox {
    /// Convert the sampled direction to degrees once for all the lights
    pub fn angles(direction: Vec3) -> (f32, f32) {
        let elevation = direction.y.clamp(-1.0, 1.0).asin().to_degrees();
        let azimuth = direction.z.atan2(direction.x).to_degrees();

        (azimuth, elevation)
    }

    /// The light the box sends along the sampled azimuth and elevation
    pub fn eval(&self, azimuth: f32, elevation: f32) -> Vec3 {
        // The azimuth difference wrapped to -180..180 degrees
        let azimuth_step = ((azimuth - self.azimuth + 540.0).rem_euclid(360.0)) - 180.0;
        let (x, y) = (azimuth_step.abs() / self.half_width, (elevation - self.elevation).abs() / self.half_height);
        let edge = |t: f32| 1.0 - crate::environment_sky::smooth((t - EDGE_FADE_START) / EDGE_FADE_WIDTH);
        self.color * edge(x) * edge(y)
    }
}
