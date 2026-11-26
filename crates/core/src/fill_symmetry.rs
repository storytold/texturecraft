//! Symmetry of a fill's 3D projection: mirrored or radial copies of it on the model

use serde::{Deserialize, Serialize};

use crate::{FULL_TURN_DEGREES, SymmetryKind};

/// Copies of a fill's projection: off for every fill saved before symmetry existed
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FillSymmetry {
    pub on: bool,
    pub kind: SymmetryKind,
    /// 0 = X, 1 = Y, 2 = Z: the mirror plane's normal, or the axis radial copies turn round
    pub axis: u8,
    /// Whether the copies' images are turned over across (U) and down (V)
    pub flip: [bool; 2],
    /// Where the mirror plane or the turning axis passes, in mesh radii from the mesh's centre
    pub position: [f32; 3],
    /// How many radial copies, the original included
    pub count: u32,
    /// The degrees the radial copies spread over; a full turn spaces them evenly round it
    pub span: f32,
}

impl FillSymmetry {
    /// Off; switched on, a mirror across X through the mesh's centre, or six radial copies over a full turn
    pub const OFF: FillSymmetry =
        FillSymmetry { on: false, kind: SymmetryKind::Mirror, axis: 0, flip: [false; 2], position: [0.0; 3], count: 6, span: FULL_TURN_DEGREES };
    /// The fewest and most radial copies
    pub const COUNTS: std::ops::RangeInclusive<u32> = 2..=64;
    /// The degrees an Angle span may be set to: at least one, at most a full turn
    pub const SPANS: std::ops::RangeInclusive<f32> = 1.0..=FULL_TURN_DEGREES;
    /// How far the axis position reaches from the mesh's centre, in mesh radii
    pub const REACH: f32 = 1.0;
    /// [`FillSymmetry::axis`] of the last axis, Z
    pub const LAST_AXIS: u8 = 2;

    /// The degrees between radial copies: a full turn spaces `count` copies evenly, a smaller span puts the first and last at its ends (a span that is not a number is a full turn)
    pub fn step(&self) -> f32 {
        let count = self.radial_count() as f32;
        let span = if self.span.is_nan() { FULL_TURN_DEGREES } else { self.span.clamp(0.0, FULL_TURN_DEGREES) };
        if span >= FULL_TURN_DEGREES { span / count } else { span / (count - 1.0) }
    }

    /// The copies there are, the original included: two for a mirror
    pub fn copies(&self) -> u32 {
        match self.kind {
            SymmetryKind::Mirror => 2,
            SymmetryKind::Radial => self.radial_count(),
        }
    }

    /// The axis the copies mirror across or turn round, the last one for an axis past it
    pub fn axis_index(&self) -> usize {
        usize::from(self.axis.min(Self::LAST_AXIS))
    }

    /// How many radial copies, kept within [`FillSymmetry::COUNTS`]
    fn radial_count(&self) -> u32 {
        self.count.clamp(*Self::COUNTS.start(), *Self::COUNTS.end())
    }

    /// Whether this is the symmetry a new fill starts with, so saves leave it out
    pub fn is_default(&self) -> bool {
        *self == FillSymmetry::default()
    }
}

impl Default for FillSymmetry {
    fn default() -> Self {
        FillSymmetry::OFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radial(count: u32, span: f32) -> FillSymmetry {
        FillSymmetry { on: true, kind: SymmetryKind::Radial, count, span, ..FillSymmetry::OFF }
    }

    #[test]
    fn a_span_under_a_full_turn_puts_the_last_copy_at_its_end() {
        assert_eq!(radial(4, 180.0).step(), 60.0);
        assert_eq!(radial(4, 360.0).step(), 90.0);
    }

    #[test]
    fn a_span_that_is_not_a_number_spaces_the_copies_round_a_full_turn() {
        assert_eq!(radial(4, f32::NAN).step(), 90.0);
    }
}
