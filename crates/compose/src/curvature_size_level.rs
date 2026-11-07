/// Deviations of ±0.5 (curvature − 0.5) stored as 16-bit integers.
pub const DEVIATION_SCALE: f32 = i16::MAX as f32 * 2.0;

/// Curvature blurred at one detail size: deviations from flat, stored `step` times coarser than the map.
pub struct CurvatureSizeLevel {
    pub step: u32,
    pub width: u32,
    pub height: u32,
    pub deviation: Vec<i16>,
}

impl CurvatureSizeLevel {
    /// The deviation at map texel `(x, y)`, bilinear between the stored texels.
    pub fn sample(&self, x: u32, y: u32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }

        let to_level = |texel: u32| (texel as f32 + 0.5) / self.step as f32 - 0.5;
        let (level_x, level_y) = (to_level(x).max(0.0), to_level(y).max(0.0));
        let (x0, y0) = ((level_x.floor() as u32).min(self.width - 1), (level_y.floor() as u32).min(self.height - 1));
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (blend_x, blend_y) = (level_x - x0 as f32, level_y - y0 as f32);
        let stored = |x: u32, y: u32| self.deviation.get((y * self.width + x) as usize).map_or(0.0, |value| f32::from(*value) / DEVIATION_SCALE);
        let top = stored(x0, y0) + (stored(x1, y0) - stored(x0, y0)) * blend_x;
        let bottom = stored(x0, y1) + (stored(x1, y1) - stored(x0, y1)) * blend_x;
        top + (bottom - top) * blend_y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_blend_the_stored_texels() {
        let level = CurvatureSizeLevel { step: 2, width: 2, height: 1, deviation: vec![0, (0.25 * DEVIATION_SCALE) as i16] };
        assert!(level.sample(0, 0).abs() < 1e-4, "the first stored texel covers map texels 0 and 1");
        assert!((level.sample(3, 0) - 0.25).abs() < 1e-3);
        assert!(level.sample(2, 0) > 0.0 && level.sample(2, 0) < 0.25, "between stored texels the value blends");
    }
}
