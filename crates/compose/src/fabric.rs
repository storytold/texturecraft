//! Woven and knitted fabrics: knit, basket weave, goose-eye twill and slub silk.
//!
//! Every pattern repeats every unit in U and V, so it tiles without seams. Values are shaded like
//! a height map: threads round across, their ends tucked under the threads that cross them.

use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_texel::noise;

use crate::organic::{smoothstep, unit};

/// A thread's round cross-section: 0 at its edges, 1 along its crest, for `across` in 0..1
fn thread_profile(across: f32) -> f32 {
    let centered = 2.0 * across - 1.0;
    (1.0 - centered * centered).max(0.0).sqrt()
}

/// How a thread swells between the crossings at its ends, for `along` in 0..1
fn bulge(along: f32) -> f32 {
    0.55 + 0.45 * (std::f32::consts::PI * along).sin()
}

/// Distance from `point` to the segment from `start` to `end`, and the fraction along it.
fn segment(point: Vec2, start: Vec2, end: Vec2) -> (f32, f32) {
    const MIN_LENGTH_SQUARED: f32 = 1e-9;
    let direction = end - start;
    let along = ((point - start).dot(direction) / direction.length_squared().max(MIN_LENGTH_SQUARED)).clamp(0.0, 1.0);
    ((point - (start + direction * along)).length(), along)
}

/// Stockinette: columns of V-shaped stitches, each of two twisted legs of yarn that lean towards
/// each other and meet below. The stitches of one row sit over those of the next.
pub(crate) fn knit(p: Vec3, seed: u32) -> f32 {
    const COLUMNS: i32 = 10;
    const ROWS: i32 = 14;
    const YARN_RADIUS_IN_STITCH_WIDTHS: f32 = 0.21;
    const TWISTS_PER_LEG: f32 = 2.5;
    const TWIST_SLOPE: f32 = 0.3;
    let point = unit(p) * Vec2::new(COLUMNS as f32, ROWS as f32);
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);

    let mut height = 0.0f32;
    let mut tone = 0.5f32;
    for stitch_row in (row - 1)..=(row + 1) {
        for stitch_column in (column - 1)..=(column + 1) {
            let cell = Vec2::new(stitch_column as f32, stitch_row as f32);
            let yarn = 0.85 + 0.15 * noise::cell_hash(stitch_column.rem_euclid(COLUMNS), stitch_row.rem_euclid(ROWS), 0, seed ^ 0x4B);

            // The two legs share the bottom of the stitch
            for (side, top) in [(-1.0f32, Vec2::new(0.1, -0.05)), (1.0, Vec2::new(0.9, -0.05))] {
                let (distance, along) = segment(point, cell + top, cell + Vec2::new(0.5, 1.05));
                if distance >= YARN_RADIUS_IN_STITCH_WIDTHS {
                    continue;
                }

                let across = distance / YARN_RADIUS_IN_STITCH_WIDTHS;
                let twist = 0.5 + 0.5 * (std::f32::consts::TAU * (along * TWISTS_PER_LEG + side * across * TWIST_SLOPE)).cos();
                let leg = (1.0 - across * across).sqrt() * (0.82 + 0.18 * twist) * yarn;
                if leg > height {
                    height = leg;
                    tone = yarn;
                }
            }
        }
    }

    (0.12 + 0.88 * height * (0.8 + 0.2 * tone)).clamp(0.0, 1.0)
}

/// A basket weave: threads in pairs, each pair lying over and then under the next pair, so the
/// pairs make a checkerboard of squares whose threads alternately run across and down.
pub(crate) fn basket_weave(p: Vec3, seed: u32) -> f32 {
    // Even, so that the checkerboard lines up again after one unit
    const BLOCKS: i32 = 4;
    const THREADS_PER_BLOCK: i32 = 2;
    const FIBRES_PER_THREAD: f32 = 6.0;
    let point = unit(p) * BLOCKS as f32;
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);
    let local = point - Vec2::new(column as f32, row as f32);

    let horizontal = (column + row).rem_euclid(2) == 0;
    let (along, position) = if horizontal { (local.x, local.y) } else { (local.y, local.x) };
    let thread_position = position * THREADS_PER_BLOCK as f32;
    let thread = thread_position.floor() as i32;
    let across = thread_position - thread as f32;

    let tone =
        noise::cell_hash((column * THREADS_PER_BLOCK + thread).rem_euclid(BLOCKS * THREADS_PER_BLOCK), row.rem_euclid(BLOCKS), horizontal as i32, seed ^ 0x2D);
    let fibre = 0.9 + 0.1 * (std::f32::consts::TAU * across * FIBRES_PER_THREAD).cos();
    let crest = thread_profile(across) * bulge(along) * fibre;
    ((0.1 + 0.9 * crest) * (0.75 + 0.25 * tone) * if horizontal { 1.0 } else { 0.9 }).clamp(0.0, 1.0)
}

/// Goose-eye twill: a 2/2 twill whose diagonal turns back every few threads, across and down, so
/// the ribs close into small diamonds with an eye at the middle. The warp, which runs down, is
/// brighter than the weft.
pub(crate) fn goose_eye(p: Vec3, seed: u32) -> f32 {
    const THREADS: i32 = 24;
    const THREADS_BETWEEN_TURNS: i32 = 6;
    const CROSSINGS_PER_REPEAT: i32 = 4;
    const WARP_PER_REPEAT: i32 = 2;
    let q = unit(p) * THREADS as f32;
    let (i, j) = (q.x.floor() as i32, q.y.floor() as i32);
    let (fx, fy) = (q.x - i as f32, q.y - j as f32);

    // Threads away from the nearest turn
    let from_turn = |thread: i32| (thread.rem_euclid(2 * THREADS_BETWEEN_TURNS) - THREADS_BETWEEN_TURNS).abs();
    let warp_over = |i: i32, j: i32| (from_turn(i) + from_turn(j)).rem_euclid(CROSSINGS_PER_REPEAT) < WARP_PER_REPEAT;

    let tone = noise::cell_hash(i.rem_euclid(THREADS), j.rem_euclid(THREADS), 0, seed ^ 0x3E);

    // Taper the ends only where the thread crosses underneath
    let tuck = |along: f32, before: bool, after: bool| {
        let start = if before { 1.0 } else { 0.55 + 0.45 * smoothstep(0.0, 0.3, along) };
        let end = if after { 1.0 } else { 0.55 + 0.45 * smoothstep(1.0, 0.7, along) };
        start * end
    };
    let (crest, brightness) = if warp_over(i, j) {
        (thread_profile(fx) * tuck(fy, warp_over(i, j - 1), warp_over(i, j + 1)), 1.0)
    } else {
        (thread_profile(fy) * tuck(fx, !warp_over(i - 1, j), !warp_over(i + 1, j)), 0.4)
    };
    ((0.1 + 0.9 * crest) * brightness * (0.9 + 0.1 * tone)).clamp(0.0, 1.0)
}

/// A fine plain weave of raw silk: here and there a thread swells into a slub, a thick, bright
/// stretch of yarn that floats over the threads it crosses, and each thread has a sheen of its own.
pub(crate) fn slub_silk(p: Vec3, seed: u32) -> f32 {
    const THREADS: i32 = 36;
    const SLUB_CELLS_PER_THREAD: i32 = 7;
    const SLUB_START: f32 = 0.74;
    const SLUB_FULL: f32 = 0.93;
    const FLOAT_THRESHOLD: f32 = 0.5;
    const SHEEN_HASH_COMPONENT: i32 = 99;
    let u = unit(p);
    let q = u * THREADS as f32;
    let (i, j) = (q.x.floor() as i32, q.y.floor() as i32);
    let (fx, fy) = (q.x - i as f32, q.y - j as f32);

    // Slub strength along a thread: smooth noise, mostly low
    let slub_of = |thread: i32, along_unit: f32, warp: bool| {
        let thread = thread.rem_euclid(THREADS);
        let along = along_unit * SLUB_CELLS_PER_THREAD as f32;
        let cell = along.floor() as i32;
        let fraction = along - cell as f32;
        let at = |step: i32| noise::cell_hash(thread, (cell + step).rem_euclid(SLUB_CELLS_PER_THREAD), warp as i32, seed ^ 0x5A);
        let start = at(0);
        smoothstep(SLUB_START, SLUB_FULL, start + (at(1) - start) * fraction * fraction * (3.0 - 2.0 * fraction))
    };

    // Warp threads run down the page and weft threads across it
    let (warp_slub, weft_slub) = (slub_of(i, u.y, true), slub_of(j, u.x, false));

    // A slubbed thread floats over the threads it crosses; elsewhere they interlace one over one
    let warp_over = if warp_slub > FLOAT_THRESHOLD {
        true
    } else if weft_slub > FLOAT_THRESHOLD {
        false
    } else {
        (i + j).rem_euclid(2) == 0
    };
    let (thread, slub, across, along) = if warp_over { (i, warp_slub, fx, fy) } else { (j, weft_slub, fy, fx) };

    let sheen = 0.45 + 0.25 * noise::cell_hash(thread.rem_euclid(THREADS), SHEEN_HASH_COMPONENT, warp_over as i32, seed ^ 0x6B);
    let tuck = if slub > FLOAT_THRESHOLD { 1.0 } else { bulge(along) };
    let crest = thread_profile(across) * tuck * (0.75 + 0.45 * slub);
    ((0.25 + 0.75 * crest) * (sheen + 0.28 * slub)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stitch_is_a_v_and_a_basket_weave_has_pairs() {
        let grid = |size: usize| (0..size * size).map(move |i| Vec3::new((i % size) as f32 / size as f32, (i / size) as f32 / size as f32, 0.0));
        let hollows = grid(128).filter(|p| knit(*p, 1) < 0.15).count();

        assert!((300..8000).contains(&hollows), "hollows cover {hollows} of 16384 samples");

        // A horizontal pair has two crests separated and flanked by hollows
        let across = |y: f32| basket_weave(Vec3::new(0.125, y, 0.0), 1);
        for crest in [0.0625, 0.1875] {
            assert!(across(crest) > 0.5, "no crest at {crest}: {}", across(crest));
        }
        for hollow in [0.001, 0.125, 0.249] {
            assert!(across(hollow) < 0.3, "no hollow at {hollow}: {}", across(hollow));
        }
    }

    #[test]
    fn goose_eye_turns_its_diagonal() {
        let row = |y: f32| (0..48).map(|i| goose_eye(Vec3::new((i as f32 + 0.5) / 48.0 / 2.0, y, 0.0), 1) > 0.5).collect::<Vec<_>>();
        let (first, second) = (row(0.5 / 24.0), row(5.5 / 24.0));

        assert_ne!(first, second, "every row weaves the same");
    }
}
