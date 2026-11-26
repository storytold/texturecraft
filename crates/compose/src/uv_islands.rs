//! The UV island generators (UV Random Color, UV Checker) at every working texel, built once per layout and settings

use std::collections::HashMap;
use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_core::{GeneratorKind, UvCheckerParameters, UvRandomColorParameters};
use texturecraft_geom::{NO_TRI, TrianglePartition};
use texturecraft_texel::linear_to_srgb;
use texturecraft_texel::noise::cell_hash;

use crate::eval::{Inputs, texel_coords};
use crate::field_cache::FieldCache;
use crate::uv_border_distance::border_distances;
use crate::working_field::REFERENCE_TILE_TEXELS;

static CACHE: FieldCache = FieldCache::new();

/// UV Random Color at every working texel of `inp`: one value per UV island (0 outside the islands)
pub(crate) fn random_color_field(inp: &Inputs, geometry: u64, random: &UvRandomColorParameters, seed: u32) -> Arc<Vec<f32>> {
    let mut settings = vec![GeneratorKind::UvRandomColor as u32, seed, u32::from(random.custom_gradient)];
    settings.push(random.used_colors().len() as u32);
    settings.extend(random.used_colors().iter().flat_map(|color| color.map(f32::to_bits)));
    settings.extend(random.positions.iter().map(|position| position.to_bits()));

    CACHE.get_or_build(inp, geometry, &[], settings, |_| {
        let island_values = island_values(inp, |island| random_gray(random, seed, island));
        let gbuf = inp.gbuf;
        gbuf.tri.par_iter().map(|triangle| island_values.get(triangle).copied().unwrap_or(0.0)).collect()
    })
}

/// The value `value_of` gives each UV island, by triangle
fn island_values(inp: &Inputs, value_of: impl Fn(u32) -> f32) -> HashMap<u32, f32> {
    let material = inp.project.texture_set(inp.ts).map_or(0, |set| set.material);
    let islands = TrianglePartition::uv_islands(inp.mesh, material);
    islands
        .groups
        .iter()
        .enumerate()
        .flat_map(|(island, triangles)| {
            let value = value_of(island as u32);
            triangles.iter().map(move |triangle| (*triangle, value))
        })
        .collect()
}

/// An island's gray: the mean of a random color's channels, or of the gradient's color at a random point
fn random_gray(random: &UvRandomColorParameters, seed: u32, island: u32) -> f32 {
    let island = island as i32;
    let color = if random.custom_gradient {
        random.at(cell_hash(island, 0, 0, seed)).unwrap_or([0.0; 3])
    } else {
        std::array::from_fn(|channel| cell_hash(island, channel as i32 + 1, 0, seed))
    };
    color.iter().sum::<f32>() / 3.0
}

/// Checker squares across one grid tile, and grid lines across one square (fitted)
const SQUARES_PER_GRID_TILE: f32 = 4.0;
const MINOR_LINES_PER_SQUARE: f32 = 4.0;

/// The dark squares are the light color times this, on its sRGB-encoded values (fitted)
const DARK_SQUARE_SHADE: f32 = 0.7128;

/// What a full-opacity minor and major grid line multiply the color by (fitted)
const MINOR_LINE_SHADE: f32 = 0.8465;
const MAJOR_LINE_SHADE: f32 = 0.4230;

/// Texels of a 2K UV tile a grid line lies within of its square's edge, and the border's width per unit of UV Borders Width (fitted: 0.2 is 3 texels)
const LINE_HALF_WIDTH_2K: f32 = 1.0;
const BORDER_TEXELS_PER_WIDTH_2K: f32 = 15.0;

/// The saturation and value random island colors are picked within (measured)
const RANDOM_SATURATION: std::ops::RangeInclusive<f32> = 0.06..=0.57;
const RANDOM_VALUE: std::ops::RangeInclusive<f32> = 0.42..=0.80;

/// UV Checker at every working texel of `inp`: the gray of its color, the mean of the sRGB-encoded channels as the reference reads it (0 outside the islands)
pub(crate) fn checker_field(inp: &Inputs, geometry: u64, checker: &UvCheckerParameters, seed: u32) -> Arc<Vec<f32>> {
    let mut settings = vec![
        GeneratorKind::UvChecker as u32,
        seed,
        u32::from(checker.random_colors),
        checker.grid_tile,
        checker.grid_opacity.to_bits(),
        checker.border_width.to_bits(),
    ];
    settings.extend(checker.checker_color.iter().chain(&checker.border_color).map(|value| value.to_bits()));

    CACHE.get_or_build(inp, geometry, &[], settings, |block| {
        let gbuf = inp.gbuf;
        let (width, height) = (gbuf.width, gbuf.height);
        let island_colors = if checker.random_colors { island_colors(inp, seed) } else { HashMap::new() };
        let distances = (checker.border_width > 0.0).then(|| border_distances(gbuf, (width, height), block));
        let border_width_texels_2k = checker.border_width * BORDER_TEXELS_PER_WIDTH_2K;
        let light = checker.checker_color.map(linear_to_srgb);
        let border = checker.border_color.map(linear_to_srgb);

        (0..width * height)
            .into_par_iter()
            .map(|texel| {
                let Some(triangle) = gbuf.tri.get(texel as usize).copied().filter(|triangle| *triangle != NO_TRI) else { return 0.0 };

                let (x, y) = (texel % width, texel / width);
                let texels_2k = REFERENCE_TILE_TEXELS / (block.0 as f32 * inp.block_scales.at(x, y));

                // Measure from the texel footprint so coarse texels still show the border they cover
                let on_border = distances.as_ref().is_some_and(|distances| {
                    let distance = distances.get(texel as usize).copied().unwrap_or(0.0);
                    distance * texels_2k - footprint_beyond_one(texels_2k) <= border_width_texels_2k
                });
                let color = if on_border {
                    border
                } else {
                    let uv = texel_coords(inp, x, y).uv;
                    let light = island_colors.get(&triangle).copied().unwrap_or(light);
                    let at = (uv.x.rem_euclid(1.0) * REFERENCE_TILE_TEXELS, uv.y.rem_euclid(1.0) * REFERENCE_TILE_TEXELS);
                    checker_color(checker, light, at, texels_2k)
                };
                color.iter().sum::<f32>() / 3.0
            })
            .collect()
    })
}

/// How many 2K texels a working texel `texels_2k` of them wide covers beyond the one at its centre
fn footprint_beyond_one(texels_2k: f32) -> f32 {
    (texels_2k - 1.0).max(0.0)
}

/// The checker's sRGB-encoded color at (`x`, `y`) in texels of a 2K UV tile (row 0 at the top) for a texel `texels_2k` of them wide, with `light` its light squares
fn checker_color(checker: &UvCheckerParameters, light: [f32; 3], (x, y): (f32, f32), texels_2k: f32) -> [f32; 3] {
    let square = REFERENCE_TILE_TEXELS / (SQUARES_PER_GRID_TILE * checker.grid_tile.max(1) as f32);
    let is_light = ((x / square).floor() + (y / square).floor()).rem_euclid(2.0) >= 1.0;
    let base = if is_light { light } else { light.map(|channel| channel * DARK_SQUARE_SHADE) };

    // Line layers in the reference's order, each laid over the last at the grid's opacity
    let minor = square / MINOR_LINES_PER_SQUARE;

    // Include the texel footprint so thin grid lines survive coarse sampling
    let reach = LINE_HALF_WIDTH_2K + footprint_beyond_one(texels_2k) / 2.0;
    let on_line = |position: f32, period: f32| (position - (position / period).round() * period).abs() < reach;
    let layers = [
        (on_line(y, minor) && !on_line(y, square), MINOR_LINE_SHADE),
        (on_line(x, minor) && !on_line(x, square), MINOR_LINE_SHADE),
        (on_line(y, square), MAJOR_LINE_SHADE),
        (on_line(x, square), MAJOR_LINE_SHADE),
    ];
    let opacity = checker.grid_opacity.clamp(0.0, 1.0);
    let shade = layers.iter().filter(|(on, _)| *on).fold(1.0, |shade, (_, line)| shade + (line - shade) * opacity);
    base.map(|channel| channel * shade)
}

/// A random light sRGB-encoded color per UV island, by triangle
fn island_colors(inp: &Inputs, seed: u32) -> HashMap<u32, [f32; 3]> {
    let material = inp.project.texture_set(inp.ts).map_or(0, |set| set.material);
    let islands = TrianglePartition::uv_islands(inp.mesh, material);
    islands
        .groups
        .iter()
        .enumerate()
        .flat_map(|(island, triangles)| {
            let island = island as i32;
            let within = |range: &std::ops::RangeInclusive<f32>, random: f32| range.start() + (range.end() - range.start()) * random;
            let color = hsv_to_rgb(
                cell_hash(island, 0, 1, seed),
                within(&RANDOM_SATURATION, cell_hash(island, 1, 1, seed)),
                within(&RANDOM_VALUE, cell_hash(island, 2, 1, seed)),
            );
            triangles.iter().map(move |triangle| (*triangle, color))
        })
        .collect()
}

/// A color from its hue (in turns), saturation and value
fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let sector = hue.rem_euclid(1.0) * 6.0;
    let fraction = sector - sector.floor();
    let (low, falling, rising) = (value * (1.0 - saturation), value * (1.0 - saturation * fraction), value * (1.0 - saturation * (1.0 - fraction)));
    match sector.floor() as u32 {
        0 => [value, rising, low],
        1 => [falling, value, low],
        2 => [low, value, rising],
        3 => [low, falling, value],
        4 => [rising, low, value],
        _ => [value, low, falling],
    }
}
