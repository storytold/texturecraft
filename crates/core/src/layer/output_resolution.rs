use serde::{Deserialize, Serialize};

use super::resolution_mode::ResolutionMode;

/// The resolution a generator computes its output at, before it is sampled onto the texture set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputResolution {
    pub mode: ResolutionMode,
    /// Texels per side for [`ResolutionMode::Custom`].
    pub custom: u32,
    /// Power of two applied to the base size (−3 = 1/8 … 3 = ×8; 0 = the same as the context).
    pub factor: i8,
}

impl OutputResolution {
    /// The factors the reference app offers: 1/8 to ×8.
    pub const FACTORS: std::ops::RangeInclusive<i8> = -3..=3;
    /// The sizes offered for [`ResolutionMode::Custom`].
    pub const CUSTOM_SIZES: [u32; 6] = [128, 256, 512, 1024, 2048, 4096];
    /// The largest output, the largest texture set resolution.
    pub const MAX_SIZE: u32 = 8192;
    /// The coarsest output: one value per this many texels of the context per side (a compositing tile).
    pub const MAX_CELL: u32 = 64;

    pub fn is_default(&self) -> bool {
        *self == OutputResolution::default()
    }

    /// Whether some UV tile can get a coarser output than its own resolution (a custom size, the set's size or a factor below 1).
    pub fn can_reduce(&self) -> bool {
        matches!(self.mode, ResolutionMode::Custom | ResolutionMode::TextureSet) || self.factor < 0
    }

    /// The largest cell a UV tile of up to `uv_tile` texels per side can get, whatever its texture set's resolution.
    pub fn largest_cell(&self, uv_tile: u32) -> u32 {
        if self.mode == ResolutionMode::TextureSet { OutputResolution::MAX_CELL } else { self.cell(uv_tile, uv_tile) }
    }

    /// Texels per side of the output for a UV tile of `uv_tile` texels per side in a texture set of `texture_set`.
    pub fn output_size(&self, uv_tile: u32, texture_set: u32) -> u32 {
        let base = match self.mode {
            ResolutionMode::Auto | ResolutionMode::UvTile => uv_tile,
            ResolutionMode::TextureSet => texture_set,
            ResolutionMode::Custom => self.custom,
        };
        let factor = self.factor.clamp(*OutputResolution::FACTORS.start(), *OutputResolution::FACTORS.end());
        let scaled = if factor >= 0 { base.saturating_mul(1 << factor) } else { base >> -factor };
        scaled.clamp((uv_tile / OutputResolution::MAX_CELL).max(1), OutputResolution::MAX_SIZE)
    }

    /// How many texels of the UV tile one output texel covers per side (1 when the output is as fine or finer).
    pub fn cell(&self, uv_tile: u32, texture_set: u32) -> u32 {
        (uv_tile / self.output_size(uv_tile, texture_set)).max(1)
    }
}

impl Default for OutputResolution {
    fn default() -> Self {
        OutputResolution { mode: ResolutionMode::Auto, custom: 2048, factor: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_follows_the_uv_tile() {
        let resolution = OutputResolution::default();
        assert_eq!(resolution.output_size(1024, 2048), 1024);
        assert_eq!(resolution.cell(1024, 2048), 1);
        assert!(!resolution.can_reduce());
    }

    #[test]
    fn the_texture_set_mode_reads_the_set_not_the_uv_tile() {
        let resolution = OutputResolution { mode: ResolutionMode::TextureSet, factor: -1, ..OutputResolution::default() };
        assert_eq!(resolution.output_size(512, 2048), 1024, "half the set's 2048");
        assert_eq!(resolution.cell(512, 2048), 1, "finer than the 512 tile: one texel per texel");
    }

    #[test]
    fn factors_and_custom_sizes_scale_the_output() {
        let eighth = OutputResolution { factor: -3, ..OutputResolution::default() };
        assert_eq!((eighth.output_size(2048, 2048), eighth.cell(2048, 2048)), (256, 8));

        let custom = OutputResolution { mode: ResolutionMode::Custom, custom: 512, factor: 1 };
        assert_eq!((custom.output_size(2048, 2048), custom.cell(2048, 2048)), (1024, 2));

        let finer = OutputResolution { factor: 3, ..OutputResolution::default() };
        assert_eq!((finer.output_size(2048, 2048), finer.cell(2048, 2048)), (8192, 1), "a finer output than the tile keeps one texel per texel");
    }

    #[test]
    fn the_largest_cell_covers_any_texture_set() {
        let set = OutputResolution { mode: ResolutionMode::TextureSet, ..OutputResolution::default() };
        assert!(set.can_reduce(), "a UV tile larger than its set gets the set's coarser size");
        assert_eq!(set.largest_cell(2048), OutputResolution::MAX_CELL);
        assert_eq!(OutputResolution { factor: -2, ..OutputResolution::default() }.largest_cell(2048), 4);
    }

    #[test]
    fn hostile_sizes_stay_within_the_largest_output_and_the_coarsest_cell() {
        let huge = OutputResolution { mode: ResolutionMode::Custom, custom: u32::MAX, factor: i8::MAX };
        assert_eq!(huge.output_size(2048, 2048), OutputResolution::MAX_SIZE);

        let tiny = OutputResolution { mode: ResolutionMode::Custom, custom: 1, factor: -3 };
        assert_eq!(tiny.cell(4096, 4096), OutputResolution::MAX_CELL);
    }
}
