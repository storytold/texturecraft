//! UV padding across seams (3D): texels past a UV seam continue the surface from the seam's other side

use std::collections::HashMap;

use texturecraft_geom::glam::{IVec2, Vec2};
use texturecraft_texel::dilate::{DilationMap, NONE};

use crate::SeamPaddingInput;
use crate::texel_edge::TexelEdge;

/// Farthest a padding texel reaches across a seam, in texels; farther ones keep their nearest covered texel
pub const SEAM_REACH: u32 = 32;

/// The padding texels of `flat` just outside a seam, each with the texel as far inside the seam's other side: (texel, source) indices sorted by texel
pub(crate) fn cross_seams(flat: &DilationMap, reach: u32, block: (u32, u32), input: &SeamPaddingInput<'_>) -> Vec<(u32, u32)> {
    let (width, height) = (flat.width, flat.height);
    let size = Vec2::new(width as f32, height as f32);
    let block_of = |x: u32, y: u32| (x.checked_div(block.0).unwrap_or(0), y.checked_div(block.1).unwrap_or(0));

    // Each crossed texel's source and how far outside its seam it is; a nearer seam replaces it
    let mut crossed: HashMap<u32, (u32, f32)> = HashMap::new();
    for seam in &input.seams {
        for (near, far) in [(seam.sides[0], seam.sides[1]), (seam.sides[1], seam.sides[0])] {
            let (Some(near_edge), Some(far_edge)) = (TexelEdge::new(near, size), TexelEdge::new(far, size)) else {
                continue;
            };

            let reach = reach as f32;
            let low = near_edge.start.min(near_edge.end) - Vec2::splat(reach);
            let high = near_edge.start.max(near_edge.end) + Vec2::splat(reach);
            let (x0, y0) = (low.x.floor().max(0.0) as u32, low.y.floor().max(0.0) as u32);
            let (x1, y1) = (high.x.ceil().min(size.x) as u32, high.y.ceil().min(size.y) as u32);
            let mut side_islands = None;
            for y in y0..y1 {
                // Only the texel centres in the band beside the seam can be within its reach
                let Some((low_x, high_x)) = near_edge.band_span(reach, y as f32 + 0.5) else { continue };
                let row_x0 = ((low_x - 0.5).floor().max(0.0) as u32).max(x0);
                let row_x1 = ((high_x + 0.5).ceil().max(0.0) as u32).min(x1);
                for x in row_x0..row_x1 {
                    let index = y * width + x;
                    let Some(&flat_source) = flat.src.get(index as usize) else { continue };

                    if flat_source == NONE || flat_source == index {
                        continue;
                    }

                    let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let Some((along, outside)) = near_edge.outside(point) else { continue };

                    if outside > reach || crossed.get(&index).is_some_and(|(_, distance)| outside >= *distance) {
                        continue;
                    }

                    // Another island's edge nearer than the seam keeps its texel
                    let (source_x, source_y) = (flat_source % width, flat_source / width);
                    if (source_x.abs_diff(x).max(source_y.abs_diff(y)) as f32) + 1.0 < outside {
                        continue;
                    }

                    // Padding stays inside its UV tile, like the 2D padding
                    let inside = near_edge.start.lerp(near_edge.end, along) + near_edge.inward * 0.5;
                    if block_of(inside.x.max(0.0) as u32, inside.y.max(0.0) as u32) != block_of(x, y) {
                        continue;
                    }

                    // Start at the first texel centres on the seam's far side
                    let far_boundary = far_edge.start.lerp(far_edge.end, along);
                    let start = far_boundary + far_edge.inward * 0.5;
                    let target = far_boundary + far_edge.inward * outside.max(0.5);
                    if target.x < 0.0 || target.y < 0.0 || target.x >= size.x || target.y >= size.y {
                        continue;
                    }

                    let target_index = target.y as u32 * width + target.x as u32;
                    if flat.src.get(target_index as usize) != Some(&target_index) {
                        continue;
                    }

                    let (Some(near_island), Some(far_island)) =
                        *side_islands.get_or_insert_with(|| (input.triangle_island(near.triangle), input.triangle_island(far.triangle)))
                    else {
                        continue;
                    };

                    if input.island_at(flat_source) != Some(near_island) {
                        continue;
                    }

                    if input.island_at(target_index) == Some(far_island) && follows_island(input, start, target, far_island, (width, height), block) {
                        crossed.insert(index, (target_index, outside));
                    }
                }
            }
        }
    }

    let mut sources: Vec<(u32, u32)> = crossed.into_iter().map(|(texel, (source, _))| (texel, source)).collect();
    sources.sort_unstable();
    sources
}

fn follows_island(input: &SeamPaddingInput<'_>, start: Vec2, target: Vec2, island: usize, (width, height): (u32, u32), block: (u32, u32)) -> bool {
    let mut cell = start.floor().as_ivec2();
    let end = target.floor().as_ivec2();
    let limit = IVec2::new(width as i32, height as i32);
    if cell.cmplt(IVec2::ZERO).any() || cell.cmpge(limit).any() {
        return false;
    }

    if block.0 != 0 && block.1 != 0 {
        let block = IVec2::new(block.0 as i32, block.1 as i32);
        if cell / block != end / block {
            return false;
        }
    }

    let step = (end - cell).signum();
    let delta = (target - start).abs();
    let step_time = Vec2::new(1.0 / delta.x, 1.0 / delta.y);
    let boundary_time = |axis: usize| {
        if step[axis] == 0 {
            return f32::INFINITY;
        }

        let boundary = cell[axis] + i32::from(step[axis] > 0);
        (boundary as f32 - start[axis]).abs() * step_time[axis]
    };
    let mut next = Vec2::new(boundary_time(0), boundary_time(1));

    // Visit every texel with a nonzero stretch of the path
    let mut last_triangle = None;
    loop {
        let Some(triangle) = input.triangle_at(cell.y as u32 * width + cell.x as u32) else { return false };

        if Some(triangle) != last_triangle {
            if input.triangle_island(triangle) != Some(island) {
                return false;
            }

            last_triangle = Some(triangle);
        }

        if cell == end {
            return true;
        }

        let time = next.x.min(next.y);
        if next.x == time {
            cell.x += step.x;
            next.x = if cell.x == end.x { f32::INFINITY } else { next.x + step_time.x };
        }
        if next.y == time {
            cell.y += step.y;
            next.y = if cell.y == end.y { f32::INFINITY } else { next.y + step_time.y };
        }
    }
}
