//! Jitter drawn from a stroke's seed: one generator per jittered quantity, so the same seed lays the same jitter dab for
//! dab. A [`crate::Stroke`] draws from it after [`crate::Stroke::seeded`]; without it each stamp hashes its own index
//! (TextureCraft's default, the same sequence in every stroke).
//!
//! The generators are L'Ecuyer's taus88 (1996), all three words of each set to one output of the POSIX `lrand48`
//! generator seeded with the stroke's seed, in the order flow, size, rotation, direction, offset. That is how the
//! reference app draws a stroke's jitter (measured on its own strokes with chosen seeds), so imported strokes replay
//! with their original jitter. A generator advances only when its quantity is jittered.

/// `lrand48` (POSIX): the state starts as `(seed << 16) | 0x330E` and steps as `X = (X·0x5DEECE66D + 0xB) mod 2^48`;
/// each output is `X >> 17`.
const LRAND48_SEED_LOW: u64 = 0x330E;
const LRAND48_A: u64 = 0x5_DEEC_E66D;
const LRAND48_C: u64 = 0xB;
const LRAND48_MASK: u64 = (1 << 48) - 1;
const LRAND48_OUTPUT_SHIFT: u32 = 17;
/// A taus88 output as a fraction: output / 2^32.
const TWO_POW_32: f64 = 4_294_967_296.0;
/// The `lrand48` outputs that seed the five generators come first; the one after them picks where a stamp cycle starts.
const CYCLE_START_OUTPUT: usize = 6;
/// Smallest `1 − u` the normal draw takes the logarithm of.
const MIN_LOG_ARGUMENT: f32 = 1e-7;

/// `lrand48`'s outputs for `seed`, in order.
fn lrand48(seed: u32) -> impl Iterator<Item = u32> {
    let mut x = (u64::from(seed) << 16) | LRAND48_SEED_LOW;
    std::iter::from_fn(move || {
        x = x.wrapping_mul(LRAND48_A).wrapping_add(LRAND48_C) & LRAND48_MASK;
        Some((x >> LRAND48_OUTPUT_SHIFT) as u32)
    })
}

/// L'Ecuyer's taus88 with all three words set to one value; masks and shifts are taus88's own.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Taus88 {
    s1: u32,
    s2: u32,
    s3: u32,
}

impl Taus88 {
    fn new(word: u32) -> Taus88 {
        Taus88 { s1: word, s2: word, s3: word }
    }

    /// Even in 0..1.
    fn unit(&mut self) -> f32 {
        self.s1 = ((self.s1 & 0xFFFF_FFFE) << 12) ^ (((self.s1 << 13) ^ self.s1) >> 19);
        self.s2 = ((self.s2 & 0xFFFF_FFF8) << 4) ^ (((self.s2 << 2) ^ self.s2) >> 25);
        self.s3 = ((self.s3 & 0xFFFF_FFF0) << 17) ^ (((self.s3 << 3) ^ self.s3) >> 11);
        (f64::from(self.s1 ^ self.s2 ^ self.s3) / TWO_POW_32) as f32
    }
}

/// How seeded position jitter moves a stamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SeededJitter {
    /// The stroke's seed.
    pub seed: u32,
    /// Along both of the view's axes on the surface, in a direction drawn per stamp; otherwise across the stroke.
    pub both_axes: bool,
    /// The distance follows a normal distribution (deviation 1) instead of an even one in −1..1.
    pub normal: bool,
}

/// One stamp's draw: each value in 0..1, 0 for a quantity that is not jittered.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Draw {
    pub size: f32,
    pub flow: f32,
    pub rotation: f32,
    /// The direction of a both-axes move: a share of a half turn.
    pub direction: f32,
    /// The distance of the move: −1..1 (even) or a normal draw.
    pub distance: f32,
}

/// Which quantities a stamp jitters (only those generators advance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Jittered {
    pub size: bool,
    pub flow: bool,
    pub rotation: bool,
    pub position: bool,
}

/// A stroke's generators.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeRandom {
    jitter: SeededJitter,
    flow: Taus88,
    size: Taus88,
    rotation: Taus88,
    direction: Taus88,
    offset: Taus88,
}

impl StrokeRandom {
    pub fn new(jitter: SeededJitter) -> StrokeRandom {
        let mut words = lrand48(jitter.seed).map(Taus88::new);
        let mut next = || words.next().unwrap_or_else(|| Taus88::new(0));
        let (flow, size, rotation, direction, offset) = (next(), next(), next(), next(), next());
        StrokeRandom { jitter, flow, size, rotation, direction, offset }
    }

    pub fn jitter(&self) -> SeededJitter {
        self.jitter
    }

    /// The next stamp's draw.
    pub fn draw(&mut self, jittered: Jittered) -> Draw {
        let size = if jittered.size { self.size.unit() } else { 0.0 };
        let flow = if jittered.flow { self.flow.unit() } else { 0.0 };
        let (mut direction, mut distance) = (0.0, 0.0);
        if jittered.position {
            distance = if self.jitter.normal { self.offset_normal() } else { 2.0 * self.offset.unit() - 1.0 };
            if self.jitter.both_axes {
                direction = self.direction.unit();
            }
        }
        let rotation = if jittered.rotation { self.rotation.unit() } else { 0.0 };
        Draw { size, flow, rotation, direction, distance }
    }

    /// Normal, mean 0, deviation 1: Box-Muller on two draws of the offset generator.
    fn offset_normal(&mut self) -> f32 {
        let (a, b) = (self.offset.unit(), self.offset.unit());
        (-2.0 * (1.0 - b).max(MIN_LOG_ARGUMENT).ln()).sqrt() * (std::f32::consts::TAU * a).cos()
    }
}

/// Where a stroke with `seed` starts a cycle of `n` stamps when the cycle starts at a drawn stamp: the `lrand48` output
/// after the five that seed the generators, modulo `n` (0 for an empty cycle).
pub fn cycle_start(seed: u32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    lrand48(seed).nth(CYCLE_START_OUTPUT - 1).map_or(0, |x| x as usize % n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrand48_follows_its_definition() {
        // lrand48 after srand48(0): the first outputs of the POSIX generator
        let first: Vec<u32> = lrand48(0).take(3).collect();
        assert_eq!(first, [366_850_414, 1_610_402_240, 206_956_554]);
    }

    #[test]
    fn the_same_seed_draws_the_same_jitter_and_another_seed_another() {
        let all = Jittered { size: true, flow: true, rotation: true, position: true };
        let draws = |seed: u32| {
            let mut r = StrokeRandom::new(SeededJitter { seed, both_axes: true, normal: true });
            (0..32).map(|_| r.draw(all)).collect::<Vec<_>>()
        };
        assert_eq!(draws(7), draws(7));
        assert_ne!(draws(7), draws(8));
        for d in draws(123) {
            for v in [d.size, d.flow, d.rotation, d.direction] {
                assert!((0.0..1.0).contains(&v), "{v}");
            }
            assert!(d.distance.is_finite());
        }
    }

    #[test]
    fn each_quantity_draws_from_its_own_generator() {
        let mut a = StrokeRandom::new(SeededJitter { seed: 42, ..SeededJitter::default() });
        let mut b = a.clone();
        // size alone, or size with flow: the size draws do not change
        let only = Jittered { size: true, ..Jittered::default() };
        let both = Jittered { size: true, flow: true, ..Jittered::default() };
        for _ in 0..16 {
            let (x, y) = (a.draw(only), b.draw(both));
            assert_eq!(x.size, y.size);
            assert_eq!(x.flow, 0.0);
        }
    }

    #[test]
    fn the_cycle_start_is_the_sixth_output() {
        // measured: a 64-stamp cycle of a stroke with this seed starts 18 stamps in
        assert_eq!(cycle_start(1_540_787_264, 64), 18);
        assert_eq!(cycle_start(1_540_787_264, 0), 0);
        assert!(cycle_start(7, 5) < 5);
    }
}
