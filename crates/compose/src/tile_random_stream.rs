//! The random streams of a Tile Generator

/// The random streams of a Tile Generator, so the randoms of one tile are independent of each other
#[derive(Clone, Copy)]
pub(crate) enum TileRandomStream {
    Luminance = 0,
    Scale,
    Offset,
    PositionX,
    PositionY,
    Rotation,
    QuarterTurn,
    QuarterTurnApplies,
    MirrorX,
    MirrorY,
    Mask,
    SizeX,
    SizeY,
    IntersticeX,
    IntersticeY,
    Input,
}

/// Odd multipliers spreading the seed, the stream and the index over the hash, and its start (the golden ratio's and xxHash32's primes)
const SEED_MIX: u32 = 0x9E37_79B1;
const STREAM_MIX: u32 = 0x85EB_CA77;
const INDEX_MIX: u32 = 0xC2B2_AE3D;
const HASH_START: u32 = 0x27D4_EB2F;
/// The finalizer's multipliers ("lowbias32", an integer hash whose output bits all depend on every input bit) and its rounds
const FINALIZER_MIX_A: u32 = 0x7FEB_352D;
const FINALIZER_MIX_B: u32 = 0x846C_A68B;
const FINALIZER_ROUNDS: usize = 3;
/// The bits a draw keeps: an f32's precision, so every value is exact and below 1
const DRAW_BITS: u32 = 24;

impl TileRandomStream {
    /// A uniform value in 0..1 per (seed, stream, index): an integer hash, so a tile's randoms do not depend on evaluation order
    pub(crate) fn draw(self, seed: u32, index: u32) -> f32 {
        let mut hash = seed
            .wrapping_mul(SEED_MIX)
            .wrapping_add((self as u32).wrapping_mul(STREAM_MIX))
            .wrapping_add(index.wrapping_mul(INDEX_MIX))
            .wrapping_add(HASH_START);
        for _ in 0..FINALIZER_ROUNDS {
            hash ^= hash >> 16;
            hash = hash.wrapping_mul(FINALIZER_MIX_A);
            hash ^= hash >> 15;
            hash = hash.wrapping_mul(FINALIZER_MIX_B);
            hash ^= hash >> 16;
        }
        (hash >> (u32::BITS - DRAW_BITS)) as f32 / (1u32 << DRAW_BITS) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A draw keeps 24 bits, so `bits / 2^24` is the value exactly
    fn exact(bits: u32) -> f32 {
        bits as f32 / (1u32 << DRAW_BITS) as f32
    }

    /// Saved projects keep only the seed and the settings, so a changed draw would move the tiles of existing textures
    #[test]
    fn draws_stay_the_values_saved_projects_were_made_with() {
        assert_eq!(TileRandomStream::Luminance.draw(0, 0), exact(6_969_297));
        assert_eq!(TileRandomStream::Rotation.draw(1, 7), exact(11_123_133));
        assert_eq!(TileRandomStream::Input.draw(u32::MAX, u32::MAX), exact(6_943_220));
    }

    #[test]
    fn draws_are_below_one_and_differ_by_seed_stream_and_index() {
        let first = TileRandomStream::Scale.draw(3, 5);

        assert_eq!(first, TileRandomStream::Scale.draw(3, 5), "the same seed, stream and index draw the same value");
        assert_ne!(first, TileRandomStream::Scale.draw(4, 5));
        assert_ne!(first, TileRandomStream::Offset.draw(3, 5));
        assert_ne!(first, TileRandomStream::Scale.draw(3, 6));
        for index in 0..1000 {
            let value = TileRandomStream::Mask.draw(index, index);
            assert!((0.0..1.0).contains(&value), "draw {index} is {value}");
        }
    }
}
