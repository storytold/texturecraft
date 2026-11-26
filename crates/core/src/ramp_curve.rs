//! The bend Bevel Smooth's ramp and Mask Outline's fade take, sanitised once per filter

/// A Curve Shape of 1 raises a ramp to the power ¼ and −1 to the power 4, 0 leaves it straight
const CURVE_SHAPE_BASE: f32 = 4.0;

/// A 0..1 ramp moved by an offset, then raised to a power: below 0 the shape rises later, above 0 sooner
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RampCurve {
    offset: f32,
    exponent: f32,
}

impl RampCurve {
    /// The curve moving a ramp by `offset` and bending it by `shape` (each −1..1, 0 when not finite)
    pub fn new(offset: f32, shape: f32) -> RampCurve {
        let bounded = |x: f32| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 };
        RampCurve { offset: bounded(offset), exponent: CURVE_SHAPE_BASE.powf(-bounded(shape)) }
    }

    /// Ramp value `along` (0..1) moved and bent
    pub fn apply(self, along: f32) -> f32 {
        (along + self.offset).clamp(0.0, 1.0).powf(self.exponent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_bends_and_the_offset_moves() {
        assert_eq!(RampCurve::new(0.0, 0.0).apply(0.3), 0.3, "straight");
        assert_eq!(RampCurve::new(0.0, 1.0).apply(0.0625), 0.5, "the power ¼ rises sooner");
        assert_eq!(RampCurve::new(0.0, -1.0).apply(0.5), 0.0625, "the power 4 rises later");
        assert_eq!(RampCurve::new(0.25, 0.0).apply(0.5), 0.75);
        assert_eq!(RampCurve::new(f32::NAN, f32::INFINITY), RampCurve::new(0.0, 0.0), "hostile settings leave it straight");
    }
}
