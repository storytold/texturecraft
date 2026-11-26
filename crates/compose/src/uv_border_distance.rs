//! The UV Border Distance generator: how far each texel lies inside its UV island, as a gradient built once per UV layout and settings

use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_core::{BorderDistanceParameters, GeneratorGlobal};

use crate::eval::Inputs;
use crate::field_cache::FieldCache;
use crate::working_field::{REFERENCE_TILE_TEXELS, blur_blocks, box_radius};

/// How far in the gradient reaches at Distance 1, in UV units (fitted: about half a UV tile)
const REACH_PER_DISTANCE: f32 = 0.4936;

/// The blur at Smoothness 1, as a gaussian sigma in texels of a 2K UV tile (fitted)
const SMOOTHNESS_SIGMA_2K: f32 = 45.0;

static CACHE: FieldCache = FieldCache::new();

/// The gradient before balance, contrast and invert for every working texel of `inp` (0 outside the islands)
pub(crate) fn border_gradient(inp: &Inputs, geometry: u64, border: &BorderDistanceParameters) -> Arc<Vec<f32>> {
    CACHE.get_or_build(inp, geometry, &[], vec![border.distance.to_bits(), border.smoothness.to_bits()], |block| build(inp, border, block))
}

/// The value at a working texel: the gradient balanced, contrasted and inverted
pub(crate) fn border_value(border: &BorderDistanceParameters, gradient: f32) -> f32 {
    GeneratorGlobal { balance: border.balance, contrast: border.contrast, ..GeneratorGlobal::default() }.apply(gradient)
}

fn build(inp: &Inputs, border: &BorderDistanceParameters, block: (u32, u32)) -> Vec<f32> {
    let gbuf = inp.gbuf;
    let (width, height) = (gbuf.width, gbuf.height);

    // Every covered texel's distance to the nearest uncovered one, within its UV tile
    let distances = border_distances(gbuf, (width, height), block);

    let mut gradient: Vec<f32> = (0..width * height)
        .into_par_iter()
        .map(|texel| {
            if !gbuf.covered(texel as usize) {
                return 0.0;
            }

            let (x, y) = (texel % width, texel / width);
            let tile_texels = block.0 as f32 * inp.block_scales.at(x, y);
            let to_border = distances.get(texel as usize).copied().unwrap_or(0.0);
            let reach = border.distance * REACH_PER_DISTANCE * tile_texels;
            if reach > 0.0 { (1.0 - to_border / reach).clamp(0.0, 1.0) } else { 0.0 }
        })
        .collect();

    // Smoothness blurs the gradient with the empty space around the islands, so their borders soften towards black
    if border.smoothness > 0.0 {
        let radius_at = |tile_texels: f32| box_radius(border.smoothness * SMOOTHNESS_SIGMA_2K * tile_texels / REFERENCE_TILE_TEXELS);
        blur_blocks(&mut gradient, (width, height), block, |left, top| radius_at(block.0 as f32 * inp.block_scales.at(left, top)));
        for (value, texel) in gradient.iter_mut().zip(0..) {
            if !gbuf.covered(texel) {
                *value = 0.0;
            }
        }
    }
    gradient
}

/// Each texel's Euclidean distance in texels to the nearest uncovered texel of its UV tile block, the space past the block's edges counting as uncovered
pub(crate) fn border_distances(gbuf: &texturecraft_geom::UvGBuffer, (width, height): (u32, u32), block: (u32, u32)) -> Vec<f32> {
    let mut squared = vec![0.0f32; (width * height) as usize];
    for top in (0..height).step_by(block.1 as usize) {
        for left in (0..width).step_by(block.0 as usize) {
            let (columns, rows) = (block.0.min(width - left) as usize, block.1.min(height - top) as usize);
            let index = |column: usize, row: usize| (top as usize + row) * width as usize + left as usize + column;

            // A ring of uncovered texels around the block, then the exact transform along rows and columns
            let (padded_columns, padded_rows) = (columns + 2, rows + 2);
            let mut field: Vec<f32> = (0..padded_rows)
                .flat_map(|row| (0..padded_columns).map(move |column| (column, row)))
                .map(|(column, row)| {
                    let inside = (1..=columns).contains(&column) && (1..=rows).contains(&row);
                    if inside && gbuf.covered(index(column - 1, row - 1)) { f32::INFINITY } else { 0.0 }
                })
                .collect();
            field.par_chunks_mut(padded_columns).for_each(squared_distance_line);
            let mut transposed: Vec<f32> = (0..padded_columns * padded_rows)
                .map(|texel| field.get((texel % padded_rows) * padded_columns + texel / padded_rows).copied().unwrap_or(0.0))
                .collect();
            transposed.par_chunks_mut(padded_rows).for_each(squared_distance_line);

            for row in 0..rows {
                for column in 0..columns {
                    let value = transposed.get((column + 1) * padded_rows + row + 1).copied().unwrap_or(0.0);
                    if let Some(target) = squared.get_mut(index(column, row)) {
                        *target = value;
                    }
                }
            }
        }
    }
    squared.into_iter().map(f32::sqrt).collect()
}

/// The squared distance transform of one line in place (Felzenszwalb and Huttenlocher's lower envelope of parabolas); `line` holds 0 at sources and infinity elsewhere
fn squared_distance_line(line: &mut [f32]) {
    let length = line.len();
    let input = line.to_vec();
    let finite: Vec<usize> = (0..length).filter(|index| input.get(*index).is_some_and(|value| value.is_finite())).collect();
    if finite.is_empty() {
        return;
    }

    // Parabola vertices and the boundaries between them
    let value_at = |index: usize| input.get(index).copied().unwrap_or(f32::INFINITY);
    let intersection = |a: usize, b: usize| ((value_at(b) + (b * b) as f32) - (value_at(a) + (a * a) as f32)) / (2.0 * (b as f32 - a as f32));
    let mut vertices: Vec<usize> = Vec::with_capacity(finite.len());
    let mut boundaries: Vec<f32> = Vec::with_capacity(finite.len() + 1);
    for &index in &finite {
        while let (Some(&last), Some(&start)) = (vertices.last(), boundaries.last()) {
            if intersection(last, index) <= start {
                vertices.pop();
                boundaries.pop();
            } else {
                break;
            }
        }
        let start = vertices.last().map_or(f32::NEG_INFINITY, |&last| intersection(last, index));
        vertices.push(index);
        boundaries.push(start);
    }

    let mut current = 0;
    for (index, value) in line.iter_mut().enumerate() {
        while boundaries.get(current + 1).is_some_and(|next| *next < index as f32) {
            current += 1;
        }
        let vertex = vertices.get(current).copied().unwrap_or(index);
        let offset = index as f32 - vertex as f32;
        *value = offset * offset + value_at(vertex);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_reads_the_squared_distance_to_its_nearest_source() {
        let mut line = [0.0, f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY, 0.0, f32::INFINITY];
        squared_distance_line(&mut line);
        assert_eq!(line, [0.0, 1.0, 4.0, 4.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn a_line_without_sources_stays_unreached() {
        let mut line = [f32::INFINITY; 3];
        squared_distance_line(&mut line);
        assert!(line.iter().all(|value| value.is_infinite()));
    }
}
