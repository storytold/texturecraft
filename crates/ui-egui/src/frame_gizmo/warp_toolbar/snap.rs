/// `value` rounded to the nearest multiple of `step`; no step (0, negative or not a number) leaves it as it is
pub(crate) fn to_step(value: f32, step: f32) -> f32 {
    if step.is_finite() && step > 0.0 { (value / step).round() * step } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_snaps_to_the_nearest_multiple_of_the_step() {
        assert!((to_step(0.29, 0.2) - 0.2).abs() < 1e-6);
        assert!((to_step(0.31, 0.2) - 0.4).abs() < 1e-6);
        assert!((to_step(-0.31, 0.2) + 0.4).abs() < 1e-6, "negative moves snap the same way");
        assert_eq!(to_step(100.0, 45.0), 90.0);
    }

    #[test]
    fn no_step_leaves_the_value_alone() {
        for step in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(to_step(0.37, step), 0.37, "{step}");
        }
    }
}
