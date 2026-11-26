//! Whole-map fields some generators precompute at the working resolution (UV Border Distance, Dripping Rust), built once per layout, mesh map and settings

use crate::eval::{Inputs, blur};

/// The resolution of a UV tile the generators' fitted sizes are measured at
pub(crate) const REFERENCE_TILE_TEXELS: f32 = 2048.0;

/// The size of one UV tile's block in `inp`'s working atlas
pub(crate) fn working_block(inp: &Inputs) -> (u32, u32) {
    let size = (inp.gbuf.width, inp.gbuf.height);
    let grid = inp.project.texture_set(inp.ts).ok().and_then(|set| set.tile_grid());
    let block = grid.map_or(size, |grid| (size.0 / grid.cols.max(1), size.1 / grid.rows.max(1)));
    (block.0.clamp(1, size.0.max(1)), block.1.clamp(1, size.1.max(1)))
}

/// The radius of three box passes that blur like a gaussian of `sigma`: three boxes of radius r spread a texel with variance r (r + 1)
pub(crate) fn box_radius(sigma: f32) -> usize {
    (((1.0 + 4.0 * sigma * sigma).sqrt() - 1.0) * 0.5).round() as usize
}

/// `data` blurred one UV tile block at a time by the radius `radius_at` gives for the block's corner, reading black past each block's edges
pub(crate) fn blur_blocks(data: &mut [f32], (width, height): (u32, u32), block: (u32, u32), radius_at: impl Fn(u32, u32) -> usize) {
    for top in (0..height).step_by(block.1 as usize) {
        for left in (0..width).step_by(block.0 as usize) {
            let radius = radius_at(left, top);
            if radius == 0 {
                continue;
            }

            let (columns, rows) = (block.0.min(width - left) as usize, block.1.min(height - top) as usize);
            let index = |column: usize, row: usize| (top as usize + row) * width as usize + left as usize + column;
            let mut values: Vec<f32> = (0..rows)
                .flat_map(|row| (0..columns).map(move |column| (column, row)))
                .map(|(column, row)| data.get(index(column, row)).copied().unwrap_or(0.0))
                .collect();
            blur(&mut values, columns, rows, radius);
            for (offset, value) in values.into_iter().enumerate() {
                if let Some(target) = data.get_mut(index(offset % columns, offset / columns)) {
                    *target = value;
                }
            }
        }
    }
}
