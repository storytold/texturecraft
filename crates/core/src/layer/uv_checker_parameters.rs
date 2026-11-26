use serde::{Deserialize, Serialize};

/// The UV Checker generator's controls: a checker with a grid laid over each UV tile, the UV islands' borders drawn in a color
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UvCheckerParameters {
    /// A random light color per UV island instead of the checker color
    pub random_colors: bool,
    /// The checker's light squares (linear RGB); the dark ones are a shade of it
    pub checker_color: [f32; 3],
    /// Grid tiles across the UV tile, each four checker squares wide ([`UvCheckerParameters::GRID_TILE_RANGE`])
    pub grid_tile: u32,
    /// How strongly the grid lines darken the checker (0..1)
    pub grid_opacity: f32,
    /// The color of the UV islands' borders (linear RGB)
    pub border_color: [f32; 3],
    /// How wide the borders are (0..1; 0.2 is 3 texels of a 2K UV tile)
    pub border_width: f32,
}

impl UvCheckerParameters {
    pub const GRID_TILE_RANGE: std::ops::RangeInclusive<u32> = 1..=24;

    /// Every value brought into its range, non-finite ones to the defaults
    pub fn clamped(self) -> UvCheckerParameters {
        let defaults = UvCheckerParameters::default();
        let unit = |value: f32, default: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { default };

        UvCheckerParameters {
            checker_color: std::array::from_fn(|channel| unit(self.checker_color[channel], defaults.checker_color[channel])),
            grid_tile: self.grid_tile.clamp(*UvCheckerParameters::GRID_TILE_RANGE.start(), *UvCheckerParameters::GRID_TILE_RANGE.end()),
            grid_opacity: unit(self.grid_opacity, defaults.grid_opacity),
            border_color: std::array::from_fn(|channel| unit(self.border_color[channel], defaults.border_color[channel])),
            border_width: unit(self.border_width, defaults.border_width),
            ..self
        }
    }
}

impl Default for UvCheckerParameters {
    /// The reference app's defaults: a light cyan checker (#5BDDFF), red borders
    fn default() -> UvCheckerParameters {
        UvCheckerParameters {
            random_colors: false,
            checker_color: [0.104_616, 0.723_055, 1.0],
            grid_tile: 16,
            grid_opacity: 0.5,
            border_color: [1.0, 0.0, 0.0],
            border_width: 0.2,
        }
    }
}
