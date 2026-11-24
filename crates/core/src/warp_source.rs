//! The values a Warp's moves follow and how they are shaped first

use serde::{Deserialize, Serialize};

use crate::WarpSourceMode;
use crate::gradient_input::{NEUTRAL_BALANCE, shape_gray};

/// A Warp's source: noise, the values it filters or an image, blurred, balanced and contrasted
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarpSource {
    pub mode: WarpSourceMode,
    /// How widely the source is averaged first (0..1, up to [`WarpSource::BLUR_SHARE`] of the UV tile)
    pub blur: f32,
    /// Added to the source around 0.5 (0..1)
    pub balance: f32,
    /// Steepens the source around mid gray (0..1)
    pub contrast: f32,
    /// Source frequency multiplier within a UV tile (0.01..16)
    pub tiling: f32,
}

impl WarpSource {
    /// The source as the library's Warp starts with it
    pub const LIBRARY: WarpSource = WarpSource { mode: WarpSourceMode::DefaultNoise, blur: 0.0, balance: NEUTRAL_BALANCE, contrast: 0.0, tiling: 1.0 };

    /// Noise features across a UV tile at Source Tiling 1
    pub const NOISE_FEATURES: f32 = 4.0;

    /// The share of the UV tile a Source Blur of 1 averages over
    pub const BLUR_SHARE: f32 = 1.0 / 16.0;

    /// Mid gray: the source value that moves nothing, and the noise's middle
    pub const MID_GRAY: f32 = 0.5;

    /// How far noise is stretched from mid gray so it spans black to white
    pub const NOISE_STRETCH: f32 = 1.5;

    /// The Tiling range
    pub const TILING_MIN: f32 = 0.01;
    pub const TILING_MAX: f32 = 16.0;

    /// Noise features per UV tile
    pub fn noise_scale(&self) -> f32 {
        Self::NOISE_FEATURES * self.tiling_factor()
    }

    /// Source tiling after validating saved settings
    pub fn tiling_factor(&self) -> f32 {
        if self.tiling.is_finite() { self.tiling.clamp(Self::TILING_MIN, Self::TILING_MAX) } else { 1.0 }
    }

    /// How many texels across the source is averaged over at `res` texels across a UV tile
    pub fn blur_texels(&self, res: u32) -> f32 {
        let blur = if self.blur.is_finite() { self.blur.clamp(0.0, 1.0) } else { 0.0 };
        blur * Self::BLUR_SHARE * res as f32
    }

    /// Fractal noise `noise` (around mid gray) stretched to span 0..1
    pub fn stretch_noise(noise: f32) -> f32 {
        if noise.is_finite() { (Self::MID_GRAY + (noise - Self::MID_GRAY) * Self::NOISE_STRETCH).clamp(0.0, 1.0) } else { Self::MID_GRAY }
    }

    /// Source value `value` after Balance and Contrast
    pub fn shape(&self, value: f32) -> f32 {
        shape_gray(value, self.balance, self.contrast, false)
    }
}
