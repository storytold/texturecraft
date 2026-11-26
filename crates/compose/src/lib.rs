//! # texturecraft-compose
//!
//! The CPU layer-stack evaluator: [`eval::composite_tiles`] turns a texture set's layers, masks,
//! fills and painted pixels into one image per channel, tile by tile in parallel, including the
//! live preview of a stroke being drawn. [`display`] converts composited tiles into the texture
//! formats the viewport samples; [`pattern`] evaluates procedurals through projections.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod axis_frame;
mod baked_maps;
pub mod below;
mod curvature_size_level;
pub mod curvature_sizes;
pub mod display;
mod distance_envelope;
mod dripping_rust;
mod effect_histogram;
pub mod environment_sh;
pub mod environment_sky;
pub mod eval;
mod fabric;
mod field_cache;
mod field_cache_key;
pub mod fill_copies;
pub mod fill_copy;
mod fill_depth;
mod fill_place;
pub mod frame_placement;
mod fur_lock;
mod generator_maps;
mod grunge_pattern;
mod held_fields;
mod histogram_scan;
mod image_identity;
mod mask_gray;
mod micro_detail;
pub mod mip_level;
mod organic;
pub mod pattern;
mod pattern_context;
pub mod projector_cover;
pub mod projector_depth;
mod projector_depth_cache;
pub mod projector_view;
mod sampled_map;
mod sections;
mod shape_distance;
mod slope_texel;
mod slot_placement;
mod soft_box;
pub mod solo;
pub mod store;
mod texture_layer;
mod three_d_distance;
pub mod thumb;
pub mod tile_length;
mod tile_pattern;
mod tile_random_stream;
mod timber;
mod uv_border_distance;
mod uv_islands;
pub mod warp;
mod warp_source_sample;
mod warp_texel;
mod wear;
mod wear_inputs;
mod wear_scan;
mod working_field;

pub use below::{BelowCache, StackKey};
pub use display::{DisplayKind, kinds_for, tile_bytes};
pub use effect_histogram::EffectHistogram;
pub use eval::{
    BLOCK, Composite, Inputs, MASK_CHANNEL, PaintTarget, Stencil, StrokePreview, all_tiles, compare_histograms, composite_tiles, composite_tiles_cached,
    composite_tiles_with_projectors, covered_tiles, effect_histogram, flatten_layer, mask_at, mask_image, mask_texels, mask_values, pad_tiles,
    stack_reads_neighbours,
};
pub use mask_gray::MaskGray;
pub use pattern_context::PatternContext;
pub use projector_depth_cache::ProjectorDepthCache;
pub use solo::solo_project;
pub use store::{BakedMaps, ImageRes, ImageStore, PixelStore};
pub use thumb::{THUMB_TEXELS, ThumbImage, ThumbTarget};
