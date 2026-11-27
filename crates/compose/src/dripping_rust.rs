//! The Dripping Rust generator: rust grown from the occlusion, broken up by grunge, then run down the mesh in drips; built once per layout and settings

use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_core::{BakedMap, DrippingRustParameters, GeneratorKind, WearParameters};
use texturecraft_geom::{NO_TRI, TrianglePartition};

use crate::eval::{Inputs, texel_coords};
use crate::field_cache::FieldCache;
use crate::histogram_scan::histogram_scan;
use crate::wear;
use crate::working_field::{REFERENCE_TILE_TEXELS, blur_blocks, box_radius};

/// The rust before any grunge: `RUST_FLOOR + RUST_GAIN * max(RUST_OCCLUSION − occlusion, 0)^RUST_POWER` (fitted to the reference)
const RUST_FLOOR: f32 = 0.315;
const RUST_GAIN: f32 = 3.19;
const RUST_OCCLUSION: f32 = 0.849;
const RUST_POWER: f32 = 2.38;

/// How much grunge breaks the rust up: the spread of the reference's noise (measured); the pattern itself is inferred
const GRUNGE_SPREAD: f32 = 0.19;

/// Blur sizes at Spreading Smoothness 1 and Drips Smoothness 1, as gaussian sigmas in texels of a 2K UV tile (fitted)
const SPREADING_SIGMA_2K: f32 = 47.0;
const DRIPS_SIGMA_2K: f32 = 11.5;

/// How far a drip runs at Drips Intensity 1, in texels of a 2K UV tile, and how it fades along its length: `1 − DRIP_FADE · along^DRIP_FADE_POWER` (fitted)
const DRIP_LENGTH_2K: f32 = 90.86;
const DRIP_FADE: f32 = 0.898;
const DRIP_FADE_POWER: f32 = 0.671;

/// Coverage below which a blurred texel has nothing to average
const MIN_COVERAGE: f32 = 1e-3;

static CACHE: FieldCache = FieldCache::new();

/// The rust before its spreading and contrast at every working texel of `inp` (0 outside the islands)
pub(crate) fn rust_field(inp: &Inputs, geometry: u64, rust: &DrippingRustParameters, seed: u32) -> Arc<Vec<f32>> {
    let settings = vec![
        rust.spreading_smoothness.to_bits(),
        rust.drips_intensity.to_bits(),
        rust.drips_smoothness.to_bits(),
        rust.drips_samples,
        u32::from(rust.axis),
        seed,
    ];
    CACHE.get_or_build(inp, geometry, &[BakedMap::AmbientOcclusion], settings, |block| build(inp, rust, seed, block))
}

/// The value at a working texel: the rust spread and contrasted
pub(crate) fn rust_value(rust: &DrippingRustParameters, field: f32) -> f32 {
    histogram_scan(field, rust.spreading, rust.contrast)
}

fn build(inp: &Inputs, rust: &DrippingRustParameters, seed: u32, block: (u32, u32)) -> Vec<f32> {
    let gbuf = inp.gbuf;
    let (width, height) = (gbuf.width, gbuf.height);
    let cover: Vec<f32> = (0..width * height).map(|texel| if gbuf.covered(texel as usize) { 1.0 } else { 0.0 }).collect();
    let tile_texels = |x: u32, y: u32| block.0 as f32 * inp.block_scales.at(x, y);

    // The rust from the occlusion, broken up by the Dirt generator's grunge
    let grunge = WearParameters::defaults(GeneratorKind::Dirt);
    let rust_at = |texel: u32| {
        let (x, y) = (texel % width, texel / width);
        let occlusion = inp.baked.sample_working(BakedMap::AmbientOcclusion, x, y, (width, height), &inp.block_scales).map_or(1.0, |value| value[0]);
        let base = RUST_FLOOR + RUST_GAIN * (RUST_OCCLUSION - occlusion).max(0.0).powf(RUST_POWER);
        let noise = grunge.map_or(0.0, |grunge| wear::grunge(GeneratorKind::Dirt, &grunge, seed, &texel_coords(inp, x, y)));
        base + GRUNGE_SPREAD * noise
    };
    let mut field: Vec<f32> = (0..width * height).into_par_iter().map(|texel| if gbuf.covered(texel as usize) { rust_at(texel) } else { 0.0 }).collect();

    let radius_at = |sigma_2k: f32, smoothness: f32| move |x: u32, y: u32| box_radius(smoothness * sigma_2k * tile_texels(x, y) / REFERENCE_TILE_TEXELS);
    blur_over_cover(&mut field, &cover, (width, height), block, radius_at(SPREADING_SIGMA_2K, rust.spreading_smoothness));
    let mut field = drips(inp, rust, field, (width, height), &tile_texels);
    blur_over_cover(&mut field, &cover, (width, height), block, radius_at(DRIPS_SIGMA_2K, rust.drips_smoothness));
    field.iter_mut().zip(&cover).for_each(|(value, covered)| *value = if *covered > 0.0 { value.clamp(0.0, 1.0) } else { 0.0 });
    field
}

/// `values` blurred within each UV tile block by the radius `radius_at` gives, weighted by `cover` so the empty space around the islands doesn't bleed in
fn blur_over_cover(values: &mut [f32], cover: &[f32], size: (u32, u32), block: (u32, u32), radius_at: impl Fn(u32, u32) -> usize + Copy) {
    let has_blur = (0..size.1).step_by(block.1 as usize).any(|top| (0..size.0).step_by(block.0 as usize).any(|left| radius_at(left, top) > 0));
    if !has_blur {
        return;
    }

    let mut sums: Vec<f32> = values.iter().zip(cover).map(|(value, covered)| value * covered).collect();
    let mut weights = cover.to_vec();
    blur_blocks(&mut sums, size, block, radius_at);
    blur_blocks(&mut weights, size, block, radius_at);
    values.par_iter_mut().zip(sums.par_iter().zip(weights.par_iter())).zip(cover.par_iter()).for_each(|((value, (sum, weight)), covered)| {
        if *covered > 0.0 && *weight > MIN_COVERAGE {
            *value = sum / weight;
        }
    });
}

/// Each texel takes the most rust found uphill along the drip axis, fading with how far up it lies
fn drips(
    inp: &Inputs,
    rust: &DrippingRustParameters,
    field: Vec<f32>,
    (width, height): (u32, u32),
    tile_texels: &(impl Fn(u32, u32) -> f32 + Sync),
) -> Vec<f32> {
    let gbuf = inp.gbuf;
    if rust.drips_samples == 0 || rust.drips_intensity <= 0.0 {
        return field;
    }

    let axis = usize::from(rust.axis.min(2));
    let (low, high) = inp.bounds;
    let low = low.to_array()[axis];
    let extent = (high.to_array()[axis] - low).max(f32::EPSILON);
    let material = inp.project.texture_set(inp.ts).map_or(0, |set| set.material);
    let islands = TrianglePartition::uv_islands(inp.mesh, material);
    let surface: Vec<(u32, f32)> = (0..width * height)
        .into_par_iter()
        .map(|texel| {
            let triangle = gbuf.tri.get(texel as usize).copied().unwrap_or(NO_TRI);
            let Some(island) = islands.index_of(triangle) else { return (NO_TRI, 0.0) };

            gbuf.surface(inp.mesh, texel as usize).map_or((NO_TRI, 0.0), |surface| (island as u32, (surface.pos.to_array()[axis] - low) / extent))
        })
        .collect();
    drop(islands);

    let index_at = |x: i64, y: i64| -> Option<usize> {
        if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
            return None;
        }

        Some((y * i64::from(width) + x) as usize)
    };
    let samples = rust.drips_samples;
    let taps: Vec<_> = (1..=samples)
        .map(|step| {
            let fraction = step as f32 / samples as f32;
            (fraction, 1.0 - DRIP_FADE * fraction.powf(DRIP_FADE_POWER))
        })
        .collect();

    (0..width * height)
        .into_par_iter()
        .map(|texel| {
            let (column, row) = (texel % width, texel / width);
            let (x, y) = (i64::from(column), i64::from(row));
            let here = field.get(texel as usize).copied().unwrap_or(0.0);
            let Some(&(island, axis_here)) = surface.get(texel as usize).filter(|(island, _)| *island != NO_TRI) else { return here };
            let axis_at = |x, y| {
                let (other_island, value) = *surface.get(index_at(x, y)?)?;
                (other_island == island).then_some(value)
            };

            let (dx, dy) = (slope(axis_at(x - 1, y), axis_here, axis_at(x + 1, y)), slope(axis_at(x, y - 1), axis_here, axis_at(x, y + 1)));
            let length = (dx * dx + dy * dy).sqrt();
            if !length.is_finite() || length <= f32::EPSILON {
                return here;
            }

            let reach = rust.drips_intensity * DRIP_LENGTH_2K * tile_texels(column, row) / REFERENCE_TILE_TEXELS;
            let mut most = here;
            let mut current = (x, y);
            'ray: for &(fraction, fade) in &taps {
                let distance = reach * fraction;
                let sample = ((x as f32 + distance * dx / length).round() as i64, (y as f32 + distance * dy / length).round() as i64);

                // A sparse tap must still have continuous coverage along the ray
                while current != sample {
                    current = next_ray_texel((x, y), current, sample, (dx, dy));
                    if axis_at(current.0, current.1).is_none() {
                        break 'ray;
                    }
                }

                // A tap beyond a ridge contributes only if it is higher than the receiving texel
                if axis_at(sample.0, sample.1).is_some_and(|value| value >= axis_here)
                    && let Some(value) = index_at(sample.0, sample.1).and_then(|index| field.get(index))
                {
                    most = most.max(*value * fade);
                }
            }
            most
        })
        .collect()
}

/// Finite difference within one UV island, one-sided at its edge
fn slope(before: Option<f32>, here: f32, after: Option<f32>) -> f32 {
    match (before, after) {
        (Some(before), Some(after)) => (after - before) * 0.5,
        (None, Some(after)) => after - here,
        (Some(before), None) => here - before,
        (None, None) => 0.0,
    }
}

/// Advance across the next texel boundary on a ray from a texel centre toward its next tap
fn next_ray_texel(origin: (i64, i64), (x, y): (i64, i64), (target_x, target_y): (i64, i64), (dx, dy): (f32, f32)) -> (i64, i64) {
    let step_x = (target_x - x).signum();
    let step_y = (target_y - y).signum();
    if step_x == 0 || step_y == 0 {
        return (x + step_x, y + step_y);
    }

    // Compare crossing distances without dividing by either component
    let crossing_x = ((x - origin.0).abs() as f32 + 0.5) * dy.abs();
    let crossing_y = ((y - origin.1).abs() as f32 + 0.5) * dx.abs();
    (x + step_x * i64::from(crossing_x <= crossing_y), y + step_y * i64::from(crossing_y <= crossing_x))
}

#[cfg(test)]
mod tests {
    use texturecraft_core::{Project, TextureSet};
    use texturecraft_geom::{UvGBuffer, primitives};

    use super::*;
    use crate::{BakedMaps, PixelStore};

    const RES: u32 = 256;

    #[test]
    fn a_drip_takes_no_rust_from_past_a_ridge_lower_on_the_axis() {
        // A strip rising gently to a ridge at its middle, then falling steeply
        let mut mesh = primitives::plane(2.0, 64);
        for position in &mut mesh.positions {
            position.y = if position.z <= 0.0 { 1.0 + position.z } else { 1.0 - 8.0 * position.z };
        }
        let gbuf = UvGBuffer::new(&mesh, 0, RES, RES);
        let mut project = Project::new("t", "strip");
        project.texture_sets.push(TextureSet::new("A", 0, RES));
        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), Default::default());
        let inputs = Inputs {
            project: &project,
            ts: 0,
            mesh: &mesh,
            gbuf: &gbuf,
            pixels: &pixels,
            baked: &baked,
            block_scales: project.texture_sets[0].block_scales(),
            preview: None,
            images: &images,
            bounds: mesh.bounds(),
        };

        // Rust only on the steep side, all lower than the gentle side's texel just below the ridge
        let rusted = |surface: &texturecraft_geom::Surface| surface.pos.z > 0.01;
        let field: Vec<f32> = (0..RES * RES).map(|texel| gbuf.surface(&mesh, texel as usize).filter(rusted).map_or(0.0, |_| 1.0)).collect();
        let rust = DrippingRustParameters { drips_intensity: 1.0, drips_samples: DrippingRustParameters::MAX_SAMPLES, axis: 1, ..Default::default() };
        let dripped = drips(&inputs, &rust, field, (RES, RES), &|_, _| RES as f32);

        let below_ridge = (0..RES * RES)
            .find(|texel| {
                gbuf.surface(&mesh, *texel as usize).is_some_and(|surface| (surface.pos.z + 0.04).abs() < 1.0 / RES as f32 && surface.pos.x.abs() < 0.1)
            })
            .expect("a texel just below the ridge");
        assert_eq!(dripped[below_ridge as usize], 0.0);
    }

    #[test]
    fn a_runoff_streak_requires_a_continuous_uv_surface() {
        const GAP_HALF_WIDTH: f32 = 1.0 / 64.0;
        let mut mesh = primitives::plane(2.0, 128);
        let connected_indices = mesh.indices.clone();
        let (triangles, _) = connected_indices.as_chunks::<3>();
        let separated_indices: Vec<_> = triangles
            .iter()
            .filter(|triangle| {
                triangle.iter().all(|&corner| mesh.positions[corner as usize].z <= -GAP_HALF_WIDTH)
                    || triangle.iter().all(|&corner| mesh.positions[corner as usize].z >= GAP_HALF_WIDTH)
            })
            .flatten()
            .copied()
            .collect();
        for position in &mut mesh.positions {
            position.y = position.z;
        }
        let mut project = Project::new("Runoff", "Plane");
        project.texture_sets.push(TextureSet::new("UV islands", 0, RES));
        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), Default::default());
        let index = |row| (row * RES + RES / 2) as usize;

        for (connected, indices) in [(true, connected_indices), (false, separated_indices)] {
            mesh.indices = indices;
            mesh.submeshes[0].count = mesh.indices.len() as u32;
            let gbuf = UvGBuffer::new(&mesh, 0, RES, RES);
            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: None,
                images: &images,
                bounds: mesh.bounds(),
            };
            let field: Vec<f32> =
                (0..RES * RES).map(|texel| f32::from(gbuf.surface(&mesh, texel as usize).is_some_and(|surface| surface.pos.z > GAP_HALF_WIDTH))).collect();
            assert!(gbuf.covered(index(124)));
            assert_eq!(gbuf.covered(index(128)), connected);
            assert_eq!(field[index(124)], 0.0);
            assert_eq!(field[index(131)], 1.0);

            for samples in [1, DrippingRustParameters::MAX_SAMPLES] {
                let rust = DrippingRustParameters { drips_intensity: 1.0, drips_samples: samples, axis: 1, ..Default::default() };

                let dripped = drips(&inputs, &rust, field.clone(), (RES, RES), &|_, _| RES as f32);

                assert_eq!(dripped[index(124)] > 0.0, connected, "a streak with {samples} taps requires a continuous covered surface");
            }
        }
    }
}
