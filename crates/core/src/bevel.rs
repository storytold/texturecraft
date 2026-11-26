//! The Bevel filter's settings and the slope it gives a shape's edge

use serde::{Deserialize, Serialize};

use crate::{BevelBlur, BevelCorner, SIZE_REFERENCE_RES};

/// Shapes (values at or above [`Bevel::EDGE`]) sloped at their edge: inwards for a negative distance, outwards for a positive one
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bevel {
    /// Reads past the texture's edges from the far side (Bevel Wrap)
    pub wrap: bool,
    /// How far the slope reaches from the edge, a fraction of the UV tile: inwards below 0, outwards above (−1..1)
    pub distance: f32,
    pub corner: BevelCorner,
    /// How much the slope is blurred (0..5, [`Bevel::SMOOTHING_TEXELS`] texels at 2K each)
    pub smoothing: f32,
    pub blur: BevelBlur,
}

impl Bevel {
    /// The Bevel filter as the library adds it
    pub const LIBRARY: Bevel = Bevel { wrap: false, distance: -0.1, corner: BevelCorner::Round, smoothing: 0.0, blur: BevelBlur::NonUniform };

    /// Values at or above it are inside the shapes
    pub const EDGE: f32 = 0.5;

    /// Texels at 2K a unit of Smoothing blurs over
    pub const SMOOTHING_TEXELS: f32 = 4.0;

    /// The largest Smoothing
    pub const SMOOTHING_MAX: f32 = 5.0;

    /// Whether the slope lies inside the shapes
    pub fn inward(&self) -> bool {
        self.distance <= 0.0 || self.distance.is_nan()
    }

    /// How many texels the slope reaches at `res` texels across a UV tile
    pub fn reach(&self, res: u32) -> f32 {
        let distance = if self.distance.is_finite() { self.distance.abs().min(1.0) } else { 0.0 };
        distance * res as f32
    }

    /// How many texels the slope is blurred over at `res` texels across a UV tile
    pub fn smoothing_texels(&self, res: u32) -> f32 {
        let smoothing = if self.smoothing.is_finite() { self.smoothing.clamp(0.0, Self::SMOOTHING_MAX) } else { 0.0 };
        smoothing * Self::SMOOTHING_TEXELS * res as f32 / SIZE_REFERENCE_RES
    }

    /// The slope (0..1) at a texel `edge` texels from the shapes' edge, inside them or not, rising from 0 at its foot to 1 at its top
    pub fn slope(&self, inside: bool, edge: f32, reach: f32) -> f32 {
        let along = if reach > 0.0 && edge.is_finite() { (edge / reach).clamp(0.0, 1.0) } else { 1.0 };
        match (self.inward(), inside) {
            (true, true) => along,
            (false, false) => 1.0 - along,
            (true, false) => 0.0,
            (false, true) => 1.0,
        }
    }

    /// The result at a texel of `value`, inside the shapes or not, on `slope`: an inward slope scales the shapes, an outward one rises around them
    pub fn apply(&self, inside: bool, value: f32, slope: f32) -> f32 {
        match (self.inward(), inside) {
            (true, true) => value * slope,
            (false, false) => value.max(slope),
            _ => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_distance_slopes_inwards_and_a_positive_one_outwards() {
        let inward = Bevel::LIBRARY;
        assert_eq!(inward.reach(2048), 204.8);
        assert_eq!((inward.slope(true, 0.0, 10.0), inward.slope(true, 5.0, 10.0), inward.slope(true, 20.0, 10.0)), (0.0, 0.5, 1.0));
        assert_eq!(inward.apply(true, 0.8, 0.5), 0.4, "inside, scaled down towards the edge");
        assert_eq!(inward.apply(false, 0.3, 0.0), 0.3, "outside untouched");
        let outward = Bevel { distance: 0.1, ..inward };
        assert_eq!((outward.slope(false, 0.0, 10.0), outward.slope(false, 5.0, 10.0), outward.slope(false, 20.0, 10.0)), (1.0, 0.5, 0.0));
        assert_eq!(outward.apply(false, 0.2, 0.5), 0.5, "rises around the shapes");
        assert_eq!(outward.apply(true, 0.7, 1.0), 0.7, "inside untouched");
    }

    #[test]
    fn hostile_settings_stay_in_range() {
        let bevel = Bevel { distance: f32::NAN, smoothing: f32::INFINITY, ..Bevel::LIBRARY };
        assert_eq!((bevel.reach(2048), bevel.smoothing_texels(2048)), (0.0, 0.0));
        assert_eq!(bevel.slope(true, 3.0, 0.0), 1.0, "no reach leaves the shapes whole");
        let excessive = Bevel { distance: -9.0, smoothing: 9.0, ..bevel };
        assert_eq!(excessive.reach(1024), 1024.0);
        assert_eq!(excessive.smoothing_texels(2048), Bevel::SMOOTHING_MAX * Bevel::SMOOTHING_TEXELS, "smoothing stops at its largest");
    }
}
