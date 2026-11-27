//! The Blur Slope filter's settings and how far it reads

use serde::{Deserialize, Serialize};

use crate::{SlopeBlending, SourceWarp, WarpSource, WarpSourceMode, source_warp};

/// Values read downhill along a source's slope, averaged or kept at their smallest or largest
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSlopeBlur {
    /// The default noise's seed
    pub seed: u32,
    /// How far downhill a texel reads, a share of the UV tile before the divider (0..1)
    pub intensity: f32,
    /// Divides `intensity` (1, 10, 100 or 1000) for short smears
    pub divider: u16,
    pub blending: SlopeBlending,
    /// The share of the reads taken along the way (0..1): fewer is faster and grainier
    pub quality: f32,
    pub source: WarpSource,
}

impl SourceSlopeBlur {
    /// The Blur Slope filter as the library adds it
    pub const LIBRARY: SourceSlopeBlur =
        SourceSlopeBlur { seed: 0, intensity: 0.3, divider: 10, blending: SlopeBlending::Blur, quality: 1.0, source: WarpSource::LIBRARY };

    /// How many texels downhill a texel reads at most at `res` texels across a UV tile
    pub fn move_texels(&self, res: u32) -> f32 {
        source_warp::move_texels(self.intensity, self.divider, res)
    }

    /// How many of `most` reads along the way Quality keeps (at least one past the texel itself)
    pub fn samples(&self, most: u32) -> u32 {
        let quality = if self.quality.is_finite() { self.quality.clamp(0.0, 1.0) } else { 1.0 };
        ((most as f32 * quality).round() as u32).clamp(1, most.max(1))
    }

    /// How many texels around a texel the filter reads at `res` texels across a UV tile: downhill, and where it measures its source's slope
    pub fn reach(&self, res: u32) -> f32 {
        let moves = self.move_texels(res);
        if self.source.mode == WarpSourceMode::DefaultNoise {
            return moves;
        }

        moves + SourceWarp::probe_texels(res) + self.source.blur_texels(res) * std::f32::consts::FRAC_1_SQRT_2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_follow_intensity_quality_and_source() {
        let slope = SourceSlopeBlur::LIBRARY;
        assert!((slope.move_texels(2048) - 61.44).abs() < 1e-3, "0.3 ÷ 10 of the tile");
        assert_eq!((slope.samples(24), SourceSlopeBlur { quality: 0.5, ..slope }.samples(24)), (24, 12));
        assert_eq!(SourceSlopeBlur { quality: 0.0, ..slope }.samples(24), 1, "always one read downhill");
        assert_eq!(SourceSlopeBlur { quality: f32::NAN, ..slope }.samples(24), 24);
        let custom = SourceSlopeBlur { source: WarpSource { mode: WarpSourceMode::CustomNoise, blur: 1.0, ..WarpSource::LIBRARY }, ..slope };
        assert!(custom.reach(2048) >= custom.move_texels(2048) + SourceWarp::probe_texels(2048) + custom.source.blur_texels(2048) * 0.5);

        let input = SourceSlopeBlur { source: WarpSource { mode: WarpSourceMode::PreviousInput, ..WarpSource::LIBRARY }, ..slope };
        assert!((input.reach(2048) - (61.44 + 8.0)).abs() < 1e-3, "plus where it measures the slope");
        assert_eq!(SourceSlopeBlur { intensity: f32::INFINITY, ..slope }.move_texels(2048), 0.0);
    }
}
