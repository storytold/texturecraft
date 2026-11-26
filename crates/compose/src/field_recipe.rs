//! What the whole-map generator fields (UV Border Distance, Dripping Rust, UV Random Color, UV Checker) are built from, for an evaluator outside this crate (the GPU compositor) to build them as these modules do

use texturecraft_core::{GeneratorKind, ProceduralKind, WearParameters};

use crate::grunge_pattern::grunge_pattern;

pub use crate::dripping_rust::{
    DRIP_LENGTH_2K, DRIPS_SIGMA_2K, GRUNGE_SPREAD, MIN_COVERAGE, RUST_FLOOR, RUST_GAIN, RUST_OCCLUSION, RUST_POWER, SPREADING_SIGMA_2K, drip_taps,
};
pub use crate::uv_border_distance::{REACH_PER_DISTANCE, SMOOTHNESS_SIGMA_2K};
pub use crate::uv_islands::{
    BORDER_TEXELS_PER_WIDTH_2K, DARK_SQUARE_SHADE, LINE_HALF_WIDTH_2K, MAJOR_LINE_SHADE, MINOR_LINE_SHADE, MINOR_LINES_PER_SQUARE, SQUARES_PER_GRID_TILE,
    island_color, random_gray,
};
pub use crate::working_field::{REFERENCE_TILE_TEXELS, box_radius};

/// The grunge Dripping Rust breaks its rust up with, when it is one plain pattern across the UV square: its kind, its repeats across the UV square, and its mean and spread
pub fn rust_grunge() -> Option<(ProceduralKind, f32, f32, f32)> {
    let wear = WearParameters::defaults(GeneratorKind::Dirt)?;
    let grunge = grunge_pattern(GeneratorKind::Dirt);
    let plain = !wear.triplanar && grunge.rotation == 0.0 && grunge.breakup.is_none();
    plain.then_some((grunge.kind, grunge.repeats_per_scale * wear.grunge_scale, grunge.mean, grunge.spread))
}
