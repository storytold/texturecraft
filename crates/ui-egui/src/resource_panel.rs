//! What the generator and pattern panels share: the attribute rows' key width, note size, the resolution factor's label, the output size readout and a random seed

use texturecraft_engine::core::{OutputResolution, TextureSet};

use crate::i18n::tr;

/// Width of the names in the Attributes section, so their values line up
pub(crate) const ATTRIBUTE_KEY_WIDTH: f32 = 80.0;
/// Font size of the notes under a control
pub(crate) const NOTE_FONT_SIZE: f32 = 10.5;

/// Random seeds stay below this, short enough to read and type
const SEED_RANGE: u32 = 10_000;
/// Knuth's multiplicative hash constant: spreads the clock's nanoseconds over the seed range
const SEED_MIX: u32 = 2_654_435_761;
/// The most powers of two a resolution factor scales by, either way
const LARGEST_FACTOR_STEPS: u8 = OutputResolution::FACTORS.end().unsigned_abs();

/// "Same as context", or the fraction or multiple a resolution factor stands for
pub(crate) fn factor_label(factor: i8) -> String {
    match factor {
        0 => tr("Same as context").to_string(),
        negative if negative < 0 => format!("1/{}", 1u32 << negative.unsigned_abs().min(LARGEST_FACTOR_STEPS)),
        positive => format!("×{}", 1u32 << positive.unsigned_abs().min(LARGEST_FACTOR_STEPS)),
    }
}

/// The output size `resolution` gives in `texture_set`: one size, or the smallest and largest when its UV tiles differ
pub(crate) fn output_size_label(resolution: &OutputResolution, texture_set: &TextureSet) -> String {
    let sizes = || texture_set.uv_tiles.iter().map(|&udim| resolution.output_size(texture_set.tile_resolution(udim), texture_set.resolution));
    let whole_set = resolution.output_size(texture_set.resolution, texture_set.resolution);
    let (smallest, largest) = (sizes().min().unwrap_or(whole_set), sizes().max().unwrap_or(whole_set));
    if smallest == largest { format!("{smallest} × {smallest}") } else { format!("{smallest}–{largest} × {smallest}–{largest}") }
}

/// A seed different from `current`, from the clock
pub(crate) fn random_seed(current: u32) -> u32 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.subsec_nanos());
    let mixed = nanos.wrapping_mul(SEED_MIX) % SEED_RANGE;
    if mixed == current { (mixed + 1) % SEED_RANGE } else { mixed }
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::core::ResolutionMode;

    use super::*;

    #[test]
    fn the_output_size_follows_each_uv_tile() {
        let mut set = TextureSet::new("Tiles", 0, 1024);
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 256);
        let per_tile = OutputResolution::default();
        let whole_set = OutputResolution { mode: ResolutionMode::TextureSet, ..per_tile };

        assert_eq!(output_size_label(&per_tile, &set), "256–1024 × 256–1024", "tiles of different sizes show the smallest and the largest");
        assert_eq!(output_size_label(&whole_set, &set), "1024 × 1024", "the texture set's size is the same for every tile");
    }

    #[test]
    fn a_set_without_uv_tiles_shows_its_own_size() {
        let set = TextureSet::new("Plain", 0, 512);

        assert_eq!(output_size_label(&OutputResolution::default(), &set), "512 × 512");
    }
}
