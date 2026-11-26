//! Where a pattern panel's edits go

use serde_json::Value;

/// Where a pattern's edits go: `command` with `base` params naming its fill (and channel, with the colors a pattern maps to)
pub(crate) struct PatternTarget {
    pub command: &'static str,
    pub base: Value,
    /// Keeps the sections of two patterns shown at once apart
    pub salt: String,
}
