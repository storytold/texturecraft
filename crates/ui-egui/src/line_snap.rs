//! Ctrl+Shift+click straight lines: the end point turns about the start to the nearest angle step

use egui::{Pos2, Vec2};

/// The angle step of a snapped line, in degrees
pub const LINE_SNAP_STEP_DEGREES: f32 = 15.0;

/// `end` turned about `start` to the nearest multiple of `step_degrees` (measured in the plane's
/// own axes); the distance from `start` stays what it was. A zero step or a zero-length line is
/// returned unchanged.
pub fn snap_line_end(start: Pos2, end: Pos2, step_degrees: f32) -> Pos2 {
    let offset = end - start;
    let length = offset.length();
    if step_degrees <= 0.0 || length == 0.0 {
        return end;
    }

    let step = step_degrees.to_radians();
    let angle = (offset.y.atan2(offset.x) / step).round() * step;
    start + Vec2::new(angle.cos(), angle.sin()) * length
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    const START: Pos2 = pos2(100.0, 50.0);

    /// The angle of `end` from `START` in degrees
    fn angle(end: Pos2) -> f32 {
        (end - START).angle().to_degrees()
    }

    fn assert_close(actual: Pos2, expected: Pos2) {
        assert!((actual - expected).length() < 1e-3, "{actual:?} vs {expected:?}");
    }

    #[test]
    fn a_line_near_0_degrees_becomes_horizontal() {
        let end = snap_line_end(START, pos2(200.0, 55.0), 15.0);
        assert_close(end, pos2(100.0 + (100.0f32 * 100.0 + 25.0).sqrt(), 50.0));
    }

    #[test]
    fn a_line_near_15_degrees_turns_to_15_degrees() {
        let end = snap_line_end(START, START + Vec2::angled(18.0f32.to_radians()) * 80.0, 15.0);
        assert!((angle(end) - 15.0).abs() < 1e-3, "{}", angle(end));
    }

    #[test]
    fn a_line_near_45_degrees_turns_to_45_degrees() {
        let end = snap_line_end(START, START + Vec2::angled(40.0f32.to_radians()) * 60.0, 15.0);
        assert!((angle(end) - 45.0).abs() < 1e-3, "{}", angle(end));
        assert_close(end, START + Vec2::angled(45.0f32.to_radians()) * 60.0);
    }

    #[test]
    fn a_line_near_90_degrees_becomes_vertical() {
        let end = snap_line_end(START, pos2(104.0, 150.0), 15.0);
        assert!((end.x - START.x).abs() < 1e-3, "{end:?}");
        assert!(end.y > START.y);
    }

    #[test]
    fn negative_angles_snap_to_negative_multiples() {
        let end = snap_line_end(START, START + Vec2::angled((-37.0f32).to_radians()) * 70.0, 15.0);
        assert!((angle(end) + 30.0).abs() < 1e-3, "{}", angle(end));
        let backwards = snap_line_end(START, START + Vec2::angled((-172.0f32).to_radians()) * 70.0, 15.0);
        assert!((angle(backwards).abs() - 165.0).abs() < 1e-3, "{}", angle(backwards));
    }

    #[test]
    fn the_distance_from_the_start_is_kept() {
        for degrees in [-170.0f32, -100.0, -33.0, -7.0, 3.0, 22.0, 58.0, 91.0, 133.0, 179.0] {
            let end = START + Vec2::angled(degrees.to_radians()) * 123.0;
            let snapped = snap_line_end(START, end, 15.0);
            assert!(((snapped - START).length() - 123.0).abs() < 1e-3, "{degrees}");
            let multiple = angle(snapped) / 15.0;
            assert!((multiple - multiple.round()).abs() < 1e-3, "{degrees}: {}", angle(snapped));
        }
    }

    #[test]
    fn a_line_already_on_a_step_stays() {
        let end = START + Vec2::angled(60.0f32.to_radians()) * 40.0;
        assert_close(snap_line_end(START, end, 15.0), end);
    }

    #[test]
    fn a_zero_length_line_or_zero_step_is_unchanged() {
        assert_eq!(snap_line_end(START, START, 15.0), START);
        assert_eq!(snap_line_end(START, pos2(130.0, 77.0), 0.0), pos2(130.0, 77.0));
    }
}
