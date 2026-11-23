//! Deterministic 3D noise (integer hashing, so results are identical on every platform).

/// PCG3D hash (Jarzynski & Olano 2020).
#[inline]
pub fn pcg3d(x: u32, y: u32, z: u32) -> [u32; 3] {
    let mut v =
        [x.wrapping_mul(1664525).wrapping_add(1013904223), y.wrapping_mul(1664525).wrapping_add(1013904223), z.wrapping_mul(1664525).wrapping_add(1013904223)];
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v = v.map(|a| a ^ (a >> 16));
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v
}

/// Every f32 at least this large is an integer.
const F32_INTEGER_THRESHOLD: f32 = 8_388_608.0;

/// The lattice cell of `p` and the position inside it: `(floor(p) as i32, p − floor(p))`,
/// using hardware floor with SSE4.1 and truncation otherwise to avoid a libm call on baseline
/// x86-64. Bit for bit the same, NaN and huge values included.
#[inline]
fn cell(p: f32) -> (i32, f32) {
    // SSE4.1 floor is cheaper than truncation plus correction and needs no special cases
    if cfg!(target_feature = "sse4.1") || p.is_nan() || p.abs() >= F32_INTEGER_THRESHOLD {
        // Preserve the original result for integral and nonfinite inputs
        let floor_value = p.floor();
        return (floor_value as i32, p - floor_value);
    }

    let truncated = p as i32;
    let cell_index = if (truncated as f32) > p { truncated - 1 } else { truncated };

    // Addition turns negative zero into the positive zero returned by subtraction
    (cell_index, (p - cell_index as f32) + 0.0)
}

#[inline]
fn hash01(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    pcg3d(x as u32 ^ seed.wrapping_mul(0x9E37_79B9), y as u32, z as u32)[0] as f32 / u32::MAX as f32
}

#[inline]
fn smooth(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Value noise in 0..1.
pub fn value(p: [f32; 3], seed: u32) -> f32 {
    let (ix, fx) = cell(p[0]);
    let (iy, fy) = cell(p[1]);
    let (iz, fz) = cell(p[2]);
    let (sx, sy, sz) = (smooth(fx), smooth(fy), smooth(fz));
    let hash = |dx: i32, dy: i32, dz: i32| hash01(ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz), seed);
    lerp(
        lerp(lerp(hash(0, 0, 0), hash(1, 0, 0), sx), lerp(hash(0, 1, 0), hash(1, 1, 0), sx), sy),
        lerp(lerp(hash(0, 0, 1), hash(1, 0, 1), sx), lerp(hash(0, 1, 1), hash(1, 1, 1), sx), sy),
        sz,
    )
}

/// Perlin's 12 edge gradients of the unit cube.
const EDGE_GRADIENTS: [[f32; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

/// The dot product of a hashed edge gradient with `f`. A table, not a `match`: the hash is
/// random, so a branch per case mispredicts most of the time. Multiplying by ±1 and adding 0
/// is exact, so the sum equals the two signed components the gradient picks.
#[inline]
fn grad(x: i32, y: i32, z: i32, seed: u32, f: [f32; 3]) -> f32 {
    let gradient = EDGE_GRADIENTS[(pcg3d(x as u32 ^ seed.wrapping_mul(0x85EB_CA6B), y as u32, z as u32)[1] % 12) as usize];
    gradient[0] * f[0] + gradient[1] * f[1] + gradient[2] * f[2]
}

/// Gradient (Perlin) noise in about 0..1.
pub fn perlin(p: [f32; 3], seed: u32) -> f32 {
    let (ix, fx) = cell(p[0]);
    let (iy, fy) = cell(p[1]);
    let (iz, fz) = cell(p[2]);
    let (sx, sy, sz) = (smooth(fx), smooth(fy), smooth(fz));
    let g =
        |dx: i32, dy: i32, dz: i32| grad(ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz), seed, [fx - dx as f32, fy - dy as f32, fz - dz as f32]);
    let v = lerp(
        lerp(lerp(g(0, 0, 0), g(1, 0, 0), sx), lerp(g(0, 1, 0), g(1, 1, 0), sx), sy),
        lerp(lerp(g(0, 0, 1), g(1, 0, 1), sx), lerp(g(0, 1, 1), g(1, 1, 1), sx), sy),
        sz,
    );
    (v * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// Worley (cellular) noise: distance to the nearest feature point, about 0..1.
pub fn worley(p: [f32; 3], seed: u32) -> f32 {
    cellular(p, seed).0
}

/// Cellular noise: distances to the nearest and second-nearest feature points (each about
/// 0..1) and a random 0..1 id of the nearest point's cell (for per-cell variation).
pub fn cellular(p: [f32; 3], seed: u32) -> (f32, f32, f32) {
    let (ix, fx) = cell(p[0]);
    let (iy, fy) = cell(p[1]);
    let (iz, fz) = cell(p[2]);
    let (mut f1, mut f2, mut id) = (f32::MAX, f32::MAX, 0u32);
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let h = pcg3d(ix.wrapping_add(dx) as u32 ^ seed.wrapping_mul(0xC2B2_AE35), iy.wrapping_add(dy) as u32, iz.wrapping_add(dz) as u32);
                let o = h.map(|v| v as f32 / u32::MAX as f32);
                let d = [dx as f32 + o[0] - fx, dy as f32 + o[1] - fy, dz as f32 + o[2] - fz];
                let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                if d2 < f1 {
                    (f2, f1, id) = (f1, d2, h[0] ^ h[1]);
                } else if d2 < f2 {
                    f2 = d2;
                }
            }
        }
    }
    (f1.sqrt().min(1.0), f2.sqrt().min(1.0), id as f32 / u32::MAX as f32)
}

/// Random 0..1 value of an integer lattice cell (per-brick or per-tile variation).
pub fn cell_hash(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    hash01(x, y, z, seed)
}

/// Fractal sum of `octaves` layers of `base` noise, normalized to 0..1. Generic, not a `fn`
/// pointer, so each base noise is inlined into the octave loop.
#[inline]
pub fn fbm(p: [f32; 3], octaves: u32, seed: u32, base: impl Fn([f32; 3], u32) -> f32) -> f32 {
    // Bound work even for untrusted octave counts
    const MAX_OCTAVES: u32 = 12;

    // Offset each octave in seed and position to decorrelate its lattice
    const OCTAVE_SEED_STRIDE: u32 = 31;
    const OCTAVE_POSITION_OFFSET: f32 = 17.13;
    let (mut sum, mut amplitude, mut normalization, mut position) = (0.0, 0.5, 0.0, p);

    for octave in 0..octaves.clamp(1, MAX_OCTAVES) {
        sum += amplitude * base(position, seed.wrapping_add(octave * OCTAVE_SEED_STRIDE));
        normalization += amplitude;
        position = position.map(|value| value * 2.0 + OCTAVE_POSITION_OFFSET);
        amplitude *= 0.5;
    }

    sum / normalization
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noises_are_deterministic_and_in_range() {
        for i in 0..500 {
            let p = [i as f32 * 0.37, i as f32 * 0.11 - 3.0, (i % 17) as f32 * 0.53];
            for f in [value, perlin, worley] {
                let a = f(p, 3);
                assert!((0.0..=1.0).contains(&a), "{a}");
                assert_eq!(a, f(p, 3));
            }
            let a = fbm(p, 5, 1, value);
            assert!((0.0..=1.0).contains(&a));
            let (f1, f2, id) = cellular(p, 3);
            assert!(f1 <= f2 && (0.0..=1.0).contains(&f2) && (0.0..=1.0).contains(&id), "{f1} {f2} {id}");
        }
    }

    /// FNV-1a of the bits of every noise over points spread across signs, lattice boundaries,
    /// −0 and huge values: the constant was taken from the `floor`/`match`/`fn`-pointer
    /// implementation, so the faster one must reproduce every pattern bit for bit. The points
    /// use only basic arithmetic (no `sin`, whose last bits differ between platforms).
    #[test]
    fn noises_match_the_reference_bit_for_bit() {
        const REFERENCE: u64 = 0x67e8_14aa_2127_b333;
        let mut checksum: u64 = 0xcbf2_9ce4_8422_2325;
        let mut add_to_checksum = |value: f32| {
            checksum = (checksum ^ u64::from(value.to_bits())).wrapping_mul(0x0100_0000_01b3);
        };
        let special_coordinates = [0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1e-7, -1e-7, 8_388_607.5, -8_388_607.5, 8_388_608.0, 3e9, -3e9];
        let mut points: Vec<[f32; 3]> =
            (0..4000).map(|i| [i as f32 * 0.37 - 700.0, ((i * 7919) % 8000) as f32 * 0.01 - 40.0, (i % 97) as f32 * -0.53]).collect();
        points.extend(special_coordinates.iter().flat_map(|&first| special_coordinates.iter().map(move |&second| [first, second, first - second])));

        for point in points {
            for seed in [0, 7] {
                add_to_checksum(value(point, seed));
                add_to_checksum(perlin(point, seed));
                let (nearest, second_nearest, cell_id) = cellular(point, seed);
                add_to_checksum(nearest);
                add_to_checksum(second_nearest);
                add_to_checksum(cell_id);
                add_to_checksum(fbm(point, 6, seed, perlin));
                add_to_checksum(fbm(point, 3, seed, value));
            }
        }

        assert_eq!(checksum, REFERENCE, "noise output changed: {checksum:#x}");
    }

    /// A coordinate past the i32 range puts its cell at i32::MAX or MIN; the neighbouring cells
    /// wrap around instead of overflowing (which panics in debug builds).
    #[test]
    fn huge_coordinates_do_not_overflow_the_lattice() {
        for coordinate in [3e9, -3e9, 2_147_483_648.0, -2_147_483_904.0, f32::MAX, f32::MIN, f32::INFINITY, f32::NEG_INFINITY] {
            for point in [[coordinate, 0.5, 0.5], [0.5, coordinate, 0.5], [0.5, 0.5, coordinate], [coordinate, coordinate, coordinate]] {
                let (value_noise, perlin_noise, (nearest, second_nearest, cell_id)) = (value(point, 1), perlin(point, 1), cellular(point, 1));

                // Infinities have no finite position inside a cell
                if coordinate.is_finite() {
                    assert!(
                        [value_noise, perlin_noise, nearest, second_nearest, cell_id].iter().all(|value| (0.0..=1.0).contains(value)),
                        "{point:?}: {value_noise} {perlin_noise} {nearest} {second_nearest} {cell_id}"
                    );
                }
            }
        }
    }

    #[test]
    fn cell_matches_floor_exactly() {
        let reference = |p: f32| {
            let f = p.floor();
            (f as i32, p - f)
        };
        let edge = [0.0, -0.0, 0.5, -0.5, 1.0, -1.0, 1.0 - f32::EPSILON, -1e-30, 8_388_607.5, -8_388_607.5, 8_388_608.0, -8_388_609.0, 3e9, -3e9];
        let all =
            edge.into_iter().chain([f32::INFINITY, f32::NEG_INFINITY, f32::NAN, f32::MAX, f32::MIN]).chain((0..20_000).map(|i| (i as f32 - 10_000.0) * 0.013));

        for coordinate in all {
            let (actual, expected) = (cell(coordinate), reference(coordinate));

            assert!(actual.0 == expected.0 && actual.1.to_bits() == expected.1.to_bits(), "{coordinate}: {actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn seeds_change_the_pattern() {
        let p = [1.3, 2.7, 0.4];
        assert_ne!(value(p, 1), value(p, 2));
    }

    #[test]
    fn value_noise_is_continuous() {
        let a = value([0.5, 0.5, 0.5], 0);
        let b = value([0.5001, 0.5, 0.5], 0);
        assert!((a - b).abs() < 1e-2);
    }
}
