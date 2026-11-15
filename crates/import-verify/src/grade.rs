//! [`Grade`]: one word for how close a channel comes to the stored result, from its [`Metrics`].
//!
//! | grade    | rule                                        |
//! |----------|---------------------------------------------|
//! | `exact`  | mean ≤ 0.5 steps and ≥ 99 % of samples within 1 step |
//! | `passes` | mean ≤ 0.5 steps and ≥ 95 % of samples within 1 step (the pass bar) |
//! | `near`   | mean ≤ 2 steps                              |
//! | `off`    | anything else                               |
//!
//! A mean of half a step is what rounding to 8 bits alone produces, so `exact` means "the same picture". A channel
//! whose largest difference reaches [`OUTLIER_STEPS`] is flagged for a look: a few wrong texels (a seam, a stray
//! stroke) can hide inside a passing mean. The flag does not change the grade.

use serde::Serialize;

use crate::Metrics;

/// Mean at or below which a channel can pass (8-bit rounding level).
pub const PASS_MEAN_STEPS: f64 = 0.5;
/// Share of samples within 1 step a passing channel needs, in percent.
pub const PASS_WITHIN1: f64 = 95.0;
/// Share of samples within 1 step an exact channel needs, in percent.
pub const EXACT_WITHIN1: f64 = 99.0;
/// Mean at or below which a channel that does not pass is still near.
pub const NEAR_MEAN_STEPS: f64 = 2.0;
/// Largest difference (steps) from which a channel is flagged for a visual check.
pub const OUTLIER_STEPS: f64 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    Off,
    Near,
    Passes,
    Exact,
}

impl Grade {
    pub const ALL: [Grade; 4] = [Grade::Exact, Grade::Passes, Grade::Near, Grade::Off];

    pub fn of(m: &Metrics) -> Grade {
        if !m.mean.is_finite() {
            return Grade::Off;
        }
        match (m.mean <= PASS_MEAN_STEPS, m.within1) {
            (true, w) if w >= EXACT_WITHIN1 => Grade::Exact,
            (true, w) if w >= PASS_WITHIN1 => Grade::Passes,
            _ if m.mean <= NEAR_MEAN_STEPS => Grade::Near,
            _ => Grade::Off,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Grade::Exact => "exact",
            Grade::Passes => "passes",
            Grade::Near => "near",
            Grade::Off => "off",
        }
    }

    /// At or above the pass bar.
    pub fn passes(self) -> bool {
        self >= Grade::Passes
    }
}

/// Whether a large difference flags the channel for a visual check.
pub fn is_outlier(m: &Metrics) -> bool {
    m.max >= OUTLIER_STEPS
}

/// Why a channel's grade is a safety net only and does not count toward the set's grade, or `None` when it counts.
///
/// Normal: the comparison is the normal channel alone, while normal maps exported with the default preset mix in the
/// baked mesh normal. A normal grade catches broken layers, but it is not the look a user exports.
pub fn safety_net_only(channel_id: &str) -> Option<&'static str> {
    match channel_id {
        "normal" => Some("safety net only: exported normal maps mix in the baked mesh normal; this compares the normal channel alone"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(mean: f64, within1: f64, max: f64) -> Metrics {
        Metrics { mean, within1, max, ..Metrics::default() }
    }

    #[test]
    fn grades_follow_the_bar() {
        assert_eq!(Grade::of(&m(0.0, 100.0, 0.0)), Grade::Exact);
        assert_eq!(Grade::of(&m(0.5, 99.0, 3.0)), Grade::Exact);
        assert_eq!(Grade::of(&m(0.5, 98.9, 3.0)), Grade::Passes);
        assert_eq!(Grade::of(&m(0.4, 95.0, 3.0)), Grade::Passes);
        // a low mean with too many texels off is not a pass
        assert_eq!(Grade::of(&m(0.4, 94.9, 3.0)), Grade::Near);
        assert_eq!(Grade::of(&m(0.51, 100.0, 1.0)), Grade::Near);
        assert_eq!(Grade::of(&m(2.0, 10.0, 9.0)), Grade::Near);
        assert_eq!(Grade::of(&m(2.01, 10.0, 9.0)), Grade::Off);
        assert_eq!(Grade::of(&m(f64::NAN, 100.0, 0.0)), Grade::Off);
        assert!(Grade::Exact.passes() && Grade::Passes.passes() && !Grade::Near.passes());
        assert!(Grade::Exact > Grade::Off);
    }

    #[test]
    fn outliers_flag_without_changing_the_grade() {
        let spiky = m(0.1, 99.9, 51.7);
        assert!(is_outlier(&spiky) && Grade::of(&spiky) == Grade::Exact);
        assert!(!is_outlier(&m(0.1, 99.9, 39.9)));
    }

    #[test]
    fn only_normal_is_a_safety_net() {
        assert!(safety_net_only("normal").is_some());
        assert!(safety_net_only("baseColor").is_none() && safety_net_only("height").is_none());
    }
}
