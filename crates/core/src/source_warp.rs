//! The Warp filter's settings and the moves it makes

use serde::{Deserialize, Serialize};

use crate::{WarpBlending, WarpMode, WarpSource, WarpSourceMode, finite_unit};

/// Texels read from where a source sends them: along its slope, along one direction, or along several blended
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceWarp {
    /// The default noise's seed
    pub seed: u32,
    pub mode: WarpMode,
    /// How far a texel moves at most, a share of the UV tile before the divider (0..1)
    pub intensity: f32,
    /// Divides `intensity` (1, 10, 100 or 1000) for fine moves
    pub divider: u16,
    /// The direction (degrees) of the directional modes, the first of several in Multi Directional Warp
    pub angle: f32,
    /// How many directions a Multi Directional Warp spreads evenly around the circle (1, 2 or 4)
    pub directions: u8,
    pub blending: WarpBlending,
    pub source: WarpSource,
}

impl SourceWarp {
    /// The Warp filter as the library adds it
    pub const LIBRARY: SourceWarp = SourceWarp {
        seed: 0,
        mode: WarpMode::Warp,
        intensity: 0.1,
        divider: 10,
        angle: 0.0,
        directions: 4,
        blending: WarpBlending::Average,
        source: WarpSource::LIBRARY,
    };

    /// The direction counts a Multi Directional Warp takes
    pub const DIRECTIONS: [u8; 3] = [1, 2, 4];

    /// The share of the UV tile the source's slope is measured across, on each side of a texel
    pub const PROBE_SHARE: f32 = 1.0 / 256.0;

    /// The source's slope (rise per UV tile) that moves a texel the whole way in Warp mode
    pub const FULL_SLOPE: f32 = 4.0;

    /// How many texels a texel moves at most at `res` texels across a UV tile
    pub fn move_texels(&self, res: u32) -> f32 {
        move_texels(self.intensity, self.divider, res)
    }

    /// How many texels each side of a texel Warp mode measures the source's slope across
    pub fn probe_texels(res: u32) -> f32 {
        (res as f32 * Self::PROBE_SHARE).max(1.0)
    }

    /// The directions (degrees) texels move along: none in Warp mode (the slope decides), the Angle, or Directions spread from it
    pub fn angles(&self) -> impl Iterator<Item = f32> + Clone + use<> {
        let angle = if self.angle.is_finite() { self.angle } else { 0.0 };
        let count = match self.mode {
            WarpMode::Warp => 0,
            WarpMode::Directional => 1,
            WarpMode::MultiDirectional if Self::DIRECTIONS.contains(&self.directions) => self.directions,
            WarpMode::MultiDirectional => 1,
        };
        (0..count).map(move |index| angle + 360.0 * f32::from(index) / f32::from(count))
    }

    /// How many texels around a texel the warp reads at `res` texels across a UV tile: its moves (chained ones add up) and where it reads its source
    pub fn reach(&self, res: u32) -> f32 {
        let chained = self.mode == WarpMode::MultiDirectional && self.blending == WarpBlending::Chain;
        let moves = self.move_texels(res) * if chained { self.angles().count().max(1) as f32 } else { 1.0 };
        if self.source.mode == WarpSourceMode::DefaultNoise {
            return moves;
        }

        let probe = if self.mode == WarpMode::Warp { Self::probe_texels(res) } else { 0.0 };
        moves + probe + self.source.blur_texels(res) * std::f32::consts::FRAC_1_SQRT_2
    }

    /// How far (−1..1 of the move) a source value moves a texel along a direction: backwards below mid gray, forwards above
    pub fn amount(value: f32) -> f32 {
        if value.is_finite() { (2.0 * value - 1.0).clamp(-1.0, 1.0) } else { 0.0 }
    }

    /// Warp mode's move (texels) from how much the source rises across [`SourceWarp::probe_texels`] each side of a texel along x and y
    pub fn slope_move(&self, rise: [f32; 2], res: u32) -> [f32; 2] {
        let across = 2.0 * Self::probe_texels(res) / res.max(1) as f32;
        let [x, y] = rise.map(|rise| if rise.is_finite() { rise / across / Self::FULL_SLOPE } else { 0.0 });
        let length = x.hypot(y);
        let fit = if length > 1.0 { 1.0 / length } else { 1.0 };
        let reach = self.move_texels(res);
        [x * fit * reach, y * fit * reach]
    }
}

/// How many texels Intensity over Divider moves at `res` texels across a UV tile (Warp's and Blur Slope's)
pub(crate) fn move_texels(intensity: f32, divider: u16, res: u32) -> f32 {
    finite_unit(intensity) / f32::from(divider.max(1)) * res as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_follow_intensity_mode_and_directions() {
        let warp = SourceWarp::LIBRARY;
        assert!((warp.move_texels(2048) - 20.48).abs() < 1e-3, "0.1 ÷ 10 of the tile");
        assert_eq!(warp.angles().count(), 0);

        let directional = SourceWarp { mode: WarpMode::Directional, angle: 30.0, ..warp };
        assert_eq!(directional.angles().collect::<Vec<_>>(), vec![30.0]);

        let multi = SourceWarp { mode: WarpMode::MultiDirectional, angle: 10.0, ..warp };
        assert_eq!(multi.angles().collect::<Vec<_>>(), vec![10.0, 100.0, 190.0, 280.0]);

        let chained = SourceWarp { blending: WarpBlending::Chain, ..multi };
        assert!((chained.reach(2048) - 4.0 * 20.48).abs() < 1e-3, "four moves one after the other");

        let custom = SourceWarp { source: WarpSource { mode: WarpSourceMode::CustomNoise, blur: 1.0, ..WarpSource::LIBRARY }, ..warp };
        assert!(custom.reach(2048) >= custom.move_texels(2048) + SourceWarp::probe_texels(2048) + custom.source.blur_texels(2048) * 0.5);

        let input = SourceWarp { source: WarpSource { mode: WarpSourceMode::PreviousInput, ..WarpSource::LIBRARY }, ..warp };
        assert!((input.reach(2048) - (20.48 + 8.0)).abs() < 1e-3, "plus where it measures the slope");
        assert_eq!((SourceWarp::amount(0.0), SourceWarp::amount(0.5), SourceWarp::amount(1.0)), (-1.0, 0.0, 1.0));
    }

    #[test]
    fn slope_moves_saturate_and_hostile_settings_stay_in_range() {
        let warp = SourceWarp::LIBRARY;
        let gentle = warp.slope_move([0.01, 0.0], 2048);
        assert!(gentle[0] > 0.0 && gentle[0] < warp.move_texels(2048), "{gentle:?}");

        let steep = warp.slope_move([1.0, 1.0], 2048);
        assert!((steep[0].hypot(steep[1]) - warp.move_texels(2048)).abs() < 1e-3, "a cliff moves the whole way, no further: {steep:?}");

        let hostile = SourceWarp { intensity: f32::NAN, angle: f32::INFINITY, directions: 9, divider: 0, mode: WarpMode::MultiDirectional, ..warp };
        assert_eq!((hostile.move_texels(2048), hostile.angles().collect::<Vec<_>>()), (0.0, vec![0.0]));
        assert_eq!(hostile.slope_move([f32::NAN, 1.0], 2048)[0], 0.0);
        assert_eq!(SourceWarp { intensity: 9.0, divider: 1, ..warp }.move_texels(1024), 1024.0);
    }
}
