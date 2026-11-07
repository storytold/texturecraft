//! The baked curvature map at the detail sizes of curvature generators, built once per map and layout.

use std::sync::{Arc, Mutex, Weak};

use rayon::prelude::*;
use texturecraft_core::{BakedMap, BlockScales, CurvatureDetail};
use texturecraft_texel::dilate::{DilationMap, NONE};

use crate::curvature_size_level::{CurvatureSizeLevel, DEVIATION_SCALE};
use crate::eval::{Inputs, blur};

/// The curvature map padded past its UV islands and blurred at each size of [`CurvatureDetail::SIZES`] after the first.
pub struct CurvatureSizes {
    pub levels: Vec<CurvatureSizeLevel>,
}

/// Built sizes by the curvature data they come from, the working texture size, the UV tile block and the tiles' scales.
type CacheEntry = (Weak<Vec<f32>>, (u32, u32), (u32, u32), BlockScales, Arc<CurvatureSizes>);

static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

impl CurvatureSizes {
    /// The sizes of the texture set `inp` composites (None without a baked curvature map).
    pub fn of(inp: &Inputs) -> Option<Arc<CurvatureSizes>> {
        let curvature = inp.baked.maps.get(&BakedMap::Curvature)?;
        let (width, height) = (inp.baked.resolution, inp.baked.height(BakedMap::Curvature));
        let grid = inp.project.texture_set(inp.ts).ok().and_then(|set| set.tile_grid());
        let block = grid.map_or((width, height), |grid| (width / grid.cols.max(1), height / grid.rows.max(1)));
        let working_size = (inp.gbuf.width, inp.gbuf.height);

        let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.retain(|(source, ..)| source.strong_count() > 0);
        let same_source = |source: &Weak<Vec<f32>>| source.upgrade().is_some_and(|data| Arc::ptr_eq(&data, curvature));
        let cached = cache.iter().find(|(source, cached_size, cached_block, cached_scales, _)| {
            same_source(source) && *cached_size == working_size && *cached_block == block && *cached_scales == inp.block_scales
        });
        if let Some((.., sizes)) = cached {
            return Some(sizes.clone());
        }

        // Covered working texels mark where they land in the bake atlas
        let mut covered = vec![false; (width * height) as usize];
        for texel in (0..working_size.0 * working_size.1).filter(|texel| inp.gbuf.covered(*texel as usize)) {
            let uv = [((texel % working_size.0) as f32 + 0.5) / working_size.0 as f32, ((texel / working_size.0) as f32 + 0.5) / working_size.1 as f32];
            if let Some(baked_uv) = inp.block_scales.bake_uv(uv, working_size) {
                let (baked_x, baked_y) = ((baked_uv[0] * width as f32) as u32, (baked_uv[1] * height as f32) as u32);
                if let Some(slot) = covered.get_mut((baked_y.min(height - 1) * width + baked_x.min(width - 1)) as usize) {
                    *slot = true;
                }
            }
        }

        let sizes = Arc::new(CurvatureSizes::build(curvature, (width, height), &covered, block));
        cache.push((Arc::downgrade(curvature), working_size, block, inp.block_scales.clone(), sizes.clone()));
        Some(sizes)
    }

    /// Pad `curvature` from its `covered` texels within each UV tile `block`, then blur each block at every size.
    fn build(curvature: &[f32], (width, height): (u32, u32), covered: &[bool], block: (u32, u32)) -> CurvatureSizes {
        let padding = DilationMap::in_rectangular_blocks(covered, width, height, None, block);
        let deviation: Vec<f32> =
            padding.src.par_iter().map(|&source| if source == NONE { 0.0 } else { curvature.get(source as usize).copied().unwrap_or(0.5) - 0.5 }).collect();
        let (block_width, block_height) = (block.0.max(1).min(width), block.1.max(1).min(height));

        let levels = CurvatureDetail::SIZES
            .iter()
            .skip(1)
            .map(|size| {
                // Sizes are texels at 2K; a blur this wide holds nothing finer than half its radius, so it is stored that much coarser
                let radius = (size * block_width as f32 / 2048.0).round().max(1.0) as u32;
                let step = (radius / 2).max(1).next_power_of_two();

                let mut blurred = deviation.clone();
                for block_y in (0..height).step_by(block_height as usize) {
                    for block_x in (0..width).step_by(block_width as usize) {
                        let region = (block_x, block_y, block_width.min(width - block_x), block_height.min(height - block_y));
                        blur_block(&mut blurred, width, region, radius);
                    }
                }
                coarsen(&blurred, (width, height), step)
            })
            .collect();
        CurvatureSizes { levels }
    }
}

/// Blur the `(x0, y0, w, h)` block of `data` (`width` texels per row) in place, reading nothing outside it.
fn blur_block(data: &mut [f32], width: u32, (x0, y0, w, h): (u32, u32, u32, u32), radius: u32) {
    let mut block: Vec<f32> =
        (y0..y0 + h).flat_map(|y| (x0..x0 + w).map(move |x| (y * width + x) as usize)).map(|index| data.get(index).copied().unwrap_or(0.0)).collect();
    blur(&mut block, w as usize, h as usize, radius as usize);

    for (row, values) in block.chunks(w as usize).enumerate() {
        let start = ((y0 + row as u32) * width + x0) as usize;
        if let Some(target) = data.get_mut(start..start + values.len()) {
            target.copy_from_slice(values);
        }
    }
}

/// `data` averaged over `step × step` squares (the last ones clipped to the map).
fn coarsen(data: &[f32], (width, height): (u32, u32), step: u32) -> CurvatureSizeLevel {
    let (coarse_width, coarse_height) = (width.div_ceil(step), height.div_ceil(step));
    let deviation = (0..coarse_width * coarse_height)
        .into_par_iter()
        .map(|cell| {
            let (cell_x, cell_y) = (cell % coarse_width, cell / coarse_width);
            let columns = cell_x * step..((cell_x + 1) * step).min(width);
            let rows = cell_y * step..((cell_y + 1) * step).min(height);
            let count = (columns.len() * rows.len()).max(1) as f32;
            let texels = rows.flat_map(|y| columns.clone().map(move |x| (y * width + x) as usize));
            let mean = texels.map(|index| data.get(index).copied().unwrap_or(0.0)).sum::<f32>() / count;
            (mean * DEVIATION_SCALE).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect();
    CurvatureSizeLevel { step, width: coarse_width, height: coarse_height, deviation }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_keeps_an_island_border_from_reading_flat() {
        let left_half = |texel: u32| texel % 64 < 32;
        let curvature: Vec<f32> = (0..64 * 64).map(|texel| if left_half(texel) { 1.0 } else { 0.0 }).collect();
        let covered: Vec<bool> = (0..64 * 64).map(left_half).collect();
        let sizes = CurvatureSizes::build(&curvature, (64, 64), &covered, (64, 64));
        for level in &sizes.levels {
            assert!((level.sample(31, 32) - 0.5).abs() < 0.01, "the border keeps the island's curvature, not the empty texels beside it");
        }
    }
}
