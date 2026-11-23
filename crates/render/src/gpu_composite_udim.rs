//! P8, UDIM texture sets and baked maps with different layouts (plan section 2.4, last two
//! rows). Owned by the P8 agent; no other package edits this file or
//! `shaders/composite_udim.wgsl`. P8 lands last: it changes coordinates everywhere, so it starts
//! after every other package has merged and then may touch their files.
//!
//! What P8 implements: the tile grid and per-tile block scales in the program, neighbourhood
//! stages once per distinct tile scale (as `eval.rs` does), per-tile `height_slope` and
//! procedural resolution per UV tile; one texture or array layer per baked map with its own
//! height (`BakedInputs::new` rejects mixed layouts today; that check is P6's code until P8).
//!
//! How: [`check_set`] is the only gate on the set today; the WGSL file and `ext[3]` of every
//! operation are free for P8's per-tile data, and `params.packages[7]` for band-wide values.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] (no library filter is P8's). Add a fallback test here for
//! UDIM sets before porting, then oracle tests on a two-tile set with different tile scales.

use texturecraft_core::TextureSet;

use super::CompositeProgram;

/// Library filter ids P8 still leaves on the CPU (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/composite_udim.wgsl");
/// P8's WGSL defines nothing other files call yet.
pub(super) const WGSL_STUB: &str = "";

/// Whether the GPU program evaluates texture set `set`; the error names the view it falls back for.
pub(super) fn check_set(set: &TextureSet, mask_preview: bool) -> Result<(), &'static str> {
    if set.uv_tiles.is_empty() {
        return Ok(());
    }
    Err(if mask_preview { "UDIM texture sets use CPU mask preview." } else { "UDIM texture sets use CPU compositing." })
}

/// Whether a program needs the full `composite_udim.wgsl`.
pub(super) fn uses_shader(_program: &CompositeProgram) -> bool {
    false
}

/// P8's vec4 of the band uniform (`params.packages[7]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}
