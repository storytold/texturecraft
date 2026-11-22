//! Neutral scoring for imported projects: how close TextureCraft's composite of an imported texture set comes to the
//! result the original file stored for it, per channel, measured on the mesh's texels only.
//!
//! - [`islands`]: which texels lie on the mesh ([`IslandMask`]); the rest of a stored result is padding.
//! - [`metrics`]: mean, max, share within 1 and 2 steps, PSNR and a gain/offset fit, in 8-bit steps of the stored
//!   encoding (1 step = 1/255), accumulated texel by texel ([`Tally`]) so no second copy of an image is held.
//! - [`grade`]: the grades (`exact`, `passes`, `near`, `off`) and the outlier flag.
//! - [`report`]: the per-channel, per-set and per-file report (JSON through serde, and a text table).
//!
//! The crate knows nothing about files or compositing: the engine's `import.verify` composites the session's sets and
//! reads the stored results through `texturecraft-import`, then feeds both here texel by texel.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod grade;
pub mod islands;
pub mod metrics;
pub mod report;

pub use grade::{Grade, OUTLIER_STEPS, safety_net_only};
pub use islands::IslandMask;
pub use metrics::{Fit, Metrics, Tally};
pub use report::{ChannelReport, GradeCounts, SetReport, VerifyReport};
