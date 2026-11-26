//! The Bevel Smooth filter's settings and the ramp it draws from a shape's edge

use serde::{Deserialize, Serialize};

use crate::{BevelDirection, RampCurve, SIZE_REFERENCE_RES, at_or_above_threshold, finite_unit};

/// A smooth ramp from the edge of the shapes (values at or above `threshold`), outwards, inwards or across it
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BevelSmooth {
    pub direction: BevelDirection,
    /// How far the ramp reaches from the edge, a fraction of the UV tile (0..0.3)
    pub distance: f32,
    /// How much the ramp is blurred (0..10, [`BevelSmooth::SMOOTHING_TEXELS`] texels at 2K each)
    pub smoothing: f32,
    /// Moves the ramp up or down before its curve (−1..1)
    pub curve_offset: f32,
    /// Bends the ramp: below 0 it rises later, above 0 sooner (−1..1)
    pub curve_shape: f32,
    /// Values at or above it are inside the shapes (Mask Threshold, 0..1)
    pub threshold: f32,
    /// Scales the distance (Distance Map Multiplier, 0..1)
    pub multiplier: f32,
    /// Reads past the texture's edges from the far side (Bevel Wrap)
    pub wrap: bool,
}

impl BevelSmooth {
    /// The Bevel Smooth filter as the library adds it
    pub const LIBRARY: BevelSmooth = BevelSmooth {
        direction: BevelDirection::Out,
        distance: 0.1,
        smoothing: 1.0,
        curve_offset: 0.0,
        curve_shape: 0.0,
        threshold: 0.5,
        multiplier: 1.0,
        wrap: false,
    };

    /// Texels at 2K a unit of Smoothing blurs over
    pub const SMOOTHING_TEXELS: f32 = 2.0;

    /// The largest Smoothing
    pub const SMOOTHING_MAX: f32 = 10.0;

    /// Whether `value` is inside the shapes
    pub fn inside(&self, value: f32) -> bool {
        at_or_above_threshold(value, self.threshold, Self::LIBRARY.threshold)
    }

    /// How many texels the ramp reaches at `res` texels across a UV tile
    pub fn reach(&self, res: u32) -> f32 {
        finite_unit(self.distance) * finite_unit(self.multiplier) * res as f32
    }

    /// How many texels the ramp is blurred over at `res` texels across a UV tile
    pub fn smoothing_texels(&self, res: u32) -> f32 {
        let smoothing = if self.smoothing.is_finite() { self.smoothing.clamp(0.0, Self::SMOOTHING_MAX) } else { 0.0 };
        smoothing * Self::SMOOTHING_TEXELS * res as f32 / SIZE_REFERENCE_RES
    }

    /// The curve the ramp takes (Curve Offset and Curve Shape), worked out once per filter
    pub fn curve(&self) -> RampCurve {
        RampCurve::new(self.curve_offset, self.curve_shape)
    }

    /// The ramp at a texel `edge` texels from the shapes' edge, inside them or not, bent by [`BevelSmooth::curve`]
    pub fn value(&self, curve: RampCurve, inside: bool, edge: f32, reach: f32) -> f32 {
        let along = if reach > 0.0 && edge.is_finite() { (edge / reach).clamp(0.0, 1.0) } else { 1.0 };
        let curved = curve.apply(along);
        match (self.direction, inside) {
            (BevelDirection::Out, true) => 1.0,
            (BevelDirection::Out, false) => 1.0 - curved,
            (BevelDirection::In, true) => curved,
            (BevelDirection::In, false) => 0.0,
            (BevelDirection::InOut, true) => 0.5 + 0.5 * curved,
            (BevelDirection::InOut, false) => 0.5 - 0.5 * curved,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ramp with the curve worked out as the filter does
    fn ramp(filter: BevelSmooth, inside: bool, edge: f32, reach: f32) -> f32 {
        filter.value(filter.curve(), inside, edge, reach)
    }

    #[test]
    fn the_ramp_lies_on_its_direction_side() {
        let out = BevelSmooth::LIBRARY;
        assert_eq!(out.reach(2048), 204.8);
        assert!(out.inside(0.5) && !out.inside(0.49));
        assert_eq!((ramp(out, true, 3.0, 10.0), ramp(out, false, 0.0, 10.0), ramp(out, false, 5.0, 10.0), ramp(out, false, 20.0, 10.0)), (1.0, 1.0, 0.5, 0.0));
        let inward = BevelSmooth { direction: BevelDirection::In, ..out };
        assert_eq!((ramp(inward, false, 3.0, 10.0), ramp(inward, true, 5.0, 10.0), ramp(inward, true, 20.0, 10.0)), (0.0, 0.5, 1.0));
        let both = BevelSmooth { direction: BevelDirection::InOut, ..out };
        assert_eq!(
            (ramp(both, true, 0.0, 10.0), ramp(both, false, 0.0, 10.0), ramp(both, true, 10.0, 10.0), ramp(both, false, 10.0, 10.0)),
            (0.5, 0.5, 1.0, 0.0)
        );
    }

    #[test]
    fn curve_offset_and_shape_bend_the_ramp() {
        let inward = BevelSmooth { direction: BevelDirection::In, ..BevelSmooth::LIBRARY };
        assert_eq!(ramp(BevelSmooth { curve_offset: 0.25, ..inward }, true, 5.0, 10.0), 0.75);
        assert!(ramp(BevelSmooth { curve_shape: 0.5, ..inward }, true, 5.0, 10.0) > 0.5, "rises sooner");
        assert!(ramp(BevelSmooth { curve_shape: -0.5, ..inward }, true, 5.0, 10.0) < 0.5, "rises later");
        assert_eq!(BevelSmooth { multiplier: 0.5, ..inward }.reach(2048), 102.4);
        let hostile = BevelSmooth { distance: f32::NAN, curve_offset: f32::NAN, curve_shape: f32::INFINITY, threshold: f32::NAN, ..inward };
        assert_eq!((hostile.reach(2048), ramp(hostile, true, 1.0, 0.0)), (0.0, 1.0));
    }
}
