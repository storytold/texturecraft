//! Settings and values sanitised to the 0..1 range filters read them in

/// A value at its threshold still counts when float rounding left it a step below (a white read back as 0.99999994)
const THRESHOLD_TOLERANCE: f32 = f32::EPSILON;

/// `x` clamped to 0..1, 0 when not finite
pub fn finite_unit(x: f32) -> f32 {
    if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
}

/// Whether `value` reaches `threshold` (clamped to 0..1, `fallback` when not finite)
pub fn at_or_above_threshold(value: f32, threshold: f32, fallback: f32) -> bool {
    let threshold = if threshold.is_finite() { threshold.clamp(0.0, 1.0) } else { fallback };
    value >= threshold - THRESHOLD_TOLERANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_land_in_the_unit_range() {
        assert_eq!([finite_unit(-2.0), finite_unit(0.25), finite_unit(7.0), finite_unit(f32::NAN), finite_unit(f32::INFINITY)], [0.0, 0.25, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_threshold_counts_a_value_rounded_just_below_it() {
        assert!(at_or_above_threshold(1.0 - f32::EPSILON / 2.0, 1.0, 0.5));
        assert!(!at_or_above_threshold(0.49, 0.5, 0.5));
        assert!(at_or_above_threshold(0.5, f32::NAN, 0.5), "a hostile threshold falls back");
        assert!(at_or_above_threshold(1.0, 9.0, 0.5), "a threshold past 1 is 1");
    }
}
