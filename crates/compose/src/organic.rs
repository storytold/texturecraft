//! Organic procedural surfaces: fur, clouds, reptile scales, bark, flesh and camouflage.
//!
//! Every pattern repeats every unit in U and V, tones and details included, so it tiles without
//! seams. Wrapping finite coordinates before scaling keeps cell indices in range. Non-finite
//! components map to zero.

use std::cell::RefCell;

use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_texel::noise;

use crate::fur_lock::FurLock;

pub(crate) fn smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Wrap the plane coordinates before converting to cell indices. Non-finite components become 0.
pub(crate) fn unit(p: Vec3) -> Vec2 {
    let wrap = |x: f32| if x.is_finite() { x.rem_euclid(1.0) } else { 0.0 };
    Vec2::new(wrap(p.x), wrap(p.y))
}

/// Smooth value noise in 0..1 on a lattice of `cells` (across, down) per unit whose indices wrap,
/// so the noise repeats every unit
pub(crate) fn lattice_noise(at: Vec2, cells: (i32, i32), seed: u32) -> f32 {
    let point = Vec2::new(at.x * cells.0 as f32, at.y * cells.1 as f32);
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);
    let fraction = point - Vec2::new(column as f32, row as f32);
    let weight = fraction * fraction * (Vec2::splat(3.0) - 2.0 * fraction);
    let corner = |dx: i32, dy: i32| noise::cell_hash((column + dx).rem_euclid(cells.0), (row + dy).rem_euclid(cells.1), 0, seed);

    let top_left = corner(0, 0);
    let bottom_left = corner(0, 1);
    let top = top_left + (corner(1, 0) - top_left) * weight.x;
    let bottom = bottom_left + (corner(1, 1) - bottom_left) * weight.x;
    top + (bottom - top) * weight.y
}

/// Fractal [`lattice_noise`] in 0..1: octaves of (cells per unit, weight), the weights adding up to 1
pub(crate) fn lattice_fbm(at: Vec2, octaves: &[(i32, f32)], seed: u32) -> f32 {
    const OCTAVE_SALT: u32 = 0x9E37;
    octaves
        .iter()
        .enumerate()
        .map(|(index, (cells, weight))| weight * lattice_noise(at, (*cells, *cells), seed ^ (index as u32).wrapping_mul(OCTAVE_SALT)))
        .sum()
}

/// Locks of fur on each side of the repeat
const FUR_CELLS: usize = 6;
const FUR_LOCK_COUNT: usize = FUR_CELLS * FUR_CELLS;

type FurLocks = [FurLock; FUR_LOCK_COUNT];
type FurCacheEntry = Option<(u32, FurLocks)>;

thread_local! {
    // Adjacent layers or channels can alternate seeds on the same worker
    static FUR_CACHE: RefCell<(bool, [FurCacheEntry; 2])> = const { RefCell::new((true, [None, None])) };
}

fn build_fur_locks(seed: u32) -> FurLocks {
    const ROOT_JITTER_CELLS: f32 = 0.7;
    const AXIS_ANGLE_RADIANS: f32 = 0.1;
    const ANGLE_JITTER_RADIANS: f32 = 0.45;
    const LENGTH_CELLS: f32 = 3.0;
    const LENGTH_JITTER: f32 = 0.27;
    const CURL_RADIANS: f32 = -0.8;

    std::array::from_fn(|index| {
        let hash = |component| noise::cell_hash((index % FUR_CELLS) as i32, (index / FUR_CELLS) as i32, component, seed ^ 0x6D);
        let (angle_sin, angle_cos) = (AXIS_ANGLE_RADIANS + (hash(3) * 2.0 - 1.0) * ANGLE_JITTER_RADIANS).sin_cos();

        FurLock {
            root: Vec2::new(0.5 + (hash(1) - 0.5) * ROOT_JITTER_CELLS, 0.5 + (hash(2) - 0.5) * ROOT_JITTER_CELLS),
            length: LENGTH_CELLS * (1.0 + (hash(4) * 2.0 - 1.0) * LENGTH_JITTER),
            angle_sin,
            angle_cos,
            curl: CURL_RADIANS * (0.8 + hash(5) * 0.4),
            strength: 0.8 + hash(6) * 0.2,
        }
    })
}

/// Long smooth locks of fur, curling as they grow and tapering to a point, shaded like a height
/// map: buried at the root, round across, rising towards the tip. Where locks cross, the higher
/// one shows. Roots sit on a jittered grid whose cells wrap.
pub(crate) fn smooth_fur(p: Vec3, seed: u32) -> f32 {
    let point = unit(p) * FUR_CELLS as f32;
    FUR_CACHE.with(|cache| {
        let Ok(mut cache) = cache.try_borrow_mut() else {
            return fur_height(point, &build_fur_locks(seed));
        };

        let (replace_first, entries) = &mut *cache;
        if let Some((_, locks)) = entries.iter().flatten().find(|(cached_seed, _)| *cached_seed == seed) {
            return fur_height(point, locks);
        }

        let [first, second] = entries;
        let replacement = if *replace_first { first } else { second };
        *replace_first = !*replace_first;
        let (_, locks) = replacement.insert((seed, build_fur_locks(seed)));
        fur_height(point, locks)
    })
}

fn fur_height(point: Vec2, locks: &FurLocks) -> f32 {
    const WIDTH_CELLS: f32 = 1.55;
    const REACH_CELLS: f32 = 4.0;
    const EARLIER_ROWS: i32 = 5;
    const LATER_ROWS: i32 = 1;
    const COLUMNS_EACH_SIDE: i32 = 5;
    const TAPER_EXPONENT: f32 = 0.65;
    const ROOT_FADE: f32 = 0.06;
    const TIP_FADE: f32 = 0.025;
    const MIN_RADIUS_CELLS: f32 = 1e-6;
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);

    let mut height = 0.0f32;

    // Locks grow down the page, so roots can reach farther from earlier rows
    for root_row in (row - EARLIER_ROWS)..=(row + LATER_ROWS) {
        let row_start = root_row.rem_euclid(FUR_CELLS as i32) as usize * FUR_CELLS;
        for root_column in (column - COLUMNS_EACH_SIDE)..=(column + COLUMNS_EACH_SIDE) {
            let index = row_start + root_column.rem_euclid(FUR_CELLS as i32) as usize;
            let Some(lock) = locks.get(index) else { continue };
            let delta = point - (Vec2::new(root_column as f32, root_row as f32) + lock.root);
            if delta.length_squared() > REACH_CELLS * REACH_CELLS {
                continue;
            }

            // Reject points beyond the root or tip before evaluating the curved axis
            let local = Vec2::new(lock.angle_cos * delta.x + lock.angle_sin * delta.y, -lock.angle_sin * delta.x + lock.angle_cos * delta.y);
            if local.y < -WIDTH_CELLS * 0.5 || local.y > lock.length {
                continue;
            }

            let Some((along, offset)) = on_arc(local, lock.length, lock.curl) else { continue };
            let radius = WIDTH_CELLS * 0.5 * (1.0 - along).powf(TAPER_EXPONENT);
            if radius <= MIN_RADIUS_CELLS || offset.abs() >= radius {
                continue;
            }

            let across = offset.abs() / radius;
            let profile = (1.0 - across * across).sqrt();
            let lock_height =
                lock.strength * profile * smoothstep(0.0, 1.0, along / ROOT_FADE) * smoothstep(0.0, 1.0, (1.0 - along) / TIP_FADE) * (0.28 + 0.72 * along);
            height = height.max(lock_height);
        }
    }

    height.clamp(0.0, 1.0)
}

/// Where a point lies along a lock whose axis grows along +V and turns by `curl` radians over
/// `length`: the fraction of the length it is at and its distance from the axis, or None outside
/// the lock's length.
fn on_arc(local: Vec2, length: f32, curl: f32) -> Option<(f32, f32)> {
    const STRAIGHT_CURL_RADIANS: f32 = 1e-4;
    if curl.abs() < STRAIGHT_CURL_RADIANS {
        let along = local.y / length;
        return (0.0..=1.0).contains(&along).then_some((along, local.x));
    }

    // The axis is an arc of radius `length / curl` around a center beside the root
    let radius = length / curl;
    let along = (local.y / radius).atan2((radius - local.x) / radius) / curl;
    let distance = Vec2::new(local.x - radius, local.y).length();
    (0.0..=1.0).contains(&along).then_some((along, distance - radius.abs()))
}

/// Soft round cloudlets of mixed sizes on a jittered grid, crowded so they overlap; the dark sky
/// shows only where none reaches.
pub(crate) fn mackerel_sky(p: Vec3, seed: u32) -> f32 {
    const CELLS: i32 = 6;
    const CLOUDLET_RADIUS_CELLS: f32 = 0.65;
    const RADIUS_VARIATION_CELLS: f32 = 0.45;
    const DOME_FEATHER_START: f32 = 0.2;
    const DOME_FEATHER_END: f32 = 1.2;

    // A soft maximum, so cloudlets merge where they meet instead of leaving a seam
    const SOFTNESS: f32 = 14.0;
    let uv = unit(p);
    let point = uv * CELLS as f32;
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);

    let mut sum = 0.0f32;
    for cloud_row in (row - 2)..=(row + 2) {
        for cloud_column in (column - 2)..=(column + 2) {
            let hash = |component| noise::cell_hash(cloud_column.rem_euclid(CELLS), cloud_row.rem_euclid(CELLS), component, seed ^ 0x5F);
            let center = Vec2::new(cloud_column as f32 + hash(1), cloud_row as f32 + hash(2));
            let radial_fraction = (point - center).length() / (CLOUDLET_RADIUS_CELLS + RADIUS_VARIATION_CELLS * hash(3));
            if radial_fraction >= DOME_FEATHER_END {
                continue;
            }

            // A blurred dome: flat and bright in the middle, fading softly past its rim
            let dome =
                smoothstep(DOME_FEATHER_END, DOME_FEATHER_START, radial_fraction) * (0.85 + 0.15 * (1.0 - radial_fraction * radial_fraction).max(0.0).sqrt());
            sum += (SOFTNESS * dome * (0.6 + 0.4 * hash(4))).exp() - 1.0;
        }
    }

    let value = (1.0 + sum).ln() / SOFTNESS;
    let mottle = lattice_fbm(uv, &[(12, 0.5), (24, 0.3), (48, 0.2)], seed ^ 0x3A);
    (value * (0.88 + 0.12 * mottle)).clamp(0.0, 1.0)
}

/// Domain-warped clouds, crossed by fine strands that follow a slowly turning flow: high frequency
/// noise averaged along the flow direction (a short line integral).
pub(crate) fn billow(p: Vec3, seed: u32) -> f32 {
    const WARP_AMPLITUDE: f32 = 0.2;
    const STRAND_HALF_SAMPLES: i32 = 5;
    const STRAND_SAMPLE_SPACING: f32 = 0.0045;
    let uv = unit(p);
    let warp = |salt: u32| lattice_fbm(uv, &[(2, 0.5), (4, 0.3), (8, 0.2)], seed ^ salt) - 0.5;
    let warped = uv + Vec2::new(warp(0x11), warp(0x22)) * WARP_AMPLITUDE;

    let body = lattice_fbm(warped, &[(3, 0.3), (6, 0.25), (12, 0.2), (24, 0.15), (48, 0.1)], seed);
    let angle = std::f32::consts::TAU * lattice_fbm(warped, &[(2, 0.6), (4, 0.4)], seed ^ 0x33);
    let flow = Vec2::new(angle.cos(), angle.sin());
    let strands = (-STRAND_HALF_SAMPLES..=STRAND_HALF_SAMPLES)
        .map(|sample| lattice_noise(warped + flow * (sample as f32 * STRAND_SAMPLE_SPACING), (64, 64), seed ^ 0x44))
        .sum::<f32>()
        / (2 * STRAND_HALF_SAMPLES + 1) as f32;
    let speckle = lattice_noise(warped, (32, 32), seed ^ 0x55);

    (0.5 + (body - 0.5) * 1.7 + (strands - 0.5) * 0.45 + (speckle - 0.5) * 0.1).clamp(0.0, 1.0)
}

/// Overlapping reptile scales: rounded shields on staggered rows, each of a size and tone of its
/// own, domed and lit from above, with a dark rim and a shadow cast on the scale below. A scale
/// covers the ones under it. The rows wrap, so the pattern repeats every unit.
pub(crate) fn reptile_scales(p: Vec3, seed: u32) -> f32 {
    const GROUND: f32 = 0.1;
    scale_at(p, seed).unwrap_or(GROUND)
}

/// The value of the scale showing at `p`, or None when no scale covers it
fn scale_at(p: Vec3, seed: u32) -> Option<f32> {
    const COLUMNS: i32 = 8;

    // Even, so that the staggered rows line up again after one unit
    const ROWS: i32 = 12;
    const HALF_WIDTH_CELLS: f32 = 0.62;
    const HALF_HEIGHT_CELLS: f32 = 1.3;
    const SHADOW_REACH: f32 = 1.35;
    let uv = unit(p);
    let point = Vec2::new(uv.x * COLUMNS as f32, uv.y * ROWS as f32);
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);
    let light = Vec3::new(-0.35, -0.5, 0.8).normalize();

    // The earliest overlapping row sits above the others
    let (mut top, mut top_row, mut shadow) = (None, i32::MAX, 0.0f32);
    for scale_row in (row - 2)..=(row + 2) {
        let stagger = if scale_row.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
        for scale_column in (column - 1)..=(column + 2) {
            let hash = |component| noise::cell_hash(scale_column.rem_euclid(COLUMNS), scale_row.rem_euclid(ROWS), component, seed ^ 0x2B);
            let center = Vec2::new(scale_column as f32 + 0.5 + stagger + (hash(1) - 0.5) * 0.3, scale_row as f32 + 0.5 + (hash(2) - 0.5) * 0.4);
            let (half_width, half_height) = (HALF_WIDTH_CELLS * (0.92 + 0.16 * hash(3)), HALF_HEIGHT_CELLS * (0.92 + 0.16 * hash(4)));
            let delta = point - center;

            // Narrower towards the lower tip
            let narrowing = 1.0 - 0.3 * (delta.y / half_height).clamp(0.0, 1.0);
            let local = Vec2::new(delta.x / (half_width * narrowing), delta.y / half_height);
            let rim = local.length();
            if rim < 1.0 {
                if scale_row < top_row || (scale_row == top_row && hash(5) > 0.5) {
                    top_row = scale_row;
                    top = Some((local, rim, hash(6), hash(7)));
                }
            } else if rim < SHADOW_REACH && delta.y > 0.0 {
                shadow = shadow.max(smoothstep(SHADOW_REACH, 1.0, rim));
            }
        }
    }

    let (local, rim, tone, ridge) = top?;
    let height = (1.0 - rim * rim).max(0.0).sqrt();

    // The keel catches light along the middle of the scale
    let keel = (-(local.x * 3.2) * (local.x * 3.2)).exp() * (0.1 + 0.25 * ridge);
    let normal = Vec3::new(local.x * 0.9, local.y * 0.9, height + keel).normalize();
    let lit = normal.dot(light).max(0.0);
    let grain = lattice_noise(uv, (96, 96), seed ^ 0x3C);
    let body = (0.25 + 0.75 * tone) * (0.35 + 0.65 * lit) * (1.0 - 0.5 * shadow) * (0.86 + 0.14 * grain);
    Some((body * (0.45 + 0.55 * smoothstep(1.0, 0.8, rim))).clamp(0.0, 1.0))
}

/// Young bark: tall plates of different tone parted by wandering furrows (0 = furrow), with fibres
/// running along the trunk. The plates are the cells of a stretched, jittered grid that wraps.
pub(crate) fn bark(p: Vec3, seed: u32) -> f32 {
    const COLUMNS: i32 = 7;
    const ROWS: i32 = 3;
    const TRUNK_AXIS_WEIGHT: f32 = 0.38;
    const PLATE_JITTER_CELLS: f32 = 0.85;
    const FURROW_FADE_END: f32 = 0.4;
    let uv = unit(p);

    let wobble = Vec2::new(lattice_noise(uv, (5, 5), seed ^ 0x31), lattice_noise(uv, (5, 5), seed ^ 0x32)) - Vec2::splat(0.5);
    let warped = uv + wobble * Vec2::new(0.06, 0.14);

    let point = Vec2::new(warped.x * COLUMNS as f32, warped.y * ROWS as f32);
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);
    let (mut nearest, mut next, mut plate_tone) = (f32::MAX, f32::MAX, 0.0f32);
    for plate_row in (row - 2)..=(row + 2) {
        for plate_column in (column - 1)..=(column + 1) {
            let hash = |component| noise::cell_hash(plate_column.rem_euclid(COLUMNS), plate_row.rem_euclid(ROWS), component, seed ^ 0x27);
            let center =
                Vec2::new(plate_column as f32 + 0.5 + (hash(1) - 0.5) * PLATE_JITTER_CELLS, plate_row as f32 + 0.5 + (hash(2) - 0.5) * PLATE_JITTER_CELLS);
            let delta = point - center;
            let distance = Vec2::new(delta.x / COLUMNS as f32, delta.y / ROWS as f32 * TRUNK_AXIS_WEIGHT).length();
            if distance < nearest {
                (next, nearest, plate_tone) = (nearest, distance, hash(3));
            } else if distance < next {
                next = distance;
            }
        }
    }

    // The furrow is where the two nearest plates meet
    let to_furrow = (next - nearest) * COLUMNS as f32;
    let ridge = smoothstep(0.0, FURROW_FADE_END, to_furrow).sqrt();
    let fibre = lattice_noise(warped, (96, 6), seed ^ 0x52);
    (ridge * (0.5 + 0.3 * plate_tone + 0.2 * fibre)).clamp(0.0, 1.0)
}

/// Rotting flesh: soft masses of wet tissue, folded and creased, threaded with branching raised
/// veins, going dark and mottled in patches of decay where blisters rise. The coordinates are
/// warped so folds and veins meander. Higher values represent moister, healthier tissue.
pub(crate) fn flesh(p: Vec3, seed: u32) -> f32 {
    const BLISTER_CELLS: i32 = 14;
    const WARP_AMPLITUDE: f32 = 0.24;
    const DECAY_START: f32 = 0.46;
    const DECAY_FULL: f32 = 0.58;
    const BLISTER_CHANCE: f32 = 0.55;
    let uv = unit(p);
    let warp = |salt: u32| lattice_fbm(uv, &[(2, 0.5), (4, 0.3), (8, 0.2)], seed ^ salt) - 0.5;
    let warped = uv + Vec2::new(warp(0x11), warp(0x22)) * WARP_AMPLITUDE;

    // Center the creases at the midpoint of the fold noise
    let body = lattice_fbm(warped, &[(3, 0.35), (6, 0.3), (12, 0.2), (24, 0.15)], seed ^ 0x33);
    let fold = lattice_fbm(warped, &[(5, 0.6), (10, 0.4)], seed ^ 0x44);
    let crease = (1.0 - (2.0 * fold - 1.0).abs()).powi(4);

    // Two vein widths meet to give the appearance of branches
    let line = |cells: (i32, i32), salt: u32, sharpness: i32| {
        (1.0 - (2.0 * lattice_fbm(warped, &[(cells.0, 0.6), (cells.1, 0.4)], seed ^ salt) - 1.0).abs()).powi(sharpness)
    };
    let vein = line((3, 7), 0x55, 14).max(0.7 * line((6, 13), 0x56, 20));
    let healthy = (0.2 + 0.7 * smoothstep(0.2, 0.8, body)) * (1.0 - 0.7 * crease) + 0.3 * vein;

    // Mottle and blisters contribute only where the decay layer is visible
    let decay = smoothstep(DECAY_START, DECAY_FULL, lattice_fbm(warped, &[(2, 0.5), (5, 0.3), (10, 0.2)], seed ^ 0x66));
    if decay == 0.0 {
        return healthy.clamp(0.0, 1.0);
    }

    let mottle = lattice_fbm(warped, &[(24, 0.5), (48, 0.5)], seed ^ 0x77);
    let point = warped * BLISTER_CELLS as f32;
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);
    let mut blister = 0.0f32;
    for blister_row in (row - 1)..=(row + 1) {
        for blister_column in (column - 1)..=(column + 1) {
            let hash = |component| noise::cell_hash(blister_column.rem_euclid(BLISTER_CELLS), blister_row.rem_euclid(BLISTER_CELLS), component, seed ^ 0x88);
            if hash(0) > BLISTER_CHANCE {
                continue;
            }

            let center = Vec2::new(blister_column as f32 + hash(1), blister_row as f32 + hash(2));
            blister = blister.max(smoothstep(1.0, 0.0, (point - center).length() / (0.3 + 0.4 * hash(3))));
        }
    }

    let rotten = healthy * (0.3 + 0.4 * mottle) + 0.35 * blister;
    (healthy + (rotten - healthy) * decay).clamp(0.0, 1.0)
}

/// Camouflage: four tones of interlocking blotches with ragged edges. The darkest tone is the
/// ground and each lighter one is laid over it in blotches of its own size, a little stretched
/// across the surface. Every tone is flat, so a gradient map turns it into woodland, desert or
/// any other colours.
pub(crate) fn camouflage(p: Vec3, seed: u32) -> f32 {
    const EDGE_FEATHER: f32 = 0.012;
    const EDGE_WOBBLE: f32 = 0.14;
    const GROUND: f32 = 0.05;
    let uv = unit(p);

    // Fractal noise on a lattice stretched across the surface
    let blotches = |cells: (i32, i32), salt: u32| {
        0.55 * lattice_noise(uv, cells, seed ^ salt)
            + 0.3 * lattice_noise(uv, (cells.0 * 2, cells.1 * 2), seed ^ salt ^ 0x9E)
            + 0.15 * lattice_noise(uv, (cells.0 * 4, cells.1 * 4), seed ^ salt ^ 0x3F)
    };

    // The same ragged edge for every tone
    let ragged = (lattice_fbm(uv, &[(24, 0.6), (48, 0.4)], seed ^ 0x71) - 0.5) * EDGE_WOBBLE;

    let layer = |field: f32, threshold: f32| smoothstep(threshold - EDGE_FEATHER, threshold + EDGE_FEATHER, field + ragged);
    let mut value = GROUND;
    for (tone, field, threshold) in [(0.35, blotches((3, 2), 0x11), 0.46), (0.65, blotches((4, 3), 0x22), 0.54), (0.95, blotches((6, 4), 0x33), 0.6)] {
        value += (tone - value) * layer(field, threshold);
    }

    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    type PatternSampler = fn(Vec3, u32) -> f32;
    const PATTERNS: [(&str, PatternSampler); 13] = [
        ("fur", smooth_fur),
        ("sky", mackerel_sky),
        ("billow", billow),
        ("scales", reptile_scales),
        ("bark", bark),
        ("flesh", flesh),
        ("camouflage", camouflage),
        ("knit", crate::fabric::knit),
        ("basket weave", crate::fabric::basket_weave),
        ("goose-eye", crate::fabric::goose_eye),
        ("slub silk", crate::fabric::slub_silk),
        ("strip flooring", crate::timber::strip_flooring),
        ("knotty wood", crate::timber::knotty_wood),
    ];

    /// A spread of points that are away from any cell edge
    fn points() -> impl Iterator<Item = Vec3> {
        (0..400).map(|i| {
            let t = i as f32;
            Vec3::new((t * 0.618_034 + 0.013).fract(), (t * 0.414_214 + 0.071).fract(), 0.0)
        })
    }

    #[test]
    fn every_pattern_repeats_every_unit() {
        for (name, pattern) in PATTERNS {
            for seed in [1, 7] {
                for p in points() {
                    let value = pattern(p, seed);
                    for shift in [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(-3.0, 5.0, 0.0)] {
                        let shifted = pattern(p + shift, seed);

                        assert!((value - shifted).abs() < 1e-3, "{name} at {p:?} (seed {seed}), shifted by {shift:?}: {value} against {shifted}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_pattern_stays_in_range_and_varies() {
        for (name, pattern) in PATTERNS {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in 0..4096 {
                let p = Vec3::new((i % 64) as f32 / 64.0, (i / 64) as f32 / 64.0, 0.0);
                let v = pattern(p, 1);
                assert!((0.0..=1.0).contains(&v), "{name} at {p:?}: {v}");
                (lo, hi) = (lo.min(v), hi.max(v));
            }

            assert!(hi - lo > 0.4, "{name} is flat: {lo}..{hi}");
        }
    }

    #[test]
    fn the_seed_changes_every_pattern() {
        for (name, pattern) in PATTERNS {
            let differs = points().any(|p| (pattern(p, 1) - pattern(p, 2)).abs() > 0.02);

            assert!(differs, "{name} ignores its seed");
        }
    }

    #[test]
    fn every_pattern_takes_extreme_coordinates() {
        let extremes = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, f32::MIN, 1e30, -1e30, 16_777_216.5, -0.0];
        for (name, pattern) in PATTERNS {
            for x in extremes {
                for y in extremes {
                    let v = pattern(Vec3::new(x, y, x), 3);
                    assert!((0.0..=1.0).contains(&v), "{name} at ({x}, {y}): {v}");
                }
            }
        }
    }

    #[test]
    fn scales_leave_no_gap_and_bark_makes_furrows() {
        let grid = |size: usize| (0..size * size).map(move |i| Vec3::new((i % size) as f32 / size as f32, (i / size) as f32 / size as f32, 0.0));

        // Check actual coverage because a scale can be as dark as the background
        for seed in 1..=3 {
            let uncovered = grid(64).filter(|p| scale_at(*p, seed).is_none()).count();

            assert_eq!(uncovered, 0, "{uncovered} of 4096 points are under no scale (seed {seed})");
        }

        let furrows = grid(64).filter(|p| bark(*p, 1) < 0.1).count();

        assert!((100..1800).contains(&furrows), "furrows cover {furrows} of 4096 samples");
    }

    #[test]
    fn flesh_has_creases_veins_and_decay() {
        let values: Vec<f32> = (0..16384).map(|i| flesh(Vec3::new((i % 128) as f32 / 128.0, (i / 128) as f32 / 128.0, 0.0), 1)).collect();
        let share = |keep: fn(f32) -> bool| values.iter().filter(|v| keep(**v)).count() as f32 / values.len() as f32;

        assert!(share(|v| v < 0.18) > 0.08, "no creases or rot: {}", share(|v| v < 0.18));
        assert!(share(|v| v > 0.5) > 0.08, "no healthy tissue: {}", share(|v| v > 0.5));
    }

    #[test]
    fn the_edges_of_the_unit_are_no_rougher_than_the_middle_of_it() {
        const STEP: f32 = 1e-4;
        const SAMPLES: usize = 512;

        // Count abrupt changes on either side of the sampled line
        let jumps = |pattern: PatternSampler, at: f32, across_u: bool| {
            (0..SAMPLES)
                .filter(|i| {
                    let t = (*i as f32 + 0.5) / SAMPLES as f32;
                    let (before, after) = (at - STEP, at + STEP);
                    let (a, b) =
                        if across_u { (Vec3::new(before, t, 0.0), Vec3::new(after, t, 0.0)) } else { (Vec3::new(t, before, 0.0), Vec3::new(t, after, 0.0)) };
                    (pattern(a, 1) - pattern(b, 1)).abs() > 0.2
                })
                .count()
        };

        for (name, pattern) in PATTERNS {
            for (edge, across_u) in [("U", true), ("V", false)] {
                // Compare against an interior line to allow normal lattice transitions
                let (seam, middle) = (jumps(pattern, 0.0, across_u), jumps(pattern, 0.5, across_u));

                assert!(seam <= 3 * middle + SAMPLES / 10, "{name}: {seam} jumps across the {edge} edge of the unit, against {middle} across its middle");
            }
        }
    }

    #[test]
    fn camouflage_lays_down_four_tones() {
        let mut counts = [0usize; 4];
        for i in 0..16384 {
            let v = camouflage(Vec3::new((i % 128) as f32 / 128.0, (i / 128) as f32 / 128.0, 0.0), 1);
            let tone = [0.05, 0.35, 0.65, 0.95].iter().enumerate().min_by(|a, b| (a.1 - v).abs().total_cmp(&(b.1 - v).abs())).map_or(0, |(index, _)| index);
            if let Some(count) = counts.get_mut(tone) {
                *count += 1;
            }
        }

        for (tone, count) in counts.iter().enumerate() {
            assert!(*count > 16384 / 25, "tone {tone} covers only {count} of 16384 samples: {counts:?}");
        }
    }
}
