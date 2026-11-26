//! Procedural patterns and projections.

use texturecraft_core::{Decal, FULL_TURN_DEGREES, Procedural, ProceduralKind, Projection, Projector, UvTransform};
use texturecraft_geom::glam::{EulerRot, Mat3, Vec2, Vec3};
use texturecraft_texel::noise;

use crate::pattern_context::PatternContext;
use crate::{
    fabric,
    organic::{self, lattice_fbm, smoothstep, unit},
    timber,
};

/// Smaller pattern scales are read as this: a pattern stretched over ten thousand repeats of its projection
const MIN_PATTERN_SCALE: f32 = 1e-4;

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
            // Stretch noise along one axis to make thin streaks
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
            // Warp ring boundaries and add fine fibers along X
            let warp = noise::fbm([p.x * 0.18, p.y * 0.6, p.z], 3, seed, noise::perlin);
            let t = ((p.y + (warp - 0.5) * 0.9) * 5.0).rem_euclid(1.0);
            let ring = 1.0 - smoothstep(0.45, 1.0, t);
            let fiber = noise::value([p.x * 2.0, p.y * 40.0, p.z], seed ^ 0x3C);
            let tone = noise::fbm([p.x * 0.6, p.y * 3.0, p.z], 2, seed ^ 0x71, noise::value);
            (ring * 0.6 + fiber * 0.25 + tone * 0.15).clamp(0.0, 1.0)
        }
        ProceduralKind::Bricks => {
            // Keep per-brick tone near its high value so mortar stays distinct
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
            // Alternate top threads for plain weave or step them into twill ribs
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
            // Offset column grids make sparse drips that fade downwards
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
            // Blend ridged elliptical prints with soft smudges
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
        ProceduralKind::TileGenerator => crate::tile_pattern::value(&crate::tile_pattern::DEFAULT, p.truncate(), seed, None),
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
        ProceduralKind::SmoothFur => organic::smooth_fur(p, seed),
        ProceduralKind::MackerelSky => organic::mackerel_sky(p, seed),
        ProceduralKind::Billow => organic::billow(p, seed),
        ProceduralKind::ReptileScales => organic::reptile_scales(p, seed),
        ProceduralKind::Bark => organic::bark(p, seed),
        ProceduralKind::Flesh => organic::flesh(p, seed),
        ProceduralKind::Camouflage => organic::camouflage(p, seed),
        ProceduralKind::Knit => fabric::knit(p, seed),
        ProceduralKind::BasketWeave => fabric::basket_weave(p, seed),
        ProceduralKind::GooseEye => fabric::goose_eye(p, seed),
        ProceduralKind::SlubSilk => fabric::slub_silk(p, seed),
        ProceduralKind::StripFlooring => timber::strip_flooring(p, seed),
        ProceduralKind::KnottyWood => timber::knotty_wood(p, seed),
    }
}

/// Hexagon tiles with joints (0 = joint), each with a tone of its own. A unit holds two columns
/// and two rows of tiles, a little taller than regular hexagons so that the pattern, tones
/// included, repeats every unit.
fn hexagon_tiles(p: Vec3, seed: u32) -> f32 {
    const VERTICAL_REPEAT: f32 = 1.732_050_8;
    const SIN_60: f32 = 0.866_025_4;
    const APOTHEM: f32 = 0.5;
    const JOINT_WIDTH: f32 = 0.012;
    const JOINT_FADE_END: f32 = 0.045;
    let uv = unit(p);

    // Regular hexagons of width 1 have centers on two lattices half a repeat apart
    let point = Vec2::new(uv.x * 2.0, uv.y * VERTICAL_REPEAT);
    let repeat = Vec2::new(1.0, VERTICAL_REPEAT);
    let wrap = |point: Vec2| point - repeat * (point / repeat).round();
    let first = wrap(point);
    let second = wrap(point - repeat * 0.5);
    let local = if first.length_squared() < second.length_squared() { first } else { second };

    // Project onto the edge normals at 0, 60 and 120 degrees
    let reach = local.x.abs().max((local.x * 0.5 + local.y * SIN_60).abs()).max((local.x * 0.5 - local.y * SIN_60).abs());
    let joint = smoothstep(JOINT_WIDTH, JOINT_FADE_END, APOTHEM - reach);

    // Tile centers sit on a half-repeat grid: four columns and two rows of them per unit
    let center = point - local;
    let (column, row) = ((center.x * 2.0).round() as i32, (center.y / VERTICAL_REPEAT * 2.0).round() as i32);
    let tone = noise::cell_hash(column.rem_euclid(4), row.rem_euclid(2), 0, seed);
    joint * (0.8 + 0.2 * tone)
}

/// Rows of circles of radius 0.5, a quarter unit apart, every other row shifted half a circle.
/// V grows downwards in texture space, so the row with the largest V covers the others and each
/// circle shows its top arcs. Repeats every unit.
fn wave_scales(p: Vec3) -> f32 {
    const ROW_SPACING: f32 = 0.25;
    const CIRCLE_RADIUS: f32 = 0.5;
    const NEIGHBOR_ROWS: i32 = 2;
    const ARC_COUNT: f32 = 3.5;
    const RIM_START: f32 = 0.9;
    let uv = unit(p);
    let row = (uv.y / ROW_SPACING).floor() as i32;

    // Search from the topmost overlapping row so its circle wins
    for circle_row in ((row - NEIGHBOR_ROWS)..=(row + NEIGHBOR_ROWS)).rev() {
        let dy = uv.y - circle_row as f32 * ROW_SPACING;
        if dy.abs() > CIRCLE_RADIUS {
            continue;
        }

        let shift = if circle_row.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
        let dx = uv.x - ((uv.x - shift).round() + shift);
        let radial_fraction = (dx * dx + dy * dy).sqrt() / CIRCLE_RADIUS;
        if radial_fraction <= 1.0 {
            let arcs = 0.5 + 0.5 * (std::f32::consts::TAU * ARC_COUNT * radial_fraction).cos();
            let rim = 1.0 - 0.7 * smoothstep(RIM_START, 1.0, radial_fraction);
            return ((0.3 + 0.7 * arcs) * rim).clamp(0.0, 1.0);
        }
    }

    0.0
}

/// Herringbone planks three times as long as wide, with gaps (0 = gap), each with a tone and a
/// grain of its own. Six plank widths make a repeat, so the pattern, tones included, repeats
/// every unit.
fn herringbone_planks(p: Vec3, seed: u32) -> f32 {
    const LENGTH_IN_WIDTHS: i32 = 3;
    const CELLS: i32 = 2 * LENGTH_IN_WIDTHS;
    const GAP_WIDTH: f32 = 0.015;
    const GAP_FADE_END: f32 = 0.06;
    let position = unit(p) * CELLS as f32;
    let (column, row) = (position.x.floor() as i32, position.y.floor() as i32);
    let local = position - Vec2::new(column as f32, row as f32);

    // Alternate a full horizontal plank and a full vertical plank along the diagonal
    let phase = (column - row).rem_euclid(CELLS);
    let (along, across, origin_column, origin_row, orientation) = if phase < LENGTH_IN_WIDTHS {
        (phase as f32 + local.x, local.y, column - phase, row, 0)
    } else {
        let step = CELLS - 1 - phase;
        (step as f32 + local.y, local.x, column, row - step, 1)
    };
    let along = along / LENGTH_IN_WIDTHS as f32;

    // Distance to the plank's border, in plank widths
    let edge = across.min(1.0 - across).min(along.min(1.0 - along) * LENGTH_IN_WIDTHS as f32);
    let body = smoothstep(GAP_WIDTH, GAP_FADE_END, edge);
    let tone = noise::cell_hash(origin_column.rem_euclid(CELLS), origin_row.rem_euclid(CELLS), orientation, seed);
    let grain = noise::value([along * 3.0, across * 28.0, tone * 31.0], seed ^ 0x7B);
    body * (0.55 + 0.25 * tone + 0.2 * grain)
}

/// A halftone screen: bright dots on a grid turned 45 degrees, sixteen dots apart per unit, sized
/// by a cloudy tone. Each dot keeps the tone at its center and the tone repeats every unit, so the
/// pattern does too.
fn halftone_dots(p: Vec3, seed: u32) -> f32 {
    // An even grid makes both axes repeat after one unit
    const GRID_SCALE: f32 = 16.0;
    const MIN_RADIUS: f32 = 0.12;
    const RADIUS_VARIATION: f32 = 0.6;
    const EDGE_FEATHER: f32 = 0.05;
    const TONE_OCTAVES: [(i32, f32); 3] = [(3, 0.55), (6, 0.3), (12, 0.15)];
    let point = unit(p) * GRID_SCALE;

    // Max coverage from the four surrounding centers keeps overlapping dots continuous
    let cell = Vec2::new((point.x + point.y) * 0.5, (point.x - point.y) * 0.5).floor();
    let mut coverage = 0.0f32;
    for offset in [Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::ONE] {
        let corner = cell + offset;
        let center = Vec2::new(corner.x + corner.y, corner.x - corner.y);
        let distance = (point - center).length();

        // Skip tone sampling where even the largest dot cannot reach
        if distance >= MIN_RADIUS + RADIUS_VARIATION + EDGE_FEATHER {
            continue;
        }

        // Stretch the noise because weighted octaves cluster around the middle
        let tone = smoothstep(0.25, 0.75, lattice_fbm(center / GRID_SCALE, &TONE_OCTAVES, seed));
        let radius = MIN_RADIUS + RADIUS_VARIATION * tone;
        coverage = coverage.max(1.0 - smoothstep(radius - EDGE_FEATHER, radius + EDGE_FEATHER, distance));
    }

    coverage
}

/// Raised diamonds on a twill ground, four by four per unit. Each diamond is a set of concentric
/// rings with a dark border; the ground shows fine diagonal ribs. Repeats every unit.
fn diamond_brocade(p: Vec3) -> f32 {
    use std::f32::consts::{PI, TAU};

    const DIAMONDS: f32 = 4.0;
    const RIBS_PER_UNIT: f32 = 40.0;
    const MOTIF_RADIUS: f32 = 0.46;
    const MOTIF_FEATHER: f32 = 0.03;
    const MOTIF_RING_COUNT: f32 = 2.5;
    let point = unit(p) * DIAMONDS;
    let local = point - point.floor() - Vec2::splat(0.5);
    let diamond_distance = local.x.abs() + local.y.abs();

    // The twill has a whole number of diagonal ribs per unit
    let ribs = 0.5 + 0.5 * (TAU * RIBS_PER_UNIT * (point.x + point.y) / DIAMONDS).sin();
    let ground = 0.22 + 0.12 * ribs;

    // Only the twill ground is visible beyond the motif
    if diamond_distance >= MOTIF_RADIUS {
        return ground;
    }

    let inside = 1.0 - smoothstep(MOTIF_RADIUS - MOTIF_FEATHER, MOTIF_RADIUS, diamond_distance);
    let rings = 0.5 + 0.5 * (TAU * MOTIF_RING_COUNT * diamond_distance / MOTIF_RADIUS).cos();
    let rise = (PI * 0.5 * (1.0 - diamond_distance / MOTIF_RADIUS)).sin().max(0.0);
    let motif = (0.45 + 0.35 * rise + 0.2 * rings) * (0.9 + 0.1 * ribs);
    (ground + (motif - ground) * inside).clamp(0.0, 1.0)
}

/// Truchet tiles: in each of eight by eight cells per unit, two quarter-circle arcs join the
/// middles of its sides, around one pair of opposite corners or the other. The arcs meet across
/// cells into winding round tubes. The choice comes from the cell's index wrapped to the grid,
/// so the pattern repeats every unit.
fn truchet_arcs(p: Vec3, seed: u32) -> f32 {
    const CELLS: i32 = 8;
    const TUBE_RADIUS: f32 = 0.13;
    let point = unit(p) * CELLS as f32;
    let (column, row) = (point.x.floor() as i32, point.y.floor() as i32);
    let local = point - Vec2::new(column as f32, row as f32);

    let flipped = noise::cell_hash(column.rem_euclid(CELLS), row.rem_euclid(CELLS), 0, seed) > 0.5;
    let corners = if flipped { [Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)] } else { [Vec2::ZERO, Vec2::ONE] };
    let offset = corners.iter().map(|corner| ((local - *corner).length() - 0.5).abs()).fold(f32::MAX, f32::min);

    let across = (offset / TUBE_RADIUS).min(1.0);
    0.05 + 0.9 * (1.0 - across * across).sqrt()
}

/// `values` raised to a triplanar `sharpness` (at least 1): whole sharpnesses up to 16 (the usual
/// ones) by repeated squaring, a few multiplies instead of three `powf` calls per texel.
#[inline]
fn sharpen(values: Vec3, sharpness: f32) -> Vec3 {
    const MAX_INTEGER_SHARPNESS: f32 = 16.0;
    let exponent = sharpness.max(1.0);
    if exponent > MAX_INTEGER_SHARPNESS || exponent.fract() != 0.0 {
        return values.powf(exponent);
    }

    let (mut remaining, mut base, mut result) = (exponent as u32, values, Vec3::ONE);
    while remaining > 0 {
        if remaining & 1 == 1 {
            result *= base;
        }
        base *= base;
        remaining >>= 1;
    }

    result
}

/// A triplanar plane whose weight, times the gain applied after blending, stays under this is
/// not evaluated: even two skipped planes change a value by less than 1/65536, under the u16
/// step layers are stored at. With sharp blends most texels need one plane, not three.
const TRIPLANAR_SKIP: f32 = 1.0 / 131_072.0;

/// Normalized triplanar blend weights (YZ, XZ, XY planes) for normal `nrm`; planes too light
/// to change a 0..1 value scaled by `gain` afterwards weigh 0 (see [`TRIPLANAR_SKIP`]).
fn triplanar_weights(nrm: Vec3, sharpness: f32, gain: f32) -> Vec3 {
    let weights = normalized_triplanar_weights(nrm, sharpness);
    let skip_threshold = TRIPLANAR_SKIP / gain.max(1.0);
    Vec3::select(weights.cmplt(Vec3::splat(skip_threshold)), Vec3::ZERO, weights)
}

fn normalized_triplanar_weights(normal: Vec3, sharpness: f32) -> Vec3 {
    let normal = normal.abs();
    let strongest = normal.max_element();
    if strongest == 0.0 || !normal.is_finite() {
        return Vec3::ZERO;
    }

    // Keep the strongest weight at one so sharp diagonal blends cannot underflow
    let weights = sharpen(normal / strongest, sharpness);
    weights / weights.element_sum()
}

fn shape(pattern: &Procedural, value: f32) -> f32 {
    let adjusted = ((value - 0.5) * pattern.contrast.max(0.0) + 0.5 + pattern.balance).clamp(0.0, 1.0);
    if pattern.invert { 1.0 - adjusted } else { adjusted }
}

/// `point` turned by `rotation` degrees, scaled by `tiling` and moved by `offset`, as UV transformations place it
fn place(point: Vec2, tiling: Vec2, rotation: f32, offset: Vec2) -> Vec2 {
    placement(tiling, rotation, offset)(point)
}

/// [`place`] with its turn's sine and cosine worked out once, for a projection that places several points
pub(crate) fn placement(tiling: Vec2, rotation: f32, offset: Vec2) -> impl Fn(Vec2) -> Vec2 {
    let (sin, cos) = rotation.to_radians().sin_cos();
    move |point| Vec2::new(point.x * cos - point.y * sin, point.x * sin + point.y * cos) * tiling + offset
}

/// `point` placed as a 3D projection's UV transformation `uv` lays its image
fn place_uv(point: Vec2, uv: &UvTransform) -> Vec2 {
    uv_placement(uv)(point)
}

/// [`place_uv`] worked out once for every point a projection places
fn uv_placement(uv: &UvTransform) -> impl Fn(Vec2) -> Vec2 {
    placement(Vec2::from(uv.tiling), uv.rotation, Vec2::from(uv.offset))
}

/// 0..1 around a cylindrical projection's axis (`depth` over `across`), over an arc of `angle` degrees centred on the front
fn around(across: f32, depth: f32, angle: f32) -> f32 {
    let turn = depth.atan2(across) / std::f32::consts::TAU + 0.5;

    // A full turn reads the turn untouched, as projects saved before the angle existed did, bit for bit
    if angle == FULL_TURN_DEGREES { turn } else { (turn - 0.5) * FULL_TURN_DEGREES / angle.max(MIN_CYLINDER_ANGLE) + 0.5 }
}

/// Narrowest arc in degrees a cylindrical projection wraps its image round, so a zero angle never divides by zero
pub const MIN_CYLINDER_ANGLE: f32 = *Projection::CYLINDER_ANGLE_RANGE.start();

/// How many pattern tiles a procedural lays round and down a spherical projection, so its cells stay near square
const SPHERE_PATTERN_TILES: Vec2 = Vec2::new(4.0, 2.0);

/// How many pattern tiles a procedural lays round a cylindrical projection's axis
const CYLINDER_PATTERN_TILES: f32 = 4.0;

/// Smallest frame side or depth (world units), so a flat frame never divides by zero
const MIN_FRAME_EXTENT: f32 = 1e-6;

/// Where `coords` lies round a spherical projection: 0..1 round its pole axis and down from its top
fn spherical_uv(projector: &Projector, offset: &[f32; 3], coords: &Coords) -> Vec2 {
    let direction = (projector_local(projector, coords).pos + Vec3::from(*offset)).try_normalize().unwrap_or(Vec3::Y);
    let u = direction.z.atan2(direction.x) / std::f32::consts::TAU + 0.5;
    let v = direction.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    Vec2::new(u, v)
}

/// The axis index (0 X, 1 Y, 2 Z) a cylindrical projection's stored `axis` wraps round: any value but X or Z reads as Y
pub fn cylinder_axis_index(axis: u8) -> usize {
    match axis {
        0 => 0,
        2 => 2,
        _ => 1,
    }
}

/// Where `coords` lies for a cylindrical projection round `axis`: 0..1 round the axis over `angle` degrees, and its height along the axis
fn cylinder_coords(axis: u8, projector: &Projector, offset: &[f32; 3], angle: f32, coords: &Coords) -> (f32, f32) {
    let point = projector_local(projector, coords).pos + Vec3::from(*offset);
    let (across, height, depth) = match cylinder_axis_index(axis) {
        0 => (point.y, point.x, point.z),
        2 => (point.x, point.z, point.y),
        _ => (point.x, point.y, point.z),
    };
    (around(across, depth, angle), height)
}

/// Smallest mesh radius projections measure in, so a flat or empty mesh never divides by zero
const MIN_MESH_RADIUS: f32 = 1e-4;

/// Nearest to zero a projection scale or stretch gets, so it never divides by zero
const MIN_STRETCH: f32 = 1e-6;

/// The mesh's centre and its radius (half the diagonal) from its bounds
pub fn mesh_frame((lo, hi): (Vec3, Vec3)) -> (Vec3, f32) {
    ((lo + hi) * 0.5, ((hi - lo).length() * 0.5).max(MIN_MESH_RADIUS))
}

/// `value` kept at least [`MIN_STRETCH`] from zero, on its own side
pub fn nonzero_stretch(value: f32) -> f32 {
    value.signum() * value.abs().max(MIN_STRETCH)
}

/// A projector's per-axis scale with no axis at zero, each keeping its sign
pub fn projector_stretch(projector: &Projector) -> Vec3 {
    Vec3::from(projector.scale).map(nonzero_stretch)
}

/// The middle of a frame's 0..1 texture coordinates: a frame's image turns round it
pub const FRAME_UV_CENTER: Vec2 = Vec2::splat(0.5);

/// `point` (0..1 across and down a frame) placed as `uv` lays a frame's image: turned round the frame's centre, then tiled and moved from its top left
pub fn place_frame_uv(point: Vec2, uv: &UvTransform) -> Vec2 {
    // An unplaced image reads `point` itself, bit for bit as decals saved before image placement did
    if *uv == UvTransform::default() {
        return point;
    }

    let tiling = Vec2::from(uv.tiling);
    place(point - FRAME_UV_CENTER, tiling, uv.rotation, Vec2::from(uv.offset)) + FRAME_UV_CENTER * tiling
}

/// The image placement a frame needs to lay images exactly as `uv` does from the top left (a frame turns its image round its centre)
pub fn frame_uv_from_corner(uv: &UvTransform) -> UvTransform {
    let offset = place_uv(FRAME_UV_CENTER, uv) - FRAME_UV_CENTER * Vec2::from(uv.tiling);
    UvTransform { offset: offset.to_array(), ..*uv }
}

/// `coords` inside a projector turned and scaled by `projector`; the default projector returns `coords` unchanged
pub fn projector_local(projector: &Projector, coords: &Coords) -> Coords {
    if !projector.turns() {
        return *coords;
    }

    let [x, y, z] = projector.rotation.map(f32::to_radians);
    let to_local = Mat3::from_euler(EulerRot::ZYX, z, y, x).transpose();
    let scale = projector_stretch(projector);
    let nrm = (to_local * coords.nrm * scale).try_normalize().unwrap_or(coords.nrm);
    Coords { pos: to_local * coords.pos / scale, nrm, ..*coords }
}

/// Evaluate a procedural through a projection, at full resolution and without images
pub fn procedural(pattern: &Procedural, projection: &Projection, coords: &Coords) -> f32 {
    procedural_in(pattern, projection, coords, &PatternContext::default())
}

/// Evaluate a procedural through a projection with what `context` gives it: its resolution and the images it reads
pub fn procedural_in(pattern: &Procedural, projection: &Projection, coords: &Coords, context: &PatternContext) -> f32 {
    let pattern_scale = pattern.scale.max(MIN_PATTERN_SCALE);

    // Evaluate two-dimensional patterns on a slice of the volume
    let flat = |point: Vec2| plane_value(pattern, point.extend(0.0), context);
    let value = match projection {
        Projection::UvTile => flat((coords.uv - coords.uv.floor()) * pattern_scale),
        Projection::Uv { scale, rotation, offset } => flat(place(coords.uv, Vec2::from(*scale), *rotation, Vec2::from(*offset)) * pattern_scale),
        Projection::Triplanar { scale, sharpness, offset, uv, projector } => {
            let coords = &projector_local(projector, coords);
            let point = coords.pos * *scale * pattern_scale + Vec3::from(*offset);
            let place_plane = uv_placement(uv);

            // Shape scales the blend by the contrast
            let weights = triplanar_weights(coords.nrm, *sharpness, pattern.contrast);
            let planes = [(weights.x, Vec2::new(point.z, point.y)), (weights.y, Vec2::new(point.x, point.z)), (weights.z, Vec2::new(point.x, point.y))];
            planes.into_iter().filter(|(weight, _)| *weight > 0.0).map(|(weight, plane)| flat(place_plane(plane)) * weight).sum()
        }

        // The coordinates are already in the placed or warped frame (see [`placed_uv`] and [`crate::warp::warp_uv`])
        Projection::Planar { uv, placed: Some(_), .. } | Projection::Warp { uv, .. } => flat(place_frame_uv(coords.uv, uv) * pattern_scale),
        Projection::Planar { scale, axis, offset, uv, projector, .. } => {
            let point = projector_local(projector, coords).pos * *scale * pattern_scale + Vec3::from(*offset);
            let plane = match axis {
                0 => Vec2::new(point.z, point.y),
                1 => Vec2::new(point.x, point.z),
                _ => Vec2::new(point.x, point.y),
            };
            flat(place_uv(plane, uv))
        }
        Projection::Spherical { scale, uv, offset, projector } => {
            let sphere = spherical_uv(projector, offset, coords);
            flat(place_uv(sphere * SPHERE_PATTERN_TILES * *scale * pattern_scale, uv))
        }
        Projection::Cylindrical { scale, axis, uv, offset, projector, angle } => {
            let (around_axis, height) = cylinder_coords(*axis, projector, offset, *angle, coords);
            flat(place_uv(Vec2::new(around_axis * CYLINDER_PATTERN_TILES, height) * *scale * pattern_scale, uv))
        }
    };

    shape(pattern, value)
}

/// The texels per side `pattern`'s resolution draws it at for a UV tile of `uv_tile` texels per side in a texture set of `texture_set`, when that is coarser than the UV tile (a finer image changes nothing: patterns are evaluated per texel)
pub fn pattern_pixels(pattern: &Procedural, uv_tile: u32, texture_set: u32) -> Option<u32> {
    if !pattern.resolution.can_reduce() {
        return None;
    }

    let size = pattern.resolution.output_size(uv_tile, texture_set);
    (size < uv_tile).then_some(size)
}

/// The pattern at `point` of its plane, read from an image of `context.pixels` texels per repeat (bilinear) when given
fn plane_value(pattern: &Procedural, point: Vec3, context: &PatternContext) -> f32 {
    let cells = context.pixels.map(|pixels| pixels as f32 / pattern.scale.max(MIN_PATTERN_SCALE)).filter(|cells| cells.is_finite() && *cells > 0.0);
    let Some(cells) = cells else { return pattern_value(pattern, point, context) };

    let grid = point.truncate() * cells - 0.5;

    // Non-finite grid coordinates cannot identify bilinear neighbors
    if !grid.is_finite() {
        return pattern_value(pattern, point, context);
    }

    let base = grid.floor();
    let fraction = grid - base;
    let at = |across: f32, down: f32| pattern_value(pattern, ((base + Vec2::new(across, down) + 0.5) / cells).extend(point.z), context);
    let top_left = at(0.0, 0.0);
    let bottom_left = at(0.0, 1.0);
    let top = top_left + (at(1.0, 0.0) - top_left) * fraction.x;
    let bottom = bottom_left + (at(1.0, 1.0) - bottom_left) * fraction.x;
    top + (bottom - top) * fraction.y
}

/// The pattern at `point` of its plane, exactly
fn pattern_value(pattern: &Procedural, point: Vec3, context: &PatternContext) -> f32 {
    match (&pattern.tile, pattern.kind) {
        (Some(tile), ProceduralKind::TileGenerator) => crate::tile_pattern::value(tile, point.truncate(), pattern.seed, context.images),
        _ => raw(pattern.kind, point, pattern.seed),
    }
}

/// Texture coordinates (and blend weights) at which an image is sampled for a projection
/// (unused slots weigh 0; no allocation per texel).
pub fn image_samples(projection: &Projection, coords: &Coords) -> [(Vec2, f32); 3] {
    let one = |uv: Vec2| [(uv, 1.0), (Vec2::ZERO, 0.0), (Vec2::ZERO, 0.0)];
    match projection {
        Projection::UvTile => one(coords.uv - coords.uv.floor()),
        Projection::Uv { scale, rotation, offset } => one(place(coords.uv, Vec2::from(*scale), *rotation, Vec2::from(*offset))),
        Projection::Triplanar { scale, sharpness, offset, uv, projector } => {
            let coords = &projector_local(projector, coords);
            let point = coords.pos * *scale + Vec3::from(*offset);
            let place_plane = uv_placement(uv);

            // Float images can exceed 1, so even a tiny plane weight can carry visible detail
            let weights = normalized_triplanar_weights(coords.nrm, *sharpness);
            [
                (place_plane(Vec2::new(point.z, -point.y)), weights.x),
                (place_plane(Vec2::new(point.x, point.z)), weights.y),
                (place_plane(Vec2::new(point.x, -point.y)), weights.z),
            ]
        }
        Projection::Planar { uv, placed: Some(_), .. } | Projection::Warp { uv, .. } => one(place_frame_uv(coords.uv, uv)),
        Projection::Planar { scale, axis, offset, uv, projector, .. } => {
            let point = projector_local(projector, coords).pos * *scale + Vec3::from(*offset);
            let plane = match axis {
                0 => Vec2::new(point.z, -point.y),
                1 => Vec2::new(point.x, point.z),
                _ => Vec2::new(point.x, -point.y),
            };
            one(place_uv(plane * 0.5 + Vec2::splat(0.5), uv))
        }
        Projection::Spherical { scale, uv, offset, projector } => one(place_uv(spherical_uv(projector, offset, coords) * *scale, uv)),
        Projection::Cylindrical { scale, axis, uv, offset, projector, angle } => {
            let (around_axis, height) = cylinder_coords(*axis, projector, offset, *angle, coords);
            one(place_uv(Vec2::new(around_axis, 0.5 - height * 0.5) * *scale, uv))
        }
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

/// World point `point` in frame `frame` turned and scaled by `projector`: across and up in frame sizes, out along its normal in world units
pub fn frame_local(frame: &Decal, projector: &Projector, point: Vec3) -> Option<Vec3> {
    let (origin, right, up, normal) = decal_axes(frame)?;
    let from_origin = point - origin;
    let mut local = Vec3::new(from_origin.dot(right), from_origin.dot(up), from_origin.dot(normal));
    if projector.turns() {
        local = projector_local(projector, &Coords { uv: Vec2::ZERO, pos: local, nrm: Vec3::Z }).pos;
    }

    frame_position(frame, local)
}

/// Read the position and facing of one sample when a placed frame culls by angle
pub(crate) fn frame_sample(frame: &Decal, projector: &Projector, position: Vec3, surface_normal: Vec3) -> Option<(Vec3, f32)> {
    let (origin, right, up, normal) = decal_axes(frame)?;
    let from_origin = position - origin;
    let local = Vec3::new(from_origin.dot(right), from_origin.dot(up), from_origin.dot(normal));

    if !projector.turns() {
        return Some((frame_position(frame, local)?, surface_normal.dot(normal)));
    }

    let normal = Vec3::new(surface_normal.dot(right), surface_normal.dot(up), surface_normal.dot(normal));
    let sample = projector_local(projector, &Coords { uv: Vec2::ZERO, pos: local, nrm: normal });
    Some((frame_position(frame, sample.pos)?, sample.nrm.z))
}

fn frame_position(frame: &Decal, local: Vec3) -> Option<Vec3> {
    let local = Vec3::new(local.x / frame.size[0].max(MIN_FRAME_EXTENT), local.y / frame.size[1].max(MIN_FRAME_EXTENT), local.z);
    local.is_finite().then_some(local)
}

/// How squarely a surface with normal `nrm` faces frame `frame` turned by `projector` (the cosine of the turn)
pub fn frame_facing(frame: &Decal, projector: &Projector, nrm: Vec3) -> f32 {
    let Some((_, right, up, normal)) = decal_axes(frame) else { return 0.0 };
    if !projector.turns() {
        return nrm.dot(normal);
    }

    let turned = Vec3::new(nrm.dot(right), nrm.dot(up), nrm.dot(normal));
    projector_local(projector, &Coords { uv: Vec2::ZERO, pos: Vec3::ZERO, nrm: turned }).nrm.z
}

/// Texture coordinates (v down) of a point at `local` in its frame (see [`frame_local`])
pub fn placed_uv(local: Vec3) -> Vec2 {
    Vec2::new(local.x + 0.5, 0.5 - local.y)
}

/// How much frame `frame` covers a point at `local` when cropped to its shape: 0 outside the frame or past its depth, fading near the depth
pub fn frame_crop(frame: &Decal, local: Vec3) -> f32 {
    if local.x.abs() > 0.5 || local.y.abs() > 0.5 {
        return 0.0;
    }

    depth_fade(local.z, frame.depth)
}

/// Where a frame's depth fade starts, as a share of its depth
pub const DEPTH_FADE_START: f32 = 0.75;

/// How much a frame reaching `depth` in front of and behind it covers a point `z` out from it: 0 past the depth, fading near it
pub fn depth_fade(z: f32, depth: f32) -> f32 {
    let (distance, depth) = (z.abs(), depth.max(MIN_FRAME_EXTENT));
    if distance > depth {
        return 0.0;
    }

    1.0 - smoothstep(DEPTH_FADE_START * depth, depth, distance)
}

/// World-space noise used by generators (seamless across UV islands).
pub fn world_noise(pos: Vec3, scale: f32, seed: u32) -> f32 {
    noise::fbm((pos * scale.max(1e-3)).to_array(), 4, seed, noise::perlin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::{OutputResolution, ResolutionMode};

    #[test]
    fn a_pattern_is_drawn_coarser_only_below_the_uv_tiles_resolution() {
        let with = |resolution: OutputResolution| pattern_pixels(&Procedural { resolution, ..Procedural::new(ProceduralKind::Clouds) }, 1024, 2048);
        assert_eq!(with(OutputResolution::default()), None, "by default a pattern is evaluated per texel");
        assert_eq!(with(OutputResolution { factor: -1, ..OutputResolution::default() }), Some(512));
        assert_eq!(with(OutputResolution { mode: ResolutionMode::Custom, custom: 256, factor: 0 }), Some(256));
        assert_eq!(with(OutputResolution { mode: ResolutionMode::Custom, custom: 2048, factor: 0 }), None, "a finer image than the UV tile changes nothing");
    }

    #[test]
    fn a_coarser_pattern_reads_its_cells_bilinearly() {
        let pattern = Procedural { scale: 1.0, ..Procedural::new(ProceduralKind::Clouds) };
        let coarse = PatternContext { pixels: Some(4), ..PatternContext::default() };
        let at = |x: f32, context: &PatternContext| {
            procedural_in(&pattern, &Projection::default(), &Coords { uv: Vec2::new(x, 0.375), pos: Vec3::ZERO, nrm: Vec3::Y }, context)
        };
        let exact = PatternContext::default();

        // Coarse reads are exact at cell centers and blend between them
        assert!((at(0.375, &coarse) - at(0.375, &exact)).abs() < 1e-5);

        let blended = (at(0.125, &exact) + at(0.375, &exact)) * 0.5;
        assert!((at(0.25, &coarse) - blended).abs() < 1e-5, "{} vs {blended}", at(0.25, &coarse));
        assert!((at(0.25, &exact) - blended).abs() > 1e-3, "the exact pattern is not the blend, so the coarse read shows");
    }

    /// A decal saved before image placement reads its frame coordinates bit for bit as it did: its image at them, its procedural at them times the pattern scale
    #[test]
    fn an_unplaced_decal_reads_its_frame_coordinates_bit_for_bit() {
        let frame = Decal {
            center: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [1.0; 2],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let decal = Projection::decal(frame);
        let pattern = Procedural { scale: 3.0, ..Procedural::new(ProceduralKind::Gradient) };
        for across in 0..=1000 {
            for down in [0.0, 0.1, 0.37, 1.0] {
                let uv = Vec2::new(across as f32 / 1000.0, down);
                let coords = Coords { uv, pos: Vec3::ZERO, nrm: Vec3::Z };

                assert_eq!(image_samples(&decal, &coords)[0].0, uv, "image at {uv}");

                let expected = shape(&pattern, raw(pattern.kind, (uv * pattern.scale).extend(0.0), pattern.seed));
                assert_eq!(procedural(&pattern, &decal, &coords).to_bits(), expected.to_bits(), "procedural at {uv}");
            }
        }
    }

    /// A frame's image turns round the frame's centre: the centre reads the same texel at any Rotation, and an unturned image reads exactly as before for any Tiling and Offset
    #[test]
    fn a_frame_image_turns_round_the_frame_centre() {
        let frame = Decal {
            center: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [1.0; 2],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let at = |uv: Vec2| Coords { uv, pos: Vec3::ZERO, nrm: Vec3::Z };
        let sample = |rotation: f32, tiling: [f32; 2], offset: [f32; 2], uv: Vec2| {
            image_samples(
                &Projection::Warp {
                    frame,
                    corners: Projection::FRAME_CORNERS,
                    depth: 0.1,
                    uv: UvTransform { tiling, rotation, offset },
                    projector: Projector::default(),
                },
                &at(uv),
            )[0]
            .0
        };
        for rotation in [37.0, 90.0, 180.0] {
            let center = sample(rotation, [1.0; 2], [0.0; 2], FRAME_UV_CENTER);

            assert!((center - FRAME_UV_CENTER).length() < 1e-5, "{rotation}°: the centre stays put, not swung round the top left: {center}");
        }
        let corner = sample(90.0, [1.0; 2], [0.0; 2], Vec2::ZERO);
        assert!((corner - Vec2::new(1.0, 0.0)).length() < 1e-5, "a quarter turn brings the top left to the top right: {corner}");

        let placed = Projection::Planar {
            scale: 1.0,
            axis: 2,
            offset: [0.0; 3],
            uv: UvTransform { rotation: 90.0, ..UvTransform::default() },
            projector: Projector::default(),
            placed: Some(frame),
        };
        assert!((image_samples(&placed, &at(FRAME_UV_CENTER))[0].0 - FRAME_UV_CENTER).length() < 1e-5, "a placed planar frame turns the same way");

        for (tiling, offset, uv) in [([2.0, 3.0], [0.25, -0.5], Vec2::new(0.1, 0.8)), ([1.0, 1.0], [0.0, 0.0], Vec2::new(0.7, 0.2))] {
            let before = uv * Vec2::from(tiling) + Vec2::from(offset);
            assert!((sample(0.0, tiling, offset, uv) - before).length() < 1e-6, "unturned: as before");
        }
    }

    #[test]
    fn every_pattern_and_projection_stays_in_range() {
        let projections = [
            Projection::default(),
            Projection::Triplanar { scale: 1.0, sharpness: 4.0, offset: [0.0; 3], uv: Default::default(), projector: Default::default() },
            Projection::Planar { scale: 1.0, axis: 1, offset: [0.0; 3], uv: Default::default(), projector: Default::default(), placed: None },
            Projection::Spherical { scale: 1.0, uv: Default::default(), offset: [0.0; 3], projector: Default::default() },
            Projection::Cylindrical { scale: 1.0, axis: 1, uv: Default::default(), offset: [0.0; 3], projector: Default::default(), angle: FULL_TURN_DEGREES },
            Projection::decal(Decal {
                center: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
                up: [0.0, 1.0, 0.0],
                size: [1.0; 2],
                rotation: 0.0,
                offset: [0.0; 2],
                depth: 1.0,
                symmetry: None,
                flip: [false; 2],
            }),
        ];
        for kind in ProceduralKind::ALL {
            let pattern = Procedural::new(kind);
            for projection in &projections {
                for index in 0..50 {
                    let position = index as f32 * 0.13;
                    let coords = Coords {
                        uv: Vec2::new(position.fract(), (position * 0.7).fract()),
                        pos: Vec3::new(position.sin(), position.cos(), position * 0.1),
                        nrm: Vec3::new(0.3, 0.9, 0.1).normalize(),
                    };
                    let value = procedural(&pattern, projection, &coords);

                    assert!((0.0..=1.0).contains(&value), "{kind:?} {projection:?} {value}");
                }
            }
        }
    }

    /// Skipping light triplanar planes stays within a u16 step of evaluating all three, at any
    /// contrast; near an axis it does skip.
    #[test]
    fn triplanar_plane_skipping_stays_under_one_u16_step() {
        let full = |pattern: &Procedural, sharpness: f32, coords: &Coords| {
            let position = coords.pos * pattern.scale;
            let weights = coords.nrm.abs().powf(sharpness);
            let weights = weights / (weights.x + weights.y + weights.z);
            shape(
                pattern,
                raw(pattern.kind, Vec3::new(position.z, position.y, 0.0), pattern.seed) * weights.x
                    + raw(pattern.kind, Vec3::new(position.x, position.z, 0.0), pattern.seed) * weights.y
                    + raw(pattern.kind, Vec3::new(position.x, position.y, 0.0), pattern.seed) * weights.z,
            )
        };
        let mut largest_difference = 0.0f32;
        for kind in ProceduralKind::ALL {
            for contrast in [1.0, 4.0, 16.0] {
                let pattern = Procedural { contrast, ..Procedural::new(kind) };
                for sharpness in [1.0, 4.0, 8.0, 16.0] {
                    let projection =
                        Projection::Triplanar { scale: 1.0, sharpness, offset: [0.0; 3], uv: UvTransform::default(), projector: Projector::default() };
                    for index in 0..400 {
                        // Cover axis aligned and oblique normals at varied positions
                        let position = index as f32 / 400.0;
                        let normal = Vec3::new(position * 1.3, 1.0, (position * 7.0).sin() * position).normalize();
                        let coords = Coords { uv: Vec2::ZERO, pos: Vec3::new((position * 11.0).sin(), (position * 5.0).cos(), position * 3.0), nrm: normal };
                        largest_difference = largest_difference.max((procedural(&pattern, &projection, &coords) - full(&pattern, sharpness, &coords)).abs());
                    }
                }
            }
        }

        assert!(largest_difference < 1.0 / 65535.0, "worst difference {largest_difference}");

        let near_axis = triplanar_weights(Vec3::new(0.2, 1.0, 0.1).normalize(), 8.0, 1.0);
        assert_eq!((near_axis.x, near_axis.z), (0.0, 0.0), "{near_axis}");

        // An oblique normal skips no plane
        let oblique = triplanar_weights(Vec3::ONE.normalize(), 8.0, 16.0);
        assert!((oblique - Vec3::splat(1.0 / 3.0)).abs().max_element() < 1e-6, "{oblique}");

        for sharpness in [47.0, 64.0] {
            let diagonal = triplanar_weights(Vec3::ONE.normalize(), sharpness, 1.0);
            assert!(diagonal.abs_diff_eq(Vec3::splat(1.0 / 3.0), 1e-6), "sharpness {sharpness}: {diagonal}");
        }
    }

    #[test]
    fn triplanar_image_hardness_preserves_diagonal_coverage() {
        let coords = Coords { uv: Vec2::ZERO, pos: Vec3::ZERO, nrm: Vec3::new(1.0, 0.95, 0.90).normalize() };
        let projection =
            |sharpness| Projection::Triplanar { scale: 1.0, sharpness, offset: [0.0; 3], uv: UvTransform::default(), projector: Projector::default() };

        let soft = image_samples(&projection(1.0), &coords).map(|(_, weight)| weight);
        let hard = image_samples(&projection(64.0), &coords).map(|(_, weight)| weight);

        assert!((hard.into_iter().sum::<f32>() - 1.0).abs() < 1e-6, "{hard:?}");
        assert!(hard[0] > soft[0] && hard[1] < soft[1] && hard[2] < soft[2], "{soft:?} {hard:?}");
    }

    #[test]
    fn triplanar_image_sampling_preserves_bright_float_pixels() {
        let image = crate::ImageRes::new(2, 2, vec![[0.0; 4], [0.0; 4], [0.0; 4], [100_000.0; 4]], false);
        let coords = Coords { uv: Vec2::ZERO, pos: Vec3::new(0.25, 0.25, 0.75), nrm: Vec3::new(0.2, 1.0, 0.1).normalize() };
        let projection = Projection::Triplanar { scale: 1.0, sharpness: 8.0, offset: [0.0; 3], uv: UvTransform::default(), projector: Projector::default() };
        let weights = coords.nrm.abs().powf(8.0);
        let expected = 100_000.0 * weights.x / (weights.x + weights.y + weights.z);

        let actual: f32 =
            image_samples(&projection, &coords).into_iter().filter(|(_, weight)| *weight > 0.0).map(|(uv, weight)| image.sample(uv.x, uv.y)[0] * weight).sum();

        assert!(expected > 0.25, "the light plane must make a visible contribution");
        assert!((actual - expected).abs() < 1.0 / 65535.0, "expected {expected}, got {actual}");
    }

    /// Repeated squaring agrees with `powf` to a few ulps for whole sharpnesses, and falls back
    /// to `powf` for the rest.
    #[test]
    fn sharpen_matches_powf() {
        for index in 0..=100 {
            let values = Vec3::new(index as f32 / 100.0, 1.0 - index as f32 / 100.0, (index as f32 * 0.37).fract());
            for sharpness in [0.0, 1.0, 2.0, 3.0, 4.0, 7.0, 8.0, 16.0] {
                let (actual, expected) = (sharpen(values, sharpness), values.powf(sharpness.max(1.0)));
                assert!((actual - expected).abs().max_element() <= 1e-6, "{values} ^ {sharpness}: {actual} vs {expected}");
            }

            for sharpness in [2.5, 17.0, 64.0] {
                assert_eq!(sharpen(values, sharpness), values.powf(sharpness));
            }
        }
    }

    /// Where frame `frame`'s planar projection lands at `point` facing `nrm`, None where it does not cover
    fn decal_uv(frame: &Decal, point: Vec3, nrm: Vec3) -> Option<(Vec2, f32)> {
        let Projection::Planar { projector, .. } = Projection::decal(*frame) else { return None };
        crate::projector_cover::placed_cover(frame, &projector, point, nrm).filter(|(_, cover)| *cover > 0.0)
    }

    #[test]
    fn decals_cover_their_frame_facing_them() {
        let d = Decal {
            center: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [2.0, 1.0],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let z = Vec3::Z;
        let (uv, w) = decal_uv(&d, Vec3::new(0.5, 0.25, 0.0), z).unwrap();
        assert!((uv - Vec2::new(0.75, 0.25)).length() < 1e-5 && (w - 1.0).abs() < 1e-5, "{uv} {w}");
        assert!(decal_uv(&d, Vec3::new(1.1, 0.0, 0.0), z).is_none(), "outside the frame");
        assert!(decal_uv(&d, Vec3::ZERO, -z).is_none(), "back faces");
        assert!(decal_uv(&d, Vec3::new(0.0, 0.0, 0.6), z).is_none(), "beyond the depth");

        // Rotated a quarter turn and shifted: the frame turns with it
        let r = Decal { rotation: 90.0, offset: [1.0, 0.0], ..d };
        let (uv, _) = decal_uv(&r, Vec3::new(1.0, 0.9, 0.0), z).unwrap();
        assert!((uv.x - 0.95).abs() < 1e-4 && (uv.y - 0.5).abs() < 1e-4, "{uv}");
        let c = decal_corners(&d).unwrap();
        assert!((c[0] - Vec3::new(-1.0, 0.5, 0.0)).length() < 1e-5 && (c[2] - Vec3::new(1.0, -0.5, 0.0)).length() < 1e-5);
        assert!(decal_axes(&Decal { normal: [0.0; 3], ..d }).is_none());
        assert!(decal_uv(&Decal { size: [f32::NAN; 2], ..d }, Vec3::ZERO, z).is_none());
    }

    #[test]
    fn decals_mirror_their_image_inside_the_frame() {
        let d = Decal {
            center: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [2.0, 1.0],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let (left, top) = (Vec3::new(-0.5, 0.25, 0.0), Vec3::Z);
        let uv = |flip: [bool; 2]| decal_uv(&Decal { flip, ..d }, left, top).unwrap().0;
        assert!((uv([false, false]) - Vec2::new(0.25, 0.25)).length() < 1e-5);
        assert!((uv([true, false]) - Vec2::new(0.75, 0.25)).length() < 1e-5, "Flip U mirrors left to right only");
        assert!((uv([false, true]) - Vec2::new(0.25, 0.75)).length() < 1e-5, "Flip V mirrors top to bottom only");
        assert!((uv([true, true]) - Vec2::new(0.75, 0.75)).length() < 1e-5);

        // The frame itself does not move: coverage is the same flipped or not
        let covered = |flip: [bool; 2], p: Vec3| decal_uv(&Decal { flip, ..d }, p, top).map(|(_, w)| w);
        assert_eq!(covered([true, true], left), covered([false, false], left));
        assert_eq!(covered([true, true], Vec3::new(1.1, 0.0, 0.0)), None);
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
        let tilings = [
            ProceduralKind::HexagonTiles,
            ProceduralKind::WaveScales,
            ProceduralKind::HerringbonePlanks,
            ProceduralKind::HalftoneDots,
            ProceduralKind::DiamondBrocade,
            ProceduralKind::TruchetArcs,
        ];

        for kind in tilings {
            for index in 0..400 {
                let p = Vec3::new((index as f32 * 0.137).fract() * 3.0 - 1.0, (index as f32 * 0.071).fract() * 3.0 - 1.0, 0.0);
                for shift in [Vec3::X, Vec3::Y, Vec3::new(-2.0, 3.0, 0.0)] {
                    let (here, there) = (raw(kind, p, 7), raw(kind, p + shift, 7));

                    // Allow coordinate rounding on steep joint edges
                    assert!((here - there).abs() < 2e-3, "{kind:?}: {here} at {p}, {there} at {}", p + shift);
                }
            }
        }
    }

    #[test]
    fn a_non_finite_coarse_grid_falls_back_to_exact_sampling() {
        let coarse = PatternContext { pixels: Some(16), ..PatternContext::default() };
        let exact = PatternContext::default();
        let extremes = [f32::MAX, -f32::MAX, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];

        for kind in ProceduralKind::ALL {
            let pattern = Procedural::new(kind);
            for extreme in extremes {
                for point in [Vec3::new(extreme, 0.125, 0.0), Vec3::new(0.125, extreme, 0.0)] {
                    let expected = plane_value(&pattern, point, &exact);
                    let actual = plane_value(&pattern, point, &coarse);

                    assert!(actual == expected || (actual.is_nan() && expected.is_nan()), "{kind:?} at {point}: {actual} instead of {expected}");
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
            for index in 0..64 {
                let tie = Vec3::new(index as f32 / 32.0, (index * 7 % 32) as f32 / 32.0, 0.0);
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

        // Check the end and midpoint of the horizontal plank starting at the origin
        assert!(at(ProceduralKind::HerringbonePlanks, 0.000_2, 0.5 / 6.0) < 0.05);
        assert!(at(ProceduralKind::HerringbonePlanks, 1.5 / 6.0, 0.5 / 6.0) >= 0.55);
    }

    #[test]
    fn every_pattern_varies() {
        for kind in ProceduralKind::ALL {
            let pattern = Procedural::new(kind);
            let (mut minimum, mut maximum) = (f32::MAX, f32::MIN);
            for i in 0..4096 {
                let uv = Vec2::new((i % 64) as f32 / 64.0, (i / 64) as f32 / 64.0);
                let value = procedural(&pattern, &Projection::default(), &Coords { uv, pos: Vec3::ZERO, nrm: Vec3::Y });
                (minimum, maximum) = (minimum.min(value), maximum.max(value));
            }

            assert!(maximum - minimum > 0.3, "{kind:?} is flat: {minimum}..{maximum}");
        }
    }
}
