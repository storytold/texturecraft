//! Parameters shared by every brush alpha: extent inside the stamp, roundness, contrast, invert.
//! The defaults leave a stamp exactly as it was before the parameters existed.

use serde::{Deserialize, Serialize};

/// Smallest extent a stamp's alpha can be shrunk to along one axis
pub const MIN_ALPHA_SIZE: f32 = 0.05;

/// Highest contrast; 1 leaves the alpha's values as they are
pub const MAX_ALPHA_CONTRAST: f32 = 8.0;

/// How the alpha is placed and graded inside the stamp square (see [`crate::Shape::sample`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlphaParams {
    /// Width of the alpha as a fraction of the stamp (1 = the whole stamp)
    pub size_x: f32,
    /// Height of the alpha as a fraction of the stamp
    pub size_y: f32,
    /// Round procedural shape only: 1 = circle, 0 = square
    pub roundness: f32,
    /// Steepness of the values around 0.5 (1 = unchanged, more = harder edges)
    pub contrast: f32,
    /// Swap paint and no paint inside the stamp square
    pub invert: bool,
}

impl Default for AlphaParams {
    fn default() -> Self {
        AlphaParams { size_x: 1.0, size_y: 1.0, roundness: 1.0, contrast: 1.0, invert: false }
    }
}

impl AlphaParams {
    /// Every value into its valid range (input from the UI, a file or an agent)
    pub fn sanitized(self) -> AlphaParams {
        let clamp_finite = |value: f32, min: f32, max: f32, default: f32| if value.is_finite() { value.clamp(min, max) } else { default };
        AlphaParams {
            size_x: clamp_finite(self.size_x, MIN_ALPHA_SIZE, 1.0, 1.0),
            size_y: clamp_finite(self.size_y, MIN_ALPHA_SIZE, 1.0, 1.0),
            roundness: clamp_finite(self.roundness, 0.0, 1.0, 1.0),
            contrast: clamp_finite(self.contrast, 1.0, MAX_ALPHA_CONTRAST, 1.0),
            invert: self.invert,
        }
    }

    /// The local coordinates the alpha is sampled at: the stamp's, stretched so the alpha fills
    /// only its extent
    pub fn place(&self, lx: f32, ly: f32) -> (f32, f32) {
        (lx / self.size_x, ly / self.size_y)
    }

    /// Contrast, then invert, on a sampled value `v` at stamp coordinates (`lx`, `ly`). Invert
    /// stays inside the stamp square, so the area around a shape isn't flooded with paint.
    pub fn grade(&self, v: f32, lx: f32, ly: f32) -> f32 {
        let v = if self.contrast == 1.0 { v } else { ((v - 0.5) * self.contrast + 0.5).clamp(0.0, 1.0) };
        match self.invert {
            true if lx.abs() <= 1.0 && ly.abs() <= 1.0 => 1.0 - v,
            true => 0.0,
            false => v,
        }
    }
}
