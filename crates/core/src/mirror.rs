//! The Mirror filter's settings and where it reads each texel from

use serde::{Deserialize, Serialize};

use crate::{MirrorCorner, MirrorMode};

/// A mirror filter: one side of an axis (or one quarter) of each UV tile copied over the rest
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mirror {
    /// Draws the mirror's axes over the result (Display Debug)
    pub debug: bool,
    pub mode: MirrorMode,
    /// Where the X and Y axes sit across the tile (0..1)
    pub offset: [f32; 2],
    /// Copies the right (X) or bottom (Y) side instead of the left or top one
    pub invert: [bool; 2],
    pub corner: MirrorCorner,
}

impl Mirror {
    /// The Mirror filter as the library adds it: left onto right across the middle
    pub const LIBRARY: Mirror = Mirror { debug: false, mode: MirrorMode::AxisX, offset: [0.5, 0.5], invert: [false, false], corner: MirrorCorner::TopLeft };

    /// Half the width of a debug axis line, in texels at 2K
    pub const DEBUG_HALF_WIDTH: f32 = 2.0;

    /// Where in its tile (0..1 across, 0..1 down) the texel at `uv` reads from; a copy that reaches past an edge folds back at it
    pub fn source(&self, [u, v]: [f32; 2]) -> [f32; 2] {
        let across = |at: f32, axis: f32, keep_after: bool| {
            let copied = if keep_after { at < axis } else { at > axis };
            fold(if copied { 2.0 * axis - at } else { at })
        };
        match self.mode {
            MirrorMode::AxisX => [across(u, self.axis(0), self.invert[0]), v],
            MirrorMode::AxisY => [u, across(v, self.axis(1), self.invert[1])],
            MirrorMode::Corner => {
                let (right, bottom) = self.corner.sides();
                [across(u, 0.5, right), across(v, 0.5, bottom)]
            }
        }
    }

    /// Whether `uv` lies within `half_width` (a fraction of the tile) of one of the mirror's axes
    pub fn on_axis(&self, [u, v]: [f32; 2], half_width: f32) -> bool {
        let near = |at: f32, axis: f32| (at - axis).abs() <= half_width;
        match self.mode {
            MirrorMode::AxisX => near(u, self.axis(0)),
            MirrorMode::AxisY => near(v, self.axis(1)),
            MirrorMode::Corner => near(u, 0.5) || near(v, 0.5),
        }
    }

    /// The axis position along `axis` (0 = X, 1 = Y), within the tile even from a hostile file
    fn axis(&self, axis: usize) -> f32 {
        let at = self.offset.get(axis).copied().unwrap_or(0.5);
        if at.is_finite() { at.clamp(0.0, 1.0) } else { 0.5 }
    }
}

/// `at` folded back into 0..1 at the tile's edges, as a mirror reflects
fn fold(at: f32) -> f32 {
    let wrapped = at.rem_euclid(2.0);
    if wrapped > 1.0 { 2.0 - wrapped } else { wrapped }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_reads_the_kept_side() {
        let mut mirror = Mirror::LIBRARY;
        assert_eq!(mirror.source([0.25, 0.4]), [0.25, 0.4], "the left side stays");
        assert_eq!(mirror.source([0.75, 0.4]), [0.25, 0.4], "the right side reads the left");

        mirror.invert[0] = true;

        assert_eq!(mirror.source([0.25, 0.4]), [0.75, 0.4], "inverted, the left side reads the right");

        mirror.offset[0] = 0.25;
        mirror.invert[0] = false;

        assert_eq!(mirror.source([0.75, 0.0]), [0.25, 0.0], "a copy past the left edge folds back at it");

        mirror.mode = MirrorMode::AxisY;

        assert_eq!(mirror.source([0.9, 0.75]), [0.9, 0.25], "Axis Y reads the top for the bottom");

        mirror.mode = MirrorMode::Corner;
        mirror.corner = MirrorCorner::BottomRight;

        assert_eq!(mirror.source([0.25, 0.25]), [0.75, 0.75], "every quarter reads the kept one");
        assert_eq!(mirror.source([0.75, 0.75]), [0.75, 0.75]);
        assert!(mirror.on_axis([0.5, 0.1], 0.01) && mirror.on_axis([0.1, 0.5], 0.01) && !mirror.on_axis([0.1, 0.1], 0.01));
    }

    #[test]
    fn a_hostile_offset_stays_in_the_tile() {
        let mirror = Mirror { offset: [f32::NAN, f32::INFINITY], ..Mirror::LIBRARY };
        let [u, v] = mirror.source([0.9, 0.9]);
        assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v));
    }
}
