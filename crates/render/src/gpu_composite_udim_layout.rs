//! The atlas of a UV-tiled texture set as the GPU program reads it (P8, `gpu_composite_udim.rs`).

use texturecraft_core::{TextureSet, TileGrid};

/// A UV-tiled set's atlas: its grid of blocks, each block's tile scale and the distinct scales
/// the neighbourhood stages run at, as the CPU's `Inputs::tile_res_at` and `filter_region` read
/// them. The working atlas is the set's size, so a block is `TextureSet::block_resolution` texels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UdimLayout {
    pub(crate) grid: TileGrid,
    /// Texels per side of an atlas block (the CPU's `tile_res` and `BlockScales` block)
    pub(crate) block: u32,
    /// The set's own resolution, the texture set size of `OutputResolution::output_size`
    pub(crate) texture_set: u32,
    /// The atlas's longest side, which caps filter radii as the CPU's `filter_extent` does
    pub(crate) longest_side: u32,
    /// A tile has a resolution of its own, so `TextureSet::from_atlas` undoes its scale
    pub(crate) scaled: bool,
    /// Each block's tile scale, row by row (`TextureSet::block_scales`)
    pub(crate) scales: Vec<f32>,
    /// The distinct block scales, largest first: level 0 is the base stage's scale
    pub(crate) levels: Vec<f32>,
}

impl UdimLayout {
    /// The layout of `set`, None for a classic set.
    pub(crate) fn of(set: &TextureSet) -> Option<Self> {
        let grid = set.tile_grid()?;
        let scales: Vec<f32> = (0..grid.rows).flat_map(|by| (0..grid.cols).map(move |bx| (bx, by))).map(|(bx, by)| set.tile_scale(grid.udim(bx, by))).collect();
        let mut levels = scales.clone();
        levels.sort_by(|a, b| b.total_cmp(a));
        levels.dedup();
        let (width, height) = set.size();
        Some(Self {
            grid,
            block: set.block_resolution(),
            texture_set: set.resolution,
            longest_side: width.max(height),
            scaled: !set.tile_resolutions.is_empty(),
            scales,
            levels,
        })
    }

    /// Texels per side of a UV tile filling `scale` of its block (`Inputs::tile_res_at`).
    pub(crate) fn tile_resolution(&self, scale: f32) -> u32 {
        ((self.block as f32 * scale).round() as u32).max(1)
    }

    /// The tile resolution of every level, the base stage's first.
    pub(crate) fn level_resolutions(&self) -> Vec<u32> {
        self.levels.iter().map(|&scale| self.tile_resolution(scale)).collect()
    }

    /// The level of each block, row by row.
    pub(crate) fn block_levels(&self) -> Vec<u32> {
        self.scales.iter().map(|scale| self.levels.iter().position(|level| level == scale).unwrap_or(0) as u32).collect()
    }
}
