//! Filters drawn from the distance to the shapes' edge: Bevel, Bevel Smooth and Directional Distance

use rayon::prelude::*;
use texturecraft_core::{Bevel, BevelBlur, BevelCorner, DirectionalDistance, Filter, lum};

use crate::distance_envelope::DistanceEnvelope;
use crate::eval::{MIN_ALPHA, blur};

/// From a texel's centre to the edge between it and the nearest texel across: half a texel short of their centres' distance
const EDGE_OFFSET: f32 = 0.5;

/// Components of a texel with color and coverage (red, green, blue, alpha)
const RGBA_COMPONENTS: usize = 4;

/// Color components of a texel, before its alpha
const COLOR_COMPONENTS: usize = 3;

/// Where a texel's alpha sits among its components
const ALPHA_INDEX: usize = 3;

/// What a texel becomes from its index, whether it is inside the shapes and its own value
type TexelResult<'a> = dyn Fn(usize, bool, f32) -> f32 + Sync + 'a;

/// Lay distance filter `filter` over premultiplied `data` (`width` × `height` texels of `components`; content's shapes from its luminance), `tile_resolution` texels across a UV tile, mixed in by `opacity`; its Distance Map `grays` (0..1 a texel) scales the distance texel by texel
pub(crate) fn distance_filter(
    data: &mut [f32],
    (cover, grays): (&[f32], &[f32]),
    components: usize,
    (width, height): (usize, usize),
    filter: &Filter,
    tile_resolution: u32,
    opacity: f32,
) {
    let map = |texel: usize| grays.get(texel).copied().unwrap_or(1.0);
    let value = |pixel: &[f32]| match pixel {
        [r, g, b, alpha] if *alpha > MIN_ALPHA => lum([r / alpha, g / alpha, b / alpha]),
        [value] => *value,
        _ => 0.0,
    };
    let shapes = |inside: &(dyn Fn(f32) -> bool + Sync)| -> Vec<bool> {
        data.par_chunks(components).zip(cover.par_iter()).map(|(pixel, covered)| *covered > 0.0 && inside(value(pixel))).collect()
    };
    let smoothed = |field: &mut [f32], texels: f32| {
        let radius = texels.round() as usize;
        if radius > 0 {
            blur(field, width, height, radius);
        }
    };

    // Per texel: whether it is inside the shapes, and what it becomes from its own value
    let (inside, result): (Vec<bool>, Box<TexelResult<'_>>) = match filter {
        Filter::DistanceBevel(bevel) => {
            let inside = shapes(&|value| value >= Bevel::EDGE);
            let edge = edge_distances(&inside, (width, height), bevel.corner == BevelCorner::Angular);
            let reach = bevel.reach(tile_resolution);
            let mut slope: Vec<f32> = inside.par_iter().zip(edge.par_iter()).map(|(inside, edge)| bevel.slope(*inside, *edge, reach)).collect();
            drop(edge);

            let smoothing_radius = bevel.smoothing_texels(tile_resolution).round() as usize;
            if smoothing_radius > 0 {
                match bevel.blur {
                    BevelBlur::Uniform => blur(&mut slope, width, height, smoothing_radius),
                    BevelBlur::NonUniform => {
                        let mut blurred = slope.clone();
                        blur(&mut blurred, width, height, smoothing_radius);

                        // Keep the foot sharp and blur the top at full strength
                        slope.par_iter_mut().zip(blurred).for_each(|(sharp, blurred)| *sharp += (blurred - *sharp) * *sharp);
                    }
                }
            }

            let bevel = *bevel;
            (inside, Box::new(move |texel, inside, value| bevel.apply(inside, value, slope.get(texel).copied().unwrap_or(1.0))))
        }
        Filter::BevelSmooth(smooth) => {
            let inside = shapes(&|value| smooth.inside(value));
            let edge = edge_distances(&inside, (width, height), false);
            let (reach, curve) = (smooth.reach(tile_resolution), smooth.curve());
            let mut ramp: Vec<f32> = inside
                .par_iter()
                .zip(edge.par_iter())
                .enumerate()
                .map(|(texel, (inside, edge))| smooth.value(curve, *inside, *edge, reach * map(texel)))
                .collect();
            drop(edge);
            smoothed(&mut ramp, smooth.smoothing_texels(tile_resolution));
            (inside, Box::new(move |texel, _, value| ramp.get(texel).copied().unwrap_or(value)))
        }
        Filter::DirectionalDistance(trail) => {
            let inside = shapes(&|value| value >= DirectionalDistance::EDGE);
            let along = trail_distances(&inside, (width, height), crate::eval::direction(trail.angle), trail.reach(tile_resolution));
            let trail = *trail;

            // A share of the full reach is a larger share of a texel's shorter one; no trail where the map is black, the shapes stay
            let shortened = move |texel: usize| {
                along.get(texel).copied().flatten().and_then(|along| match map(texel) {
                    _ if along <= 0.0 => Some(0.0),
                    gray if gray > 0.0 => Some(along / gray),
                    _ => None,
                })
            };
            (inside, Box::new(move |texel, _, _| trail.value(shortened(texel).filter(|along| *along <= 1.0))))
        }
        _ => return,
    };

    let has_alpha = components == RGBA_COMPONENTS;
    data.par_chunks_mut(components).zip(inside.par_iter()).zip(cover.par_iter()).enumerate().for_each(|(texel, ((pixel, inside), covered))| {
        if *covered <= 0.0 {
            return;
        }

        // Content is premultiplied by its coverage, which stays
        let coverage = if has_alpha { pixel.get(ALPHA_INDEX).copied().unwrap_or(0.0) } else { 1.0 };
        for slot in pixel.iter_mut().take(components.min(COLOR_COMPONENTS)) {
            let before = if has_alpha { if coverage > MIN_ALPHA { *slot / coverage } else { 0.0 } } else { *slot };
            let after = result(texel, *inside, before);
            *slot = (before + (after - before) * opacity) * coverage;
        }
    });
}

/// Every texel's distance (texels) to the edge between it and the nearest texel on the other side of the shapes `inside`: straight-line, or the larger axis distance when `angular`
pub fn edge_distances(inside: &[bool], (width, height): (usize, usize), angular: bool) -> Vec<f32> {
    let distances = if angular { chessboard_distances } else { straight_distances };
    let mut to_inside = distances(inside, (width, height), true);
    let to_outside = distances(inside, (width, height), false);

    to_inside
        .par_iter_mut()
        .zip(to_outside)
        .zip(inside)
        .for_each(|((to_inside, to_outside), inside)| *to_inside = (if *inside { to_outside } else { *to_inside } - EDGE_OFFSET).max(0.0));

    to_inside
}

/// Each texel's straight-line distance to the nearest texel on the requested side.
fn straight_distances(inside: &[bool], (width, height): (usize, usize), target_side: bool) -> Vec<f32> {
    // Start beyond the region so a line without a feature stays out of reach
    let far = ((width + height) as f32).powi(2);
    let mut rows: Vec<f32> = inside.iter().map(|inside| if *inside == target_side { 0.0 } else { far }).collect();
    rows.par_chunks_mut(width.max(1)).for_each_init(DistanceEnvelope::default, |envelope, row| envelope.lower(row));

    let mut columns: Vec<f32> = (0..width * height).map(|at| rows.get((at % height.max(1)) * width + at / height.max(1)).copied().unwrap_or(far)).collect();
    columns.par_chunks_mut(height.max(1)).for_each_init(DistanceEnvelope::default, |envelope, column| envelope.lower(column));

    // Reuse the row buffer for the final distances in row-major order
    rows.resize(width * height, far);
    for (index, distance) in rows.iter_mut().enumerate() {
        *distance = columns.get((index % width.max(1)) * height + index / width.max(1)).copied().unwrap_or(far).sqrt();
    }

    rows
}

/// Each texel's distance to the nearest texel on the requested side, measured along the larger axis.
fn chessboard_distances(inside: &[bool], (width, height): (usize, usize), target_side: bool) -> Vec<f32> {
    let far = (width + height) as f32;
    let mut distances: Vec<f32> = inside.iter().map(|inside| if *inside == target_side { 0.0 } else { far }).collect();

    // One pass down from the texels above and left, one up from those below and right
    let step = |distances: &mut [f32], (x, y): (usize, usize), neighbours: [(isize, isize); 4]| {
        let nearest = neighbours
            .iter()
            .filter_map(|(dx, dy)| {
                let (neighbour_x, neighbour_y) = (x.checked_add_signed(*dx)?, y.checked_add_signed(*dy)?);
                (neighbour_x < width && neighbour_y < height).then(|| distances.get(neighbour_y * width + neighbour_x).copied()).flatten()
            })
            .fold(f32::MAX, f32::min);
        if let Some(slot) = distances.get_mut(y * width + x) {
            *slot = slot.min(nearest + 1.0);
        }
    };
    for y in 0..height {
        for x in 0..width {
            step(&mut distances, (x, y), [(-1, 0), (-1, -1), (0, -1), (1, -1)]);
        }
    }
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            step(&mut distances, (x, y), [(1, 0), (1, 1), (0, 1), (-1, 1)]);
        }
    }
    distances
}

/// For every texel, how far (a share of `reach` texels) the nearest texel of the shapes `inside` lies behind it against `direction`, if one is in reach
pub fn trail_distances(inside: &[bool], (width, height): (usize, usize), direction: [f32; 2], reach: f32) -> Vec<Option<f32>> {
    let steps = trail_steps((width, height), direction, reach);

    // Each row of shapes as bits, a row without one as no words: steps landing on it find nothing
    let shapes: Vec<Vec<u64>> = inside.par_chunks(width.max(1)).map(row_bits).collect();

    let mut distances = vec![None; width * height];
    distances.par_chunks_mut(width.max(1)).enumerate().for_each(|(y, row)| trail_row(row, y, &shapes, &steps));
    distances
}

/// The texel steps back along a trail against `direction`, nearest first, each with its share of `reach`: texel coordinates are whole, so every texel takes the same steps
fn trail_steps((width, height): (usize, usize), direction: [f32; 2], reach: f32) -> Vec<(isize, isize, f32)> {
    // Steps of one texel along the direction's longer axis visit every column (or row) the trail crosses, so no thin shape is stepped over
    let longer_axis = direction[0].abs().max(direction[1].abs());
    let samples = if reach > 0.0 && longer_axis > 0.0 { (reach * longer_axis).ceil() as usize } else { 0 };
    let step = if samples > 0 { reach / samples as f32 } else { 0.0 };

    let mut steps: Vec<(isize, isize, f32)> = Vec::new();
    for sample in 0..=samples {
        let back = sample as f32 * step;

        // Rounded half up, so a whole texel coordinate added to it rounds the same; a NaN direction lands nowhere
        let (across, down) = ((0.5 - direction[0] * back).floor(), (0.5 - direction[1] * back).floor());
        if !(across.is_finite() && down.is_finite()) {
            break;
        }

        // A step as long as the region leaves it from every texel, and never comes back in
        let (across, down) = (across as isize, down as isize);
        if across.unsigned_abs() >= width || down.unsigned_abs() >= height {
            break;
        }

        // Rounding puts several samples on one texel: the nearest counts
        if steps.last().is_some_and(|(last_across, last_down, _)| (*last_across, *last_down) == (across, down)) {
            continue;
        }

        steps.push((across, down, if reach > 0.0 { back / reach } else { 0.0 }));
    }
    steps
}

/// A row of shapes as bits, [`u64::BITS`] texels a word; no words when it has none
fn row_bits(row: &[bool]) -> Vec<u64> {
    if !row.contains(&true) {
        return Vec::new();
    }

    let mut words = vec![0u64; row.len().div_ceil(WORD_BITS)];
    for (word, texels) in words.iter_mut().zip(row.chunks(WORD_BITS)) {
        *word = texels.iter().enumerate().filter(|(_, inside)| **inside).fold(0, |bits, (bit, _)| bits | 1 << bit);
    }
    words
}

/// Texels in a word of a row's bits
const WORD_BITS: usize = u64::BITS as usize;

/// Row `y`'s trail distances into `row`: a step at a time, every texel the step brings onto a shape takes its share, unless an earlier step did
fn trail_row(row: &mut [Option<f32>], y: usize, shapes: &[Vec<u64>], steps: &[(isize, isize, f32)]) {
    let width = row.len();
    let mut found = vec![0u64; width.div_ceil(WORD_BITS)];
    let mut unfound = width;

    // The last word's bits past the row's end are no texels
    let last_word = found.len().saturating_sub(1);
    let tail = width % WORD_BITS;
    let last_word_texels = if tail > 0 { (1u64 << tail) - 1 } else { u64::MAX };

    for (across, down, share) in steps {
        // A step past the region's top or bottom leaves it for good
        let Some(source) = y.checked_add_signed(*down).and_then(|source_y| shapes.get(source_y)) else { break };
        if source.is_empty() {
            continue;
        }

        for (word, found_word) in found.iter_mut().enumerate() {
            let in_row = if word == last_word { last_word_texels } else { u64::MAX };
            let landed = bits_from(source, (word * WORD_BITS) as isize + across) & !*found_word & in_row;
            *found_word |= landed;

            let mut remaining = landed;
            while remaining != 0 {
                let bit = remaining.trailing_zeros() as usize;
                remaining &= remaining - 1;
                if let Some(slot) = row.get_mut(word * WORD_BITS + bit) {
                    *slot = Some(*share);
                }
                unfound = unfound.saturating_sub(1);
            }
        }

        if unfound == 0 {
            break;
        }
    }
}

/// The [`WORD_BITS`] bits of row bits `source` from texel `start` on, none past either end
fn bits_from(source: &[u64], start: isize) -> u64 {
    let bits = WORD_BITS as isize;
    let (word, shift) = (start.div_euclid(bits), start.rem_euclid(bits) as u32);
    let at = |index: isize| usize::try_from(index).ok().and_then(|index| source.get(index)).copied().unwrap_or(0);
    let low = at(word) >> shift;
    if shift == 0 { low } else { low | at(word + 1) << (u64::BITS - shift) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_straight_or_chessboard() {
        // One feature texel in the middle of a 5 × 5 region
        let mut feature = vec![false; 25];
        feature[12] = true;
        let straight = straight_distances(&feature, (5, 5), true);
        let chessboard = chessboard_distances(&feature, (5, 5), true);
        assert_eq!((straight[12], straight[13], chessboard[13]), (0.0, 1.0, 1.0));
        assert!((straight[0] - 8f32.sqrt()).abs() < 1e-5, "{}", straight[0]);
        assert_eq!(chessboard[0], 2.0, "a corner is as far as its larger axis");
        let none = straight_distances(&[false; 4], (2, 2), true);
        assert!(none.iter().all(|distance| *distance >= 4.0), "no feature is beyond the region");
    }

    #[test]
    fn distances_keep_rows_and_columns_apart_in_a_region_wider_than_tall() {
        // One feature texel at column 1, row 0 of a 7 × 3 region
        let (width, height) = (7, 3);
        let mut feature = vec![false; width * height];
        feature[1] = true;
        let straight = straight_distances(&feature, (width, height), true);
        let chessboard = chessboard_distances(&feature, (width, height), true);

        let at = |x: usize, y: usize| y * width + x;
        assert_eq!((straight[at(1, 0)], straight[at(6, 0)], straight[at(1, 2)]), (0.0, 5.0, 2.0));
        assert!((straight[at(6, 2)] - 29f32.sqrt()).abs() < 1e-5, "{}", straight[at(6, 2)]);
        assert_eq!((chessboard[at(6, 0)], chessboard[at(0, 2)], chessboard[at(6, 2)]), (5.0, 2.0, 5.0));
    }

    #[test]
    fn a_trail_finds_the_nearest_shape_behind_a_texel_as_a_share_of_its_reach() {
        // A shape at column 0 of a 5 × 2 region; the trail runs towards +x, so a texel looks back towards -x
        let (width, height) = (5, 2);
        let inside: Vec<bool> = (0..width * height).map(|texel| texel % width == 0).collect();
        let trail = trail_distances(&inside, (width, height), [1.0, 0.0], 4.0);

        assert_eq!(trail[0], Some(0.0), "on the shape");
        assert_eq!(trail[2], Some(0.5), "two texels behind, half the reach");
        assert_eq!(trail[width + 4], Some(1.0), "at the reach's end");
    }

    #[test]
    fn a_long_trail_steps_over_no_thin_shape() {
        // A one-texel shape at column 0 of an 8 × 1 region, with a reach far longer than the region
        let (width, height) = (8, 1);
        let inside: Vec<bool> = (0..width * height).map(|texel| texel == 0).collect();
        let reach = 128.0;

        let trail = trail_distances(&inside, (width, height), [1.0, 0.0], reach);

        assert_eq!(trail[3], Some(3.0 / reach), "three texels behind");
        assert!(trail.iter().all(Option::is_some), "every texel behind the shape is in its trail: {trail:?}");
    }

    /// The trail as every texel walks it on its own, one sample at a time: what the shared steps must match
    fn walked_trail_distances(inside: &[bool], (width, height): (usize, usize), direction: [f32; 2], reach: f32) -> Vec<Option<f32>> {
        let longer_axis = direction[0].abs().max(direction[1].abs());
        let samples = if reach > 0.0 && longer_axis > 0.0 { (reach * longer_axis).ceil() as usize } else { 0 };
        let step = if samples > 0 { reach / samples as f32 } else { 0.0 };
        (0..width * height)
            .into_par_iter()
            .map(|texel| {
                let (x, y) = ((texel % width) as f32, (texel / width) as f32);
                (0..=samples)
                    .map(|sample| {
                        let back = sample as f32 * step;
                        (back, (x - direction[0] * back + 0.5).floor(), (y - direction[1] * back + 0.5).floor())
                    })
                    .take_while(|(_, sample_x, sample_y)| *sample_x >= 0.0 && *sample_y >= 0.0 && (*sample_x as usize) < width && (*sample_y as usize) < height)
                    .find_map(|(back, sample_x, sample_y)| inside[sample_y as usize * width + sample_x as usize].then(|| back / reach))
            })
            .collect()
    }

    #[test]
    fn shared_trail_steps_find_what_each_texel_walking_its_own_finds() {
        // Sparse rows end partway through a word
        // Keep the region small enough to avoid rounding nearly tied rays into different texels
        let (width, height) = (150, 97);
        let mut seed = 12345u32;
        let inside: Vec<bool> = (0..width * height)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) % 97 < 2
            })
            .collect();

        for angle in [0.0, 17.0, 45.0, 90.0, 135.0, 200.0, 290.0, 333.0] {
            for reach in [9.3, 60.0, 400.0] {
                let direction = crate::eval::direction(angle);

                let shared = trail_distances(&inside, (width, height), direction, reach);

                let walked = walked_trail_distances(&inside, (width, height), direction, reach);
                assert!(walked.iter().any(Option::is_some) && walked.iter().any(Option::is_none), "{angle}° {reach}: a field with texels in and out of reach");
                assert_eq!(shared, walked, "{angle}° reach {reach}");
            }
        }
    }
}
