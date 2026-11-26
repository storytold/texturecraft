//! The size a cooked source (a MaterialCraft material's maps) is made at

use serde::{Deserialize, Serialize};

use crate::layer::OutputResolution;
use crate::{MAX_RESOLUTION, MIN_RESOLUTION};

/// How big a source is cooked: relative to its texture set (following it when it changes) or a fixed size
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CookResolution {
    /// The texture set's resolution times 2 to this power
    Relative(i8),
    /// Pixels across
    Fixed(u32),
}

impl CookResolution {
    /// The texture set's own resolution (Auto)
    pub const TEXTURE_SET: CookResolution = CookResolution::Relative(0);

    /// Powers of two a relative size may scale the texture set's by, as a generator's output resolution does
    pub const FACTORS: std::ops::RangeInclusive<i8> = OutputResolution::FACTORS;

    /// Pixels across for a texture set `set` pixels across: a power of two the engine can cook, else the texture set's size
    pub fn size(self, set: u32) -> u32 {
        let size = match self {
            CookResolution::Relative(factor) => {
                let factor = factor.clamp(*Self::FACTORS.start(), *Self::FACTORS.end());
                if factor >= 0 { set.saturating_mul(1 << factor) } else { set >> factor.unsigned_abs() }
            }
            CookResolution::Fixed(size) => size,
        };
        if Self::cookable(size) { size } else { set }
    }

    /// Whether maps may be cooked `size` pixels across
    pub fn cookable(size: u32) -> bool {
        size.is_power_of_two() && (MIN_RESOLUTION..=MAX_RESOLUTION).contains(&size)
    }
}

#[cfg(test)]
mod tests {
    use super::CookResolution;

    #[test]
    fn sizes_follow_the_texture_set_and_stay_cookable() {
        assert_eq!(CookResolution::TEXTURE_SET.size(2048), 2048);
        assert_eq!(CookResolution::Relative(-1).size(2048), 1024);
        assert_eq!(CookResolution::Relative(2).size(1024), 4096);
        assert_eq!(CookResolution::Relative(3).size(2048), 2048, "16384 is past the largest cook");
        assert_eq!(CookResolution::Relative(i8::MIN).size(1024), 128);
        assert_eq!(CookResolution::Fixed(512).size(2048), 512);
        assert_eq!(CookResolution::Fixed(300).size(2048), 2048);
        assert_eq!(CookResolution::Fixed(u32::MAX).size(2048), 2048);
    }
}
