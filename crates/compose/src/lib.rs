//! # texturecraft-compose
//!
//! The CPU layer-stack evaluator: [`eval::composite_tiles`] turns a texture set's layers, masks,
//! fills and painted pixels into one image per channel, tile by tile in parallel, including the
//! live preview of a stroke being drawn. [`display`] converts composited tiles into the texture
//! formats the viewport samples; [`pattern`] evaluates procedurals through projections, [`placed`] the placed
//! projections of imported fills.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod baked_maps;
pub mod below;
mod curvature_size_level;
pub mod curvature_sizes;
pub mod display;
pub mod eval;
pub mod field_cache;
pub mod graphs;
mod held_fields;
mod image_identity;
pub mod pattern;
pub mod placed;
pub mod store;
pub mod thumb;

pub use below::{BelowCache, StackKey};
pub use display::{DisplayKind, tile_bytes};
pub use eval::{
    BLOCK, Composite, Inputs, PaintTarget, Stencil, StrokePreview, all_tiles, composite_tiles, composite_tiles_cached, covered_tiles, effect_histogram,
    flatten_layer, mask_at, mask_image, pad_tiles,
};
pub use field_cache::{FIELD_BUDGET, FieldCache};
pub use graphs::{Cooked, GraphStore, GraphUse};
pub use store::{BakedMaps, ImageOrigin, ImageRes, ImageSource, ImageStore, PixelStore, Texels};
pub use thumb::{THUMB_TEXELS, ThumbImage, ThumbTarget};
