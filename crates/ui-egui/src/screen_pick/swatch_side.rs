/// Where the eyedropper's colour disc sits beside the pointer's tip: below right unless a screen edge would cut it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwatchSide {
    BelowRight,
    BelowLeft,
    AboveLeft,
    /// Past the pipette's bulb, so the disc never covers the pipette (only in a bottom-left corner)
    AboveRight,
}

/// The distance from the tip to the disc's centre at a 32-pixel pointer, in pixels: clear of the picked pixel and the pipette
const DISC_OFFSET_PX: f32 = 22.0;
/// How far right of the tip the disc's centre sits above the pipette, clear of its bulb
const PAST_BULB_PX: f32 = 44.0;

impl SwatchSide {
    /// Every side, in the order tried: the pipette points up right from its tip, so the disc moves round it the other way
    pub(crate) const ALL: [SwatchSide; 4] = [SwatchSide::BelowRight, SwatchSide::BelowLeft, SwatchSide::AboveLeft, SwatchSide::AboveRight];

    /// The disc's centre from the tip's pixel centre at a 32-pixel pointer, in pixels (y down)
    pub fn offset(self) -> [f32; 2] {
        match self {
            SwatchSide::BelowRight => [DISC_OFFSET_PX, DISC_OFFSET_PX],
            SwatchSide::BelowLeft => [-DISC_OFFSET_PX, DISC_OFFSET_PX],
            SwatchSide::AboveLeft => [-DISC_OFFSET_PX, -DISC_OFFSET_PX],
            SwatchSide::AboveRight => [PAST_BULB_PX, -DISC_OFFSET_PX],
        }
    }

    /// The first side whose disc (`reach` px, `unit` px per nominal px) by `tip` stays in `bounds` (left, top, right, bottom)
    pub fn fitting(tip: [f32; 2], unit: f32, reach: f32, bounds: [f32; 4]) -> Self {
        let [left, top, right, bottom] = bounds;
        let fits = |side: &SwatchSide| {
            let [dx, dy] = side.offset();
            let [x, y] = [tip[0] + dx * unit, tip[1] + dy * unit];
            x - reach >= left && x + reach <= right && y - reach >= top && y + reach <= bottom
        };
        Self::ALL.into_iter().find(fits).unwrap_or(SwatchSide::BelowRight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: [f32; 4] = [0.0, 0.0, 1920.0, 1080.0];

    #[test]
    fn the_disc_flips_away_from_the_edge_it_would_cross() {
        let reach = 21.0;

        assert_eq!(SwatchSide::fitting([900.0, 500.0], 1.0, reach, SCREEN), SwatchSide::BelowRight);
        assert_eq!(SwatchSide::fitting([1900.0, 500.0], 1.0, reach, SCREEN), SwatchSide::BelowLeft, "the right edge");
        assert_eq!(SwatchSide::fitting([900.0, 1070.0], 1.0, reach, SCREEN), SwatchSide::AboveLeft, "the bottom edge");
        assert_eq!(SwatchSide::fitting([1915.0, 1075.0], 1.0, reach, SCREEN), SwatchSide::AboveLeft, "the bottom-right corner");
        assert_eq!(SwatchSide::fitting([5.0, 1075.0], 1.0, reach, SCREEN), SwatchSide::AboveRight, "the bottom-left corner");
        assert_eq!(SwatchSide::fitting([900.0, 1050.0], 2.0, reach * 2.0, SCREEN), SwatchSide::AboveLeft, "a bigger pointer flips sooner");
    }

    #[test]
    fn a_monitor_left_of_the_main_one_has_its_own_edges() {
        let left_monitor = [-1280.0, 0.0, 0.0, 1024.0];

        assert_eq!(SwatchSide::fitting([-10.0, 300.0], 1.0, 21.0, left_monitor), SwatchSide::BelowLeft);
    }
}
