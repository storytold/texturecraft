//! Procedural patterns and projections.

use texturecraft_core::{Decal, Procedural, ProceduralKind, Projection};
use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_texel::noise;

/// Where a texel is, for pattern evaluation.
#[derive(Clone, Copy, Debug)]
pub struct Coords {
    pub uv: Vec2,
    /// World position divided by the scene radius (scale-independent).
    pub pos: Vec3,
    pub nrm: Vec3,
}

/// Raw pattern value at a 3D point, 0..1.
fn raw(kind: ProceduralKind, p: Vec3, seed: u32) -> f32 {
    let a = p.to_array();
    match kind {
        ProceduralKind::Clouds => noise::fbm(a, 6, seed, noise::value),
        ProceduralKind::Perlin => noise::fbm(a, 4, seed, noise::perlin),
        ProceduralKind::Noise => noise::value((p * 16.0).to_array(), seed),
        ProceduralKind::Cells => noise::worley(a, seed),
        ProceduralKind::Grunge => {
            let n = noise::fbm(a, 6, seed, noise::perlin) * 0.7 + noise::fbm((p * 3.7).to_array(), 3, seed ^ 0x55, noise::value) * 0.3;
            ((n - 0.45) * 4.0 + 0.5).clamp(0.0, 1.0)
        }
        ProceduralKind::Scratches => {
            // Long thin streaks: noise stretched along one axis, sharpened to lines.
            let q = Vec3::new(p.x * 0.15, p.y * 6.0, p.z * 6.0);
            let n = noise::perlin(q.to_array(), seed);
            let line = 1.0 - ((n - 0.5).abs() * 40.0).min(1.0);
            let gate = noise::value((p * 0.8).to_array(), seed ^ 0x99);
            line * ((gate - 0.45) * 4.0).clamp(0.0, 1.0)
        }
        ProceduralKind::Checker => {
            let c = p.floor();
            if (c.x + c.y + c.z).rem_euclid(2.0) < 1.0 { 1.0 } else { 0.0 }
        }
        ProceduralKind::Stripes => {
            let t = p.x.rem_euclid(1.0);
            if t < 0.5 { 1.0 } else { 0.0 }
        }
        ProceduralKind::Gradient => p.x.rem_euclid(1.0),
        ProceduralKind::Wood => {
            // Rings across Y (warped by low-frequency noise), latewood darkening sharply before
            // each ring boundary; fine fibers stretched along X.
            let warp = noise::fbm([p.x * 0.18, p.y * 0.6, p.z], 3, seed, noise::perlin);
            let t = ((p.y + (warp - 0.5) * 0.9) * 5.0).rem_euclid(1.0);
            let ring = 1.0 - smoothstep(0.45, 1.0, t);
            let fiber = noise::value([p.x * 2.0, p.y * 40.0, p.z], seed ^ 0x3C);
            let tone = noise::fbm([p.x * 0.6, p.y * 3.0, p.z], 2, seed ^ 0x71, noise::value);
            (ring * 0.6 + fiber * 0.25 + tone * 0.15).clamp(0.0, 1.0)
        }
        ProceduralKind::Bricks => {
            // Bricks 1 × 0.5 in running bond; per-brick tone in the top 15 % so low/high map to
            // mortar/brick colors without washing the bricks out.
            let row = (p.y * 2.0).floor();
            let x = p.x + if (row as i32).rem_euclid(2) == 1 { 0.5 } else { 0.0 };
            let (col, fx, fy) = (x.floor(), x.rem_euclid(1.0), (p.y * 2.0).rem_euclid(1.0));
            let d = fx.min(1.0 - fx).min(fy.min(1.0 - fy) * 0.5);
            let tone = noise::cell_hash(col as i32, row as i32, 0, seed) * 0.6 + noise::value((p * 9.0).to_array(), seed) * 0.4;
            smoothstep(0.02, 0.045, d) * (0.85 + 0.15 * tone)
        }
        ProceduralKind::Tiles => {
            let (c, f) = (p.floor(), p - p.floor());
            let d = f.x.min(1.0 - f.x).min(f.y.min(1.0 - f.y));
            smoothstep(0.015, 0.035, d) * (0.9 + 0.1 * noise::cell_hash(c.x as i32, c.y as i32, 0, seed))
        }
        ProceduralKind::Weave | ProceduralKind::Twill => {
            // Warp along X, weft along Y: in each cell the thread on top shows its round
            // cross-section, bulging mid-span. A plain weave alternates every cell; a 2/2 twill
            // floats over two cells and steps one cell per row, so the floats line up in diagonal
            // ribs, the weft darker than the warp.
            let c = (p * 2.0).floor();
            let f = p * 2.0 - c;
            let pi = std::f32::consts::PI;
            let twill = kind == ProceduralKind::Twill;
            let (period, warp_cells) = if twill { (4, 2) } else { (2, 1) };
            let warp_on_top = (c.x as i32).wrapping_add(c.y as i32).rem_euclid(period) < warp_cells;
            let (across, along) = if warp_on_top { (f.y, f.x) } else { (f.x, f.y) };
            let thread = (pi * across).sin().max(0.0).sqrt() * (0.8 + 0.2 * (pi * along).sin());
            let tone = if twill && !warp_on_top { 0.55 } else { 1.0 };
            let fiber = noise::value([p.x * 40.0, p.y * 40.0, p.z], seed);
            (thread * tone * (0.88 + 0.12 * fiber)).clamp(0.0, 1.0)
        }
        ProceduralKind::Leather => {
            let w = noise::perlin((p * 0.7).to_array(), seed ^ 0x2D) - 0.5;
            let (f1, f2, _) = noise::cellular([p.x + w * 0.4, p.y - w * 0.4, p.z], seed);
            let crease = smoothstep(0.0, 0.16, f2 - f1);
            let pebble = 1.0 - f1 * 0.5;
            let fine = noise::value((p * 7.0).to_array(), seed ^ 0x13);
            (crease * pebble * (0.85 + 0.15 * fine)).clamp(0.0, 1.0)
        }
        ProceduralKind::Hammered => {
            let dent = |q: Vec3, s: u32| {
                let (f1, _, _) = noise::cellular(q.to_array(), s);
                (f1 * 1.25).min(1.0).powi(2)
            };
            (dent(p, seed) * 0.75 + dent(p * 2.3 + Vec3::splat(5.1), seed ^ 0x5A) * 0.25).clamp(0.0, 1.0)
        }
        ProceduralKind::Brushed => {
            let a = noise::value([p.x * 1.5, p.y * 60.0, p.z], seed);
            let b = noise::value([p.x * 0.4, p.y * 18.0, p.z], seed ^ 0x77);
            (a * 0.6 + b * 0.4).clamp(0.0, 1.0)
        }
        ProceduralKind::Marble => {
            let turb = noise::fbm(a, 5, seed, noise::perlin);
            let veins = ((p.x + p.y * 0.5) * 2.5 + turb * 9.0).sin().abs().powf(0.3);
            let cloud = noise::fbm((p * 2.0).to_array(), 3, seed ^ 0x19, noise::value);
            (veins * 0.85 + cloud * 0.15).clamp(0.0, 1.0)
        }
        ProceduralKind::Voronoi => noise::cellular(a, seed).2,
        ProceduralKind::Streaks => {
            // Drips: thin columns along Y, each segment starting at a random height and fading
            // as it runs down (Y is up), over two offset column grids.
            let drip = |x: f32, salt: u32| {
                let cx = x * 6.0;
                let col = cx.floor() as i32;
                let h = |k: i32| noise::cell_hash(col, k, salt as i32, seed);
                let width = 0.08 + 0.25 * h(0);
                let across = 1.0 - smoothstep(width * 0.3, width, (cx - cx.floor() - 0.5 - (h(1) - 0.5) * 0.5).abs());
                let yy = p.y * (0.35 + 0.4 * h(2)) + h(3) * 7.0;
                let t = yy - yy.floor();
                let on = if noise::cell_hash(col, yy.floor() as i32, 3 + salt as i32, seed) > 0.5 { 1.0 } else { 0.0 };
                across * on * t.powf(1.6) * (1.0 - smoothstep(0.9, 1.0, t))
            };
            let fine = noise::value([p.x * 37.0, p.y * 1.3, p.z], seed ^ 0x2B);
            (drip(p.x, 0).max(drip(p.x + 0.37, 1) * 0.8) * (0.6 + 0.4 * fine)).clamp(0.0, 1.0)
        }
        ProceduralKind::Smudges => {
            // Sparse elliptical prints with concentric ridges, over soft greasy smudges.
            let w = noise::perlin((p * 1.3).to_array(), seed ^ 0x61) - 0.5;
            let q = Vec3::new(p.x * 1.15 + w * 0.3, p.y * 0.85 + w * 0.3, p.z);
            let (f1, _, id) = noise::cellular(q.to_array(), seed);
            let print = (1.0 - smoothstep(0.18 + 0.1 * id, 0.3 + 0.12 * id, f1)) * if id > 0.4 { 1.0 } else { 0.0 };
            let ridges = 0.5 + 0.5 * (f1 * 190.0 + w * 12.0).sin();
            let smudge = ((noise::fbm((p * 1.7).to_array(), 4, seed ^ 0x33, noise::value) - 0.55) * 3.0).clamp(0.0, 1.0);
            (print * ridges * 0.9 + smudge * 0.4).clamp(0.0, 1.0)
        }
        ProceduralKind::Pores => {
            let (f1, _, _) = noise::cellular(a, seed);
            let (g1, _, _) = noise::cellular((p * 2.7 + Vec3::splat(13.0)).to_array(), seed ^ 0x6B);
            smoothstep(0.08, 0.16, f1).min(smoothstep(0.06, 0.12, g1))
        }
        ProceduralKind::Chips => {
            let (f1, _, _) = noise::cellular((p * 2.0).to_array(), seed ^ 0x45);
            let n = noise::fbm(a, 5, seed, noise::perlin) + (f1 - 0.5) * 0.25;
            smoothstep(0.55, 0.58, n)
        }
        ProceduralKind::HexagonTiles => hexagon_tiles(p, seed),
        ProceduralKind::WaveScales => wave_scales(p),
        ProceduralKind::HerringbonePlanks => herringbone_planks(p, seed),
        ProceduralKind::HalftoneDots => halftone_dots(p, seed),
        ProceduralKind::DiamondBrocade => diamond_brocade(p),
        ProceduralKind::TruchetArcs => truchet_arcs(p, seed),
    }
}

/// Hexagon tiles with joints (0 = joint), each with a tone of its own. A unit holds two columns
/// and two rows of tiles, a little taller than regular hexagons so that the pattern, tones
/// included, repeats every unit.
fn hexagon_tiles(p: Vec3, seed: u32) -> f32 {
    let p = within_unit(p);
    const ROW: f32 = 1.732_050_8;
    // Regular hexagons of width 1 in (u, v), centers on two lattices offset by half a repeat
    let q = Vec2::new(p.x * 2.0, p.y * ROW);
    let repeat = Vec2::new(1.0, ROW);
    let wrap = |q: Vec2| q - repeat * (q / repeat).round();
    let (a, b) = (wrap(q), wrap(q - repeat * 0.5));
    let local = if a.length_squared() < b.length_squared() { a } else { b };

    // Distance to the nearest edge: the apothem is 0.5 and the edge normals sit at 0, 60 and 120 degrees
    let reach = local.x.abs().max((local.x * 0.5 + local.y * 0.866_025_4).abs()).max((local.x * 0.5 - local.y * 0.866_025_4).abs());
    let joint = smoothstep(0.012, 0.045, 0.5 - reach);

    // Tile centers sit on a half-repeat grid: four columns and two rows of them per unit
    let center = q - local;
    let (column, row) = ((center.x * 2.0).round() as i32, (center.y / ROW * 2.0).round() as i32);
    let tone = noise::cell_hash(column.rem_euclid(4), row.rem_euclid(2), 0, seed);
    joint * (0.8 + 0.2 * tone)
}

/// Rows of circles of radius 0.5, a quarter unit apart, every other row shifted half a circle.
/// V grows downwards in texture space, so the row with the largest V covers the others and each
/// circle shows its top arcs. Repeats every unit.
fn wave_scales(p: Vec3) -> f32 {
    let p = within_unit(p);
    let row = (p.y * 4.0).floor() as i32;
    // Rows from the last one back: the first circle that reaches the point is the one on top
    for j in ((row - 2)..=(row + 2)).rev() {
        let dy = p.y - j as f32 * 0.25;
        if dy.abs() > 0.5 {
            continue;
        }

        let shift = if j.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
        let dx = p.x - ((p.x - shift).round() + shift);
        let t = (dx * dx + dy * dy).sqrt() / 0.5;
        if t <= 1.0 {
            // Concentric arcs, brighter towards the middle, with a dark rim where the next circle begins
            let arcs = 0.5 + 0.5 * (std::f32::consts::TAU * 3.5 * t).cos();
            let rim = 1.0 - 0.7 * smoothstep(0.9, 1.0, t);
            return ((0.3 + 0.7 * arcs) * rim).clamp(0.0, 1.0);
        }
    }

    0.0
}

/// Herringbone planks three times as long as wide, with gaps (0 = gap), each with a tone and a
/// grain of its own. Six plank widths make a repeat, so the pattern, tones included, repeats
/// every unit.
fn herringbone_planks(p: Vec3, seed: u32) -> f32 {
    let p = within_unit(p);
    const LENGTH: i32 = 3;
    const CELLS: i32 = 2 * LENGTH;
    let (x, y) = (p.x * CELLS as f32, p.y * CELLS as f32);
    let (i, j) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - i as f32, y - j as f32);

    // Along the diagonal the planks alternate: LENGTH cells of horizontal plank, then LENGTH of vertical
    let phase = (i - j).rem_euclid(CELLS);
    let (along, across, plank) = if phase < LENGTH {
        (phase as f32 + fx, fy, (i - phase, j, 0))
    } else {
        let step = CELLS - 1 - phase;
        (step as f32 + fy, fx, (i, j - step, 1))
    };
    let along = along / LENGTH as f32;

    // Distance to the plank's border, in plank widths
    let edge = across.min(1.0 - across).min(along.min(1.0 - along) * LENGTH as f32);
    let body = smoothstep(0.015, 0.06, edge);
    let tone = noise::cell_hash(plank.0.rem_euclid(CELLS), plank.1.rem_euclid(CELLS), plank.2, seed);
    let grain = noise::value([along * 3.0, across * 28.0, tone * 31.0], seed ^ 0x7B);
    body * (0.55 + 0.25 * tone + 0.2 * grain)
}

/// A halftone screen: bright dots on a grid turned 45 degrees, sixteen dots apart per unit, sized
/// by a cloudy tone. Each dot keeps the tone at its center and the tone repeats every unit, so the
/// pattern does too.
fn halftone_dots(p: Vec3, seed: u32) -> f32 {
    // Even, so that the turned grid lines up again after one unit
    const DOTS: f32 = 16.0;
    let p = within_unit(p);
    let q = Vec2::new(p.x, p.y) * DOTS;

    // In (u, v) the turned grid is the integer grid; its points are √2 apart in q. The point lies
    // in the square of four of them, and the dot that covers it most shows: there is no edge
    // where the choice of dot jumps, so dots that meet stay continuous
    const SMALLEST: f32 = 0.12;
    const GROWTH: f32 = 0.6;
    const EDGE: f32 = 0.05;
    let (u, v) = ((q.x + q.y) * 0.5, (q.x - q.y) * 0.5);
    let mut coverage = 0.0f32;
    for (du, dv) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let (cu, cv) = (u.floor() + du, v.floor() + dv);
        let center = Vec2::new(cu + cv, cu - cv);
        let distance = (q - center).length();
        // Skips the tone, the costly part, where even the largest dot would not reach
        if distance >= SMALLEST + GROWTH + EDGE {
            continue;
        }

        // Stretched, as a sum of octaves gathers around the middle
        let tone = smoothstep(0.25, 0.75, periodic_fbm(center / DOTS, seed));
        let radius = SMALLEST + GROWTH * tone;
        coverage = coverage.max(1.0 - smoothstep(radius - EDGE, radius + EDGE, distance));
    }

    coverage
}

/// `p` moved by whole units into the unit square, for patterns that repeat every unit: their
/// integer cell indices then stay small, and a large coordinate keeps its precision. A coordinate
/// that is not finite becomes 0
fn within_unit(p: Vec3) -> Vec3 {
    let wrap = |x: f32| if x.is_finite() { x.rem_euclid(1.0) } else { 0.0 };
    Vec3::new(wrap(p.x), wrap(p.y), p.z)
}

/// Smooth value noise in 0..1 on a lattice of `cells` × `cells` per unit, whose indices wrap, so
/// the noise repeats every unit
fn periodic_value_noise(at: Vec2, cells: i32, seed: u32) -> f32 {
    let q = at * cells as f32;
    let (i, j) = (q.x.floor() as i32, q.y.floor() as i32);
    let f = q - Vec2::new(i as f32, j as f32);
    let s = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let corner = |di: i32, dj: i32| noise::cell_hash((i + di).rem_euclid(cells), (j + dj).rem_euclid(cells), 0, seed);
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * s.x;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * s.x;
    top + (bottom - top) * s.y
}

/// Fractal value noise in 0..1 that repeats every unit: octaves of [`periodic_value_noise`] with
/// a whole number of cells per unit each
fn periodic_fbm(at: Vec2, seed: u32) -> f32 {
    const OCTAVES: [(i32, f32); 3] = [(3, 0.55), (6, 0.3), (12, 0.15)];
    OCTAVES.iter().enumerate().map(|(k, (cells, weight))| weight * periodic_value_noise(at, *cells, seed ^ (k as u32 * 0x9E37))).sum()
}

/// Raised diamonds on a twill ground, four by four per unit. Each diamond is a set of concentric
/// rings with a dark border; the ground shows fine diagonal ribs. Repeats every unit.
fn diamond_brocade(p: Vec3) -> f32 {
    use std::f32::consts::{PI, TAU};

    const DIAMONDS: f32 = 4.0;
    let p = within_unit(p);
    let q = Vec2::new(p.x, p.y) * DIAMONDS;
    let local = q - q.floor() - Vec2::splat(0.5);
    // 0 at a diamond's center, 0.5 at its corners' edge
    let d = local.x.abs() + local.y.abs();

    // The twill ground: fine diagonal ribs, a whole number of them per unit
    let ribs = 0.5 + 0.5 * (TAU * 40.0 * (q.x + q.y) / DIAMONDS).sin();
    let ground = 0.22 + 0.12 * ribs;

    // The motif, rising towards the middle in rings, nearly filling its cell
    const MOTIF: f32 = 0.46;
    let inside = 1.0 - smoothstep(MOTIF - 0.03, MOTIF, d);
    let rings = 0.5 + 0.5 * (TAU * 2.5 * d / MOTIF).cos();
    let rise = (PI * 0.5 * (1.0 - d / MOTIF)).sin().max(0.0);
    let motif = (0.45 + 0.35 * rise + 0.2 * rings) * (0.9 + 0.1 * ribs);
    (ground + (motif - ground) * inside).clamp(0.0, 1.0)
}

/// Truchet tiles: in each of eight by eight cells per unit, two quarter-circle arcs join the
/// middles of its sides, around one pair of opposite corners or the other. The arcs meet across
/// cells into winding round tubes. The choice comes from the cell's index wrapped to the grid,
/// so the pattern repeats every unit.
fn truchet_arcs(p: Vec3, seed: u32) -> f32 {
    let p = within_unit(p);
    const CELLS: i32 = 8;
    const TUBE: f32 = 0.13;
    let q = Vec2::new(p.x, p.y) * CELLS as f32;
    let (i, j) = (q.x.floor() as i32, q.y.floor() as i32);
    let local = q - Vec2::new(i as f32, j as f32);

    // Around the top-left and bottom-right corners, or the top-right and bottom-left ones
    let flipped = noise::cell_hash(i.rem_euclid(CELLS), j.rem_euclid(CELLS), 0, seed) > 0.5;
    let corners = if flipped { [Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)] } else { [Vec2::ZERO, Vec2::ONE] };
    let offset = corners.iter().map(|corner| ((local - *corner).length() - 0.5).abs()).fold(f32::MAX, f32::min);

    // A round tube across, over a dark ground
    let s = (offset / TUBE).min(1.0);
    0.05 + 0.9 * (1.0 - s * s).sqrt()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn shape(pr: &Procedural, v: f32) -> f32 {
    let v = ((v - 0.5) * pr.contrast.max(0.0) + 0.5 + pr.balance).clamp(0.0, 1.0);
    if pr.invert { 1.0 - v } else { v }
}

/// Evaluate a procedural through a projection.
pub fn procedural(pr: &Procedural, proj: &Projection, c: &Coords) -> f32 {
    let s = pr.scale.max(1e-4);
    let v = match proj {
        Projection::Uv { scale, rotation, offset } => {
            let (sn, cs) = rotation.to_radians().sin_cos();
            let uv = Vec2::new(c.uv.x * cs - c.uv.y * sn, c.uv.x * sn + c.uv.y * cs) * Vec2::from(*scale) + Vec2::from(*offset);
            // 2D patterns sample a slice of the 3D pattern.
            raw(pr.kind, (uv * s).extend(0.0), pr.seed)
        }
        Projection::Triplanar { scale, sharpness, offset } => {
            let p = c.pos * *scale * s + Vec3::from(*offset);
            let w = c.nrm.abs().powf(sharpness.max(1.0));
            let w = w / (w.x + w.y + w.z).max(1e-6);
            raw(pr.kind, Vec3::new(p.z, p.y, 0.0), pr.seed) * w.x
                + raw(pr.kind, Vec3::new(p.x, p.z, 0.0), pr.seed) * w.y
                + raw(pr.kind, Vec3::new(p.x, p.y, 0.0), pr.seed) * w.z
        }
        Projection::Planar { scale, axis, offset } => {
            let p = c.pos * *scale * s + Vec3::from(*offset);
            let q = match axis {
                0 => Vec3::new(p.z, p.y, 0.0),
                1 => Vec3::new(p.x, p.z, 0.0),
                _ => Vec3::new(p.x, p.y, 0.0),
            };
            raw(pr.kind, q, pr.seed)
        }
        Projection::Spherical { scale } => {
            let d = c.pos.try_normalize().unwrap_or(Vec3::Y);
            let u = d.z.atan2(d.x) / std::f32::consts::TAU + 0.5;
            let v = d.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
            raw(pr.kind, Vec3::new(u * 4.0, v * 2.0, 0.0) * *scale * s, pr.seed)
        }
        Projection::Cylindrical { scale, axis } => {
            let p = c.pos;
            let (a, h, b) = match axis {
                0 => (p.y, p.x, p.z),
                2 => (p.x, p.z, p.y),
                _ => (p.x, p.y, p.z),
            };
            let u = b.atan2(a) / std::f32::consts::TAU + 0.5;
            raw(pr.kind, Vec3::new(u * 4.0, h, 0.0) * *scale * s, pr.seed)
        }
        // `c.uv` is already in the decal's frame (see [`decal_uv`]).
        Projection::Decal(_) => raw(pr.kind, (c.uv * s).extend(0.0), pr.seed),
    };
    shape(pr, v)
}

/// Texture coordinates (and blend weights) at which an image is sampled for a projection
/// (unused slots weigh 0; no allocation per texel).
pub fn image_samples(proj: &Projection, c: &Coords) -> [(Vec2, f32); 3] {
    let one = |uv: Vec2| [(uv, 1.0), (Vec2::ZERO, 0.0), (Vec2::ZERO, 0.0)];
    match proj {
        Projection::Uv { scale, rotation, offset } => {
            let (sn, cs) = rotation.to_radians().sin_cos();
            let uv = Vec2::new(c.uv.x * cs - c.uv.y * sn, c.uv.x * sn + c.uv.y * cs) * Vec2::from(*scale) + Vec2::from(*offset);
            one(uv)
        }
        Projection::Triplanar { scale, sharpness, offset } => {
            let p = c.pos * *scale + Vec3::from(*offset);
            let w = c.nrm.abs().powf(sharpness.max(1.0));
            let w = w / (w.x + w.y + w.z).max(1e-6);
            [(Vec2::new(p.z, -p.y), w.x), (Vec2::new(p.x, p.z), w.y), (Vec2::new(p.x, -p.y), w.z)]
        }
        Projection::Planar { scale, axis, offset } => {
            let p = c.pos * *scale + Vec3::from(*offset);
            let q = match axis {
                0 => Vec2::new(p.z, -p.y),
                1 => Vec2::new(p.x, p.z),
                _ => Vec2::new(p.x, -p.y),
            };
            one(q * 0.5 + Vec2::splat(0.5))
        }
        Projection::Spherical { scale } => {
            let d = c.pos.try_normalize().unwrap_or(Vec3::Y);
            let u = d.z.atan2(d.x) / std::f32::consts::TAU + 0.5;
            let v = d.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
            one(Vec2::new(u, v) * *scale)
        }
        Projection::Cylindrical { scale, axis } => {
            let p = c.pos;
            let (a, h, b) = match axis {
                0 => (p.y, p.x, p.z),
                2 => (p.x, p.z, p.y),
                _ => (p.x, p.y, p.z),
            };
            let u = b.atan2(a) / std::f32::consts::TAU + 0.5;
            one(Vec2::new(u, 0.5 - h * 0.5) * *scale)
        }
        Projection::Decal(_) => one(c.uv),
    }
}

/// A decal's frame: origin (center plus offset), unit right, up and normal (None when degenerate).
pub fn decal_axes(d: &Decal) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    if !d.size.iter().all(|v| v.is_finite() && *v > 0.0) {
        return None;
    }
    let n = Vec3::from(d.normal).try_normalize()?;
    let up0 = (Vec3::from(d.up) - n * n.dot(Vec3::from(d.up))).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
    let right0 = up0.cross(n);
    let origin = Vec3::from(d.center) + right0 * d.offset[0] + up0 * d.offset[1];
    let (sn, cs) = d.rotation.to_radians().sin_cos();
    let right = right0 * cs + up0 * sn;
    let up = up0 * cs - right0 * sn;
    origin.is_finite().then_some((origin, right, up, n))
}

/// The decal's corners in world space (top left, top right, bottom right, bottom left).
pub fn decal_corners(d: &Decal) -> Option<[Vec3; 4]> {
    let (o, r, u, _) = decal_axes(d)?;
    let (hw, hh) = (d.size[0] * 0.5, d.size[1] * 0.5);
    Some([o - r * hw + u * hh, o + r * hw + u * hh, o + r * hw - u * hh, o - r * hw - u * hh])
}

/// Where world point `p` with surface normal `nrm` falls in the decal: texture coordinates
/// (v down) and how much the decal covers it, None outside the frame, behind it, beyond its
/// depth or on faces turned away. Coverage fades on grazing faces and near the depth limit.
pub fn decal_uv(d: &Decal, p: Vec3, nrm: Vec3) -> Option<(Vec2, f32)> {
    let (o, r, u, n) = decal_axes(d)?;
    let facing = nrm.dot(n);
    let q = p - o;
    let (lx, ly, lz) = (q.dot(r) / d.size[0].max(1e-6), q.dot(u) / d.size[1].max(1e-6), q.dot(n).abs());
    let depth = d.depth.max(1e-6);
    if facing <= 0.0 || lx.abs() > 0.5 || ly.abs() > 0.5 || lz > depth || !(lx.is_finite() && ly.is_finite()) {
        return None;
    }
    let w = smoothstep(0.0, 0.3, facing) * (1.0 - smoothstep(0.75 * depth, depth, lz));
    Some((Vec2::new(lx + 0.5, 0.5 - ly), w))
}

/// World-space noise used by generators (seamless across UV islands).
pub fn world_noise(pos: Vec3, scale: f32, seed: u32) -> f32 {
    noise::fbm((pos * scale.max(1e-3)).to_array(), 4, seed, noise::perlin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_and_projection_stays_in_range() {
        let projs = [
            Projection::default(),
            Projection::Triplanar { scale: 1.0, sharpness: 4.0, offset: [0.0; 3] },
            Projection::Planar { scale: 1.0, axis: 1, offset: [0.0; 3] },
            Projection::Spherical { scale: 1.0 },
            Projection::Cylindrical { scale: 1.0, axis: 1 },
            Projection::Decal(Decal {
                center: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                up: [0.0, 1.0, 0.0],
                size: [1.0; 2],
                rotation: 0.0,
                offset: [0.0; 2],
                depth: 1.0,
            }),
        ];
        for k in ProceduralKind::ALL {
            let pr = Procedural::new(k);
            for proj in &projs {
                for i in 0..50 {
                    let t = i as f32 * 0.13;
                    let c = Coords {
                        uv: Vec2::new(t.fract(), (t * 0.7).fract()),
                        pos: Vec3::new(t.sin(), t.cos(), t * 0.1),
                        nrm: Vec3::new(0.3, 0.9, 0.1).normalize(),
                    };
                    let v = procedural(&pr, proj, &c);
                    assert!((0.0..=1.0).contains(&v), "{k:?} {proj:?} {v}");
                }
            }
        }
    }

    #[test]
    fn decals_cover_their_frame_facing_them() {
        let d = Decal { center: [0.0; 3], normal: [0.0, 0.0, 1.0], up: [0.0, 1.0, 0.0], size: [2.0, 1.0], rotation: 0.0, offset: [0.0; 2], depth: 0.5 };
        let z = Vec3::Z;
        let (uv, w) = decal_uv(&d, Vec3::new(0.5, 0.25, 0.0), z).unwrap();
        assert!((uv - Vec2::new(0.75, 0.25)).length() < 1e-5 && (w - 1.0).abs() < 1e-5, "{uv} {w}");
        assert!(decal_uv(&d, Vec3::new(1.1, 0.0, 0.0), z).is_none(), "outside the frame");
        assert!(decal_uv(&d, Vec3::ZERO, -z).is_none(), "back faces");
        assert!(decal_uv(&d, Vec3::new(0.0, 0.0, 0.6), z).is_none(), "beyond the depth");
        // Rotated a quarter turn and shifted: the frame turns with it.
        let r = Decal { rotation: 90.0, offset: [1.0, 0.0], ..d };
        let (uv, _) = decal_uv(&r, Vec3::new(1.0, 0.9, 0.0), z).unwrap();
        assert!((uv.x - 0.95).abs() < 1e-4 && (uv.y - 0.5).abs() < 1e-4, "{uv}");
        let c = decal_corners(&d).unwrap();
        assert!((c[0] - Vec3::new(-1.0, 0.5, 0.0)).length() < 1e-5 && (c[2] - Vec3::new(1.0, -0.5, 0.0)).length() < 1e-5);
        assert!(decal_axes(&Decal { normal: [0.0; 3], ..d }).is_none());
        assert!(decal_uv(&Decal { size: [f32::NAN; 2], ..d }, Vec3::ZERO, z).is_none());
    }

    #[test]
    fn twill_floats_run_in_diagonal_ribs() {
        // The value at the centre of cell (x, y); cells are half a unit.
        let at = |k, x: i32, y: i32| raw(k, Vec3::new(x as f32 * 0.5 + 0.25, y as f32 * 0.5 + 0.25, 0.0), 3);
        for y in 1..8 {
            let warp: Vec<bool> = (0..8).map(|x| at(ProceduralKind::Twill, x, y) > 0.7).collect();
            assert_eq!(warp.iter().filter(|w| **w).count(), 4, "row {y}: half the floats are warp");
            for x in 0..7 {
                assert_eq!(warp[x as usize], at(ProceduralKind::Twill, x + 1, y - 1) > 0.7, "the rib continues diagonally at ({x}, {y})");
            }
        }
        // A plain weave shows every top thread alike.
        assert!((0..8).all(|x| at(ProceduralKind::Weave, x, 0) > 0.7));
    }

    #[test]
    fn geometric_tilings_repeat_every_unit() {
        // Shifted points round differently, which shows on the steep edges of a joint, so a seam
        // is anything above that rounding
        let tilings = [
            ProceduralKind::HexagonTiles,
            ProceduralKind::WaveScales,
            ProceduralKind::HerringbonePlanks,
            ProceduralKind::HalftoneDots,
            ProceduralKind::DiamondBrocade,
            ProceduralKind::TruchetArcs,
        ];
        for kind in tilings {
            for i in 0..400 {
                let p = Vec3::new((i as f32 * 0.137).fract() * 3.0 - 1.0, (i as f32 * 0.071).fract() * 3.0 - 1.0, 0.0);
                for shift in [Vec3::X, Vec3::Y, Vec3::new(-2.0, 3.0, 0.0)] {
                    let (here, there) = (raw(kind, p, 7), raw(kind, p + shift, 7));
                    assert!((here - there).abs() < 2e-3, "{kind:?}: {here} at {p}, {there} at {}", p + shift);
                }
            }
        }
    }

    #[test]
    fn geometric_tilings_take_extreme_coordinates() {
        let tilings = [
            ProceduralKind::HexagonTiles,
            ProceduralKind::WaveScales,
            ProceduralKind::HerringbonePlanks,
            ProceduralKind::HalftoneDots,
            ProceduralKind::DiamondBrocade,
            ProceduralKind::TruchetArcs,
        ];
        let extremes = [0.0, 5.4e8, -5.4e8, f32::MAX, -f32::MAX, f32::INFINITY, f32::NAN];
        for kind in tilings {
            for x in extremes {
                for y in extremes {
                    let value = raw(kind, Vec3::new(x, y, 0.0), 7);
                    assert!((0.0..=1.0).contains(&value), "{kind:?} at ({x}, {y}): {value}");
                }
            }
        }
    }

    #[test]
    fn halftone_dots_stay_continuous_where_neighbouring_dots_meet() {
        // Halfway between two dot centers lie the points 1/32 apart on the turned grid
        for seed in 0..32 {
            for k in 0..64 {
                let tie = Vec3::new(k as f32 / 32.0, (k * 7 % 32) as f32 / 32.0, 0.0);
                for step in [Vec3::X, Vec3::Y] {
                    let (before, after) =
                        (raw(ProceduralKind::HalftoneDots, tie - step * 1e-4, seed), raw(ProceduralKind::HalftoneDots, tie + step * 1e-4, seed));
                    assert!((before - after).abs() < 0.05, "seed {seed}, {tie}: {before} then {after}");
                }
            }
        }
    }

    #[test]
    fn hexagon_tiles_and_herringbone_planks_have_joints_between_them() {
        let at = |kind, x: f32, y: f32| raw(kind, Vec3::new(x, y, 0.0), 7);

        // A tile is centered at the origin and its right edge is a quarter unit away
        assert!(at(ProceduralKind::HexagonTiles, 0.0, 0.0) >= 0.8);
        assert!(at(ProceduralKind::HexagonTiles, 0.25, 0.0) < 0.05);

        // A unit is six plank widths: a horizontal plank starts at the origin, its middle is a plank and a half to the right
        assert!(at(ProceduralKind::HerringbonePlanks, 0.000_2, 0.5 / 6.0) < 0.05);
        assert!(at(ProceduralKind::HerringbonePlanks, 1.5 / 6.0, 0.5 / 6.0) >= 0.55);
    }

    #[test]
    fn every_pattern_varies() {
        for k in ProceduralKind::ALL {
            let pr = Procedural::new(k);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in 0..4096 {
                let uv = Vec2::new((i % 64) as f32 / 64.0, (i / 64) as f32 / 64.0);
                let v = procedural(&pr, &Projection::default(), &Coords { uv, pos: Vec3::ZERO, nrm: Vec3::Y });
                (lo, hi) = (lo.min(v), hi.max(v));
            }
            assert!(hi - lo > 0.3, "{k:?} is flat: {lo}..{hi}");
        }
    }
}
