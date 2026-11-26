//! A factor slider: ×1 in the middle, halving to the left and doubling to the right

use std::ops::RangeInclusive;

use super::PropertySheet;
use super::slider::{KNOB_RADIUS, kept_in};
use super::track::Track;

impl PropertySheet<'_> {
    /// A factor (×1.50) on a track `doublings` halvings or doublings each way from ×1 (marked by the zero tick); a typed value may go past the track's ends, within `typed`. `Some(merge)` when it changed
    pub(crate) fn factor_slider(&mut self, label: &str, factor: &mut f32, doublings: f32, typed: RangeInclusive<f32>) -> Option<bool> {
        let (control, center_y) = self.row(label);
        let id = self.next_id();
        let track = Track { start: control.left() + KNOB_RADIUS, end: self.track_end(), center_y, zero: Some(0.0), ramp: false };
        let track_range = -doublings..=doublings;
        let mut along = kept_in(factor.max(f32::MIN_POSITIVE).log2(), &track_range);
        let dragged = self.track(id, label, &track, &mut along, &track_range);
        if dragged.is_some() {
            *factor = along.exp2();
        }
        let shown = format!("×{:.2}", *factor);
        let entered = self.column_value_field(id, &label, center_y, &shown, "", *factor, &typed, 2);
        Self::apply_typed(factor, entered, dragged)
    }
}
