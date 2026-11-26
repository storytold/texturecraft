//! Imports projects of the reference app into TextureCraft projects: the translator on top of
//! `texturecraft-import-reader`, which reads the file. This crate maps what the reader gives onto TextureCraft's
//! [`Project`](texturecraft_core::Project), layers, effects and sources:
//!
//! - [`import`]: the entry points ([`import_bytes`], [`import_path`]) and the per-set build.
//! - [`sets`], [`mesh`], [`channels`], [`blend`], [`baked`], [`bake_settings`], [`cameras`]: the mapping tables.
//! - [`layers`]: the layer tree, content, masks, projections and geometry masks.
//! - [`graph_library`], [`graph_uses`]: the project's compiled graphs decompiled, and every use of one as a graph
//!   source or effect (a placeholder where it cannot run).
//! - [`replay`]: brush strokes handed to a [`replay::StrokeReplayer`] (the engine's brush) as replay jobs.
//! - [`safety_net`]: each stack composited by TextureCraft's compositor and compared with the original's stored result.
//! - [`stored`]: the original's stored results per set and channel, read for `import.verify`.
//! - [`uv_tiles`]: sets the original paints per UV tile (imported as one texture, with a note).
//!
//! The import reads nothing but the file and its options; nothing of the original file is stored in the project.
//! This crate rasterises no strokes: with a replayer ([`Hooks::replay`]) painted masks and brush work become pixels
//! (approximated) before the safety net measures; without one they stay placeholders.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bake_settings;
pub mod baked;
pub mod blend;
pub mod cameras;
pub mod channels;
pub mod correction;
pub mod error;
pub mod graph_library;
pub mod graph_uses;
pub mod image;
pub mod import;
pub mod layers;
pub mod library;
pub mod mesh;
mod native_graphs;
pub mod options;
pub mod pixel_spill;
pub mod polygon_fill;
pub mod progress;
pub mod replay;
pub mod resources;
#[cfg(test)]
mod rule_tables;
pub mod safety_net;
pub mod sets;
pub mod status;
pub mod stored;
pub mod uv_tiles;

pub use error::{ImportError, PartError};
pub use graph_uses::GraphMode;
#[cfg(not(target_arch = "wasm32"))]
pub use import::import_path;
pub use import::{DEFAULTS_LAYER_NAME, Hooks, IMPORTER, Imported, SetSummary, Step, import_bytes};
pub use options::ImportOptions;
pub use texturecraft_compose::graphs::Cancel;
pub use texturecraft_import_reader::format_names::PROJECT_EXT;
