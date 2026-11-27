use super::BakeChange;
use serde::{Deserialize, Serialize};

/// Fingerprints of what one mesh map was baked from: a later change to any of them shows the map out of date
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BakeSource {
    /// The low poly's geometry
    pub low_poly: u64,
    /// The embedded mesh file when this map reads its material colors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_material_file: Option<u64>,
    /// The high poly files, in list order (0: none, the set bakes from the low poly)
    pub high_poly: u64,
    /// The cage file (0: none)
    pub cage: u64,
    /// The settings every map of the set shares (size, high poly, cage, skew) and its UV tiles
    pub settings: u64,
    /// The map's own settings
    pub map_settings: u64,
}

impl BakeSource {
    /// What differs in `current` from this record, in [`BakeChange`] order (empty: up to date)
    pub fn changes(&self, current: &BakeSource) -> Vec<BakeChange> {
        // Switching color source is already reported under high poly or map settings
        let material_changed = match (self.low_material_file, current.low_material_file) {
            (Some(previous), Some(now)) => previous != now,
            (None, None) => false,
            _ => self.high_poly == current.high_poly && self.map_settings == current.map_settings,
        };

        [
            (self.low_poly != current.low_poly || material_changed, BakeChange::LowPoly),
            (self.high_poly != current.high_poly, BakeChange::HighPoly),
            (self.cage != current.cage, BakeChange::Cage),
            (self.settings != current.settings, BakeChange::Settings),
            (self.map_settings != current.map_settings, BakeChange::MapSettings),
        ]
        .into_iter()
        .filter_map(|(changed, change)| changed.then_some(change))
        .collect()
    }
}

/// A fingerprint of `bytes` that stays the same across runs, platforms and Rust versions (unlike `std::hash`)
pub fn fingerprint(bytes: &[u8]) -> u64 {
    let words = bytes.chunks(8).map(|chunk| {
        let mut word_bytes = [0; 8];
        word_bytes[..chunk.len()].copy_from_slice(chunk);
        u64::from_le_bytes(word_bytes)
    });
    fingerprint_words(words.chain([bytes.len() as u64]))
}

/// [`fingerprint`] of a sequence of words: FNV-1a taking a 64-bit word per step
pub fn fingerprint_words(words: impl IntoIterator<Item = u64>) -> u64 {
    // FNV-1a 64-bit offset basis and prime
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    words.into_iter().fold(OFFSET_BASIS, |hash, word| (hash ^ word).wrapping_mul(PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_changes_with_any_byte_and_with_the_length() {
        let bytes: Vec<u8> = (0..=20).collect();
        let mut changed = bytes.clone();
        changed[17] ^= 1;

        assert_ne!(fingerprint(&bytes), fingerprint(&changed));
        assert_ne!(fingerprint(&bytes), fingerprint(&bytes[..20]));
        assert_ne!(fingerprint(&[]), fingerprint(&[0]));
    }
}
