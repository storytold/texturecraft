//! Height To Normal's settings: the layer's Height turned into its Normal

use serde::{Deserialize, Serialize};

/// A Height To Normal filter: the normal of the slopes of the layer's Height, over or in place of its Normal
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeightToNormal {
    /// Overwrite Existing Normal: the made normal replaces the Normal below instead of being laid over it
    pub overwrite: bool,
    /// Use World Units: the slope comes from Surface Size and Height Depth instead of Normal Intensity
    pub world_units: bool,
    /// Surface Size (cm): how wide a UV tile is on the model
    pub surface_size: f32,
    /// Height Depth (cm): how deep a height of 1 is
    pub height_depth: f32,
    /// Normal Intensity: how strongly the slopes tilt the normal without world units
    pub intensity: f32,
}

impl HeightToNormal {
    /// A new Height To Normal
    pub const LIBRARY: HeightToNormal = HeightToNormal { overwrite: true, world_units: true, surface_size: 300.0, height_depth: 16.0, intensity: 10.0 };

    /// The smallest Surface Size: whole cm, so the reference's 0.1 rounds up to 1
    pub const MIN_SURFACE_SIZE: f32 = 1.0;

    /// The largest Surface Size (cm)
    pub const MAX_SURFACE_SIZE: f32 = 1000.0;

    /// The largest Height Depth (cm)
    pub const MAX_HEIGHT_DEPTH: f32 = 100.0;

    /// The largest Normal Intensity
    pub const MAX_INTENSITY: f32 = 50.0;

    /// Height Depth over Surface Size a Normal Intensity of 1 tilts like (so the default 10 matches the default 16 cm over 300 cm)
    const INTENSITY_UNIT: f32 = Self::LIBRARY.height_depth / Self::LIBRARY.surface_size / Self::LIBRARY.intensity;

    /// How much a change of height per texel tilts the normal at `res` texels across a UV tile (0 for hostile values)
    pub fn strength(&self, res: u32) -> f32 {
        let finite = |value: f32, max: f32| if value.is_finite() { value.clamp(0.0, max) } else { 0.0 };
        if self.world_units && !self.surface_size.is_finite() {
            return 0.0;
        }

        let per_tile = if self.world_units {
            finite(self.height_depth, Self::MAX_HEIGHT_DEPTH) / finite(self.surface_size, Self::MAX_SURFACE_SIZE).max(Self::MIN_SURFACE_SIZE)
        } else {
            finite(self.intensity, Self::MAX_INTENSITY) * Self::INTENSITY_UNIT
        };
        per_tile * res as f32
    }

    /// The unit tangent-space normal (OpenGL, +Y up the image) of a height changing by `gx` per texel to the right and `gy` per texel down
    pub fn normal(&self, [gx, gy]: [f32; 2], res: u32) -> [f32; 3] {
        let strength = self.strength(res);
        let (x, y) = (-gx * strength, gy * strength);
        let length = (x * x + y * y + 1.0).sqrt();
        if length.is_finite() { [x / length, y / length, 1.0 / length] } else { [0.0, 0.0, 1.0] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_units_and_normal_intensity_tilt_the_normal_up_the_slope() {
        let filter = HeightToNormal::LIBRARY;
        assert_eq!(filter.normal([0.0, 0.0], 2048), [0.0, 0.0, 1.0], "a flat height");
        let [x, y, z] = filter.normal([0.01, 0.0], 2048);
        assert!(x < 0.0 && y == 0.0 && z > 0.0, "rising to the right faces left");
        let expected = 0.01 * 16.0 / 300.0 * 2048.0;
        assert!((-x / z - expected).abs() < 1e-4, "16 cm over 300 cm");
        let intensity = HeightToNormal { world_units: false, ..filter };
        assert!((intensity.strength(2048) - filter.strength(2048)).abs() < 1e-3, "Normal Intensity 10 matches the world default");
        assert!(HeightToNormal { intensity: 20.0, ..intensity }.strength(2048) > intensity.strength(2048));
        assert!(HeightToNormal { height_depth: 32.0, ..filter }.strength(2048) > filter.strength(2048));
        assert_eq!(filter.normal([0.0, 0.01], 2048)[1].signum(), 1.0, "rising down the image faces up");
        assert_eq!(HeightToNormal { surface_size: f32::NAN, ..filter }.strength(2048), 0.0);
        let hostile = HeightToNormal { surface_size: f32::NAN, height_depth: f32::INFINITY, ..filter };
        assert!(hostile.normal([1.0, 1.0], 2048).iter().all(|v| v.is_finite()));
    }
}
