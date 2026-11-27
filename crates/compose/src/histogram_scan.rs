//! The histogram scan several generators finish with: a ramp slid through the values and narrowed by a contrast

/// `value` scanned at `position` (0 all black, 1 all white) over a ramp `1 − contrast` wide; contrast 1 is a hard step
pub(crate) fn histogram_scan(value: f32, position: f32, contrast: f32) -> f32 {
    // The ends hold at a hard step too, where the ramp has no width to reach them
    if position <= 0.0 {
        return 0.0;
    }

    if position >= 1.0 {
        return 1.0;
    }

    let width = 1.0 - contrast.clamp(0.0, 1.0);

    // The ramp's low end runs from 1 towards position 0 to −width towards position 1
    let low = (1.0 - position) * (1.0 + width) - width;
    if width <= f32::EPSILON {
        return if value >= low { 1.0 } else { 0.0 };
    }
    ((value - low) / width).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ends_of_the_position_are_all_black_and_all_white() {
        for value in [0.0, 0.3, 1.0] {
            assert_eq!(histogram_scan(value, 0.0, 0.5), 0.0, "{value}");
            assert_eq!(histogram_scan(value, 1.0, 0.5), 1.0, "{value}");
        }
    }

    #[test]
    fn the_ends_of_the_position_hold_at_a_hard_step() {
        assert_eq!(histogram_scan(1.0, 0.0, 1.0), 0.0, "position 0 is all black, even for white");
        assert_eq!(histogram_scan(0.0, 1.0, 1.0), 1.0, "position 1 is all white, even for black");
    }

    #[test]
    fn the_middle_position_keeps_mid_gray_and_contrast_narrows_around_it() {
        assert!((histogram_scan(0.5, 0.5, 0.5) - 0.5).abs() < 1e-6);
        assert!((histogram_scan(0.5, 0.5, 0.0) - 0.5).abs() < 1e-6);
        assert!((histogram_scan(0.7, 0.5, 0.5) - 0.9).abs() < 1e-6, "a ramp half wide from 0.25");
        assert!(histogram_scan(0.6, 0.5, 0.9) > histogram_scan(0.6, 0.5, 0.5));
        assert_eq!(histogram_scan(0.51, 0.5, 1.0), 1.0);
        assert_eq!(histogram_scan(0.49, 0.5, 1.0), 0.0);
    }
}
