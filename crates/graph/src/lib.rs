// SPDX-License-Identifier: MIT OR Apache-2.0
//! The graph runtime: node graphs parsed from text and evaluated to whole float RGBA images on the CPU (f32, the
//! reference), with the node kinds, function graphs, FX-maps, pattern programs and the image codecs they read and
//! write. It depends on nothing outside the standard library, and on nothing of the paint stack that renders graphs into
//! layers: callers pass in their settings with each evaluation ([`context::EvalContext`]: cancel flag, logging,
//! profiler, memory counter) and to the node cache ([`graph::NodeCache::new`]). No evaluation state is process-wide.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::unimplemented, clippy::todo, clippy::unreachable)]
pub mod atomic;
pub mod checksum;
pub mod compiled_ops;
pub mod context;
pub mod curve;
pub mod deflate;
pub mod expr;
pub mod exr;
pub mod font;
pub mod func;
pub mod fxmap;
pub mod graph;
pub mod grunge;
pub mod half;
pub mod lcg_rand;
pub mod math;
pub mod measured;
pub mod names;
pub mod nodes2;
pub mod noise;
pub mod ops;
pub mod par;
pub mod params;
pub mod patterns;
pub mod png;
pub mod profile;
pub mod scatter;
pub mod tex;
pub mod vector;
pub mod versions;

/// The data-parallel library the engine runs on, for hosts that build an [`context::EvalContext::pool`].
pub use rayon;

#[cfg(test)]
mod rule_tables;
