// SPDX-License-Identifier: MIT OR Apache-2.0
//! A folder of results ([`DiskStore`]) kept between runs and shared by any number of processes at once, for the
//! computations an import repeats (graph renders). Native builds only: the web build has no folder to keep them in.
//!
//! - An entry is written to a temporary file and renamed into place, so a reader sees all of it or none.
//! - An entry carries its key, its lengths and a checksum; anything that does not add up is a miss and is removed,
//!   never a result. The key is the caller's fingerprint of what went in, so an entry cannot be right for anything else.
//! - The folder is kept under a limit by removing what was used longest ago ([`eviction`]). Removing an entry another
//!   process wants only costs that process a computation.
//!
//! What the bytes mean and what goes into a key is the callers' business.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod config;
pub mod entry;
pub mod eviction;
pub mod store;

pub use config::Config;
pub use store::{DiskStore, Stats};
