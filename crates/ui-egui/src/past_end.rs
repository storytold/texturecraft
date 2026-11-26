//! How far a Properties list scrolls past its last row

/// When a Properties list leaves blank space under its rows so its last section's top can reach the top of the list
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PastEnd {
    /// Always, as the docked panel: even when every section fits
    Always,
    /// Only once the rows no longer fit, as a popup sized to its rows
    WhenScrolling,
}
