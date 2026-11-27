//! Timber: strip flooring and knotty wood grain.
//!
//! Both repeat every unit in U and V, so they tile without seams. The grain runs along U.

use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_texel::noise;

use crate::organic::{lattice_fbm, lattice_noise, smoothstep, unit};

/// Strip flooring: long boards laid along U in rows, each row cut into pieces whose butt joints
/// are staggered from row to row (0 = gap). Every piece has a tone and growth rings of its own.
pub(crate) fn strip_flooring(p: Vec3, seed: u32) -> f32 {
    const ROWS: i32 = 6;
    const PIECES_PER_ROW: i32 = 2;
    const GAP_WIDTH: f32 = 0.0012;
    const GAP_FADE_END: f32 = 0.0035;
    let uv = unit(p);
    let y = uv.y * ROWS as f32;
    let row = y.floor() as i32;
    let across = y - row as f32;

    // Shift the joints independently in each row
    let wrapped_row = row.rem_euclid(ROWS);
    let row_offset = noise::cell_hash(wrapped_row, 0, 0, seed ^ 0x13);
    let x = (uv.x + row_offset) * PIECES_PER_ROW as f32;
    let piece = x.floor() as i32;
    let along = x - piece as f32;
    let hash = |component: i32| noise::cell_hash(wrapped_row, piece.rem_euclid(PIECES_PER_ROW), component + 1, seed ^ 0x13);

    // Distances to the board's edge and to the butt joint, in units
    let to_edge = across.min(1.0 - across) / ROWS as f32;
    let to_joint = along.min(1.0 - along) / PIECES_PER_ROW as f32;
    let gap = smoothstep(GAP_WIDTH, GAP_FADE_END, to_edge.min(to_joint));

    // Noise bends the growth rings into long arcs on the face of the board
    let warp = lattice_fbm(uv, &[(3, 0.6), (7, 0.4)], seed ^ 0x24) - 0.5;
    let rings = across * (1.6 + 1.2 * hash(0)) + hash(1) * 7.0 + warp * 2.2;
    let ring = 0.5 + 0.5 * (std::f32::consts::TAU * rings).cos();
    let fibre = lattice_noise(uv, (12, 240), seed ^ 0x35);

    let tone = 0.42 + 0.4 * hash(2);
    (gap * tone * (0.68 + 0.32 * ring.powf(1.4)) * (0.86 + 0.14 * fibre)).clamp(0.0, 1.0)
}

/// Knotty wood: growth rings running along U that bend around knots, tight circles at the heart
/// of each knot and a dark core. Knots sit on a jittered grid whose cells wrap.
pub(crate) fn knotty_wood(p: Vec3, seed: u32) -> f32 {
    const RINGS: f32 = 9.0;
    const CELLS: i32 = 3;
    const KNOT_REACH_RADII: f32 = 2.2;
    const KNOT_FULL_BLEND_RADII: f32 = 0.8;
    let u = unit(p);

    // Rings across the board, wandering a little
    let wander = (lattice_fbm(u, &[(2, 0.6), (5, 0.4)], seed ^ 0x14) - 0.5) * 1.4;
    let mut phase = u.y * RINGS + wander;
    let point = u * CELLS as f32;
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);

    let mut core = 0.0f32;
    let mut ring_noise = None;
    for knot_row in (row - 1)..=(row + 1) {
        let wrapped_row = knot_row.rem_euclid(CELLS);
        for knot_column in (column - 1)..=(column + 1) {
            let wrapped_column = knot_column.rem_euclid(CELLS);
            let hash = |component| noise::cell_hash(wrapped_column, wrapped_row, component, seed ^ 0x47);
            if hash(0) > 0.45 {
                continue;
            }

            let center_jitter = Vec2::new(hash(1), hash(2)) * 0.6;
            let center = Vec2::new(knot_column as f32 + 0.2, knot_row as f32 + 0.2) + center_jitter;
            let radius = 0.04 + 0.035 * hash(3);

            // Knots are longer along the grain
            let delta = u - center / CELLS as f32;
            let distance = Vec2::new(delta.x * 0.5, delta.y).length() / radius;
            if distance >= KNOT_REACH_RADII {
                continue;
            }

            let noise = *ring_noise.get_or_insert_with(|| (lattice_noise(u, (24, 24), seed ^ 0x66) - 0.5) * 0.8);
            let circle = distance * 1.9 + hash(4) * 3.0 + noise;

            // Measure the phase in the knot's own repeat to preserve continuity across wrap
            let home_y = (wrapped_row as f32 + 0.2 + center_jitter.y) / CELLS as f32;
            let ring_here = (home_y + delta.y) * RINGS + wander;
            phase += (circle - ring_here) * smoothstep(KNOT_REACH_RADII, KNOT_FULL_BLEND_RADII, distance);
            core = core.max(smoothstep(0.5, 0.2, distance));
        }
    }

    let ring = (0.5 + 0.5 * (std::f32::consts::TAU * phase).cos()).powf(1.6);
    let fibre = lattice_noise(u, (10, 260), seed ^ 0x58);
    let base = 0.34 + 0.5 * ring;
    (base * (0.86 + 0.14 * fibre) * (1.0 - 0.8 * core)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flooring_has_gaps_between_boards_and_pieces() {
        let grid = |size: usize| (0..size * size).map(move |i| Vec3::new((i % size) as f32 / size as f32, (i / size) as f32 / size as f32, 0.0));
        let gaps = grid(256).filter(|p| strip_flooring(*p, 1) < 0.02).count();

        assert!((300..6000).contains(&gaps), "gaps cover {gaps} of 65536 samples");

        // The joints of neighbouring rows do not line up
        let joints = |y: f32| (0..400).filter(|i| strip_flooring(Vec3::new(*i as f32 / 400.0, y, 0.0), 1) < 0.02).map(|i| i / 4).collect::<Vec<_>>();

        assert_ne!(joints(0.5 / 6.0), joints(1.5 / 6.0), "the rows share their joints");
    }

    #[test]
    fn knotty_wood_has_dark_knots() {
        let grid = |size: usize| (0..size * size).map(move |i| Vec3::new((i % size) as f32 / size as f32, (i / size) as f32 / size as f32, 0.0));
        let dark = grid(128).filter(|p| knotty_wood(*p, 1) < 0.12).count();

        assert!(dark > 10, "only {dark} samples are knot-dark");
    }
}
