//! The Directional Distance filter's settings and the trail it draws from the shapes

use serde::{Deserialize, Serialize};

use crate::finite_unit;

/// A trail fading away from the shapes (values at or above [`DirectionalDistance::EDGE`]) towards `angle`
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectionalDistance {
    /// How far the trail reaches, a fraction of the UV tile (0..0.3)
    pub distance: f32,
    /// Where the trail runs to, in degrees counterclockwise from the right (270 runs down)
    pub angle: f32,
    /// Hardens the trail's fade (0..1)
    pub contrast: f32,
    /// Scales the distance (Distance Map Multiplier, 0..1)
    pub multiplier: f32,
}

impl DirectionalDistance {
    /// The Directional Distance filter as the library adds it
    pub const LIBRARY: DirectionalDistance = DirectionalDistance { distance: 0.1, angle: 270.0, contrast: 0.0, multiplier: 1.0 };

    /// Values at or above it are inside the shapes
    pub const EDGE: f32 = 0.5;

    /// The most contrast reaches: a hard step, kept off a division by zero
    const HARDEST: f32 = 0.99;

    /// How many texels the trail reaches at `res` texels across a UV tile
    pub fn reach(&self, res: u32) -> f32 {
        finite_unit(self.distance) * finite_unit(self.multiplier) * res as f32
    }

    /// The trail where the nearest shape behind a texel is `along` of the reach away (0 on a shape), or none is in reach
    pub fn value(&self, along: Option<f32>) -> f32 {
        let Some(along) = along.filter(|along| along.is_finite()) else { return 0.0 };
        let fade = 1.0 - along.clamp(0.0, 1.0);
        let contrast = if self.contrast.is_finite() { self.contrast.clamp(0.0, Self::HARDEST) } else { 0.0 };
        (0.5 + (fade - 0.5) / (1.0 - contrast)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trail_fades_with_distance_and_contrast_hardens_it() {
        let trail = DirectionalDistance::LIBRARY;
        assert_eq!(trail.reach(2048), 204.8);
        assert_eq!((trail.value(Some(0.0)), trail.value(Some(0.25)), trail.value(None)), (1.0, 0.75, 0.0));
        let hard = DirectionalDistance { contrast: 0.5, ..trail };
        assert_eq!((hard.value(Some(0.25)), hard.value(Some(0.8))), (1.0, 0.0));
        let hostile = DirectionalDistance { distance: f32::NAN, contrast: f32::INFINITY, multiplier: -1.0, ..trail };
        assert_eq!((hostile.reach(2048), hostile.value(Some(f32::NAN))), (0.0, 0.0));
    }
}
