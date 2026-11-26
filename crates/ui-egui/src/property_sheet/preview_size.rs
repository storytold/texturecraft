use std::ops::RangeInclusive;

/// The size beside a preview square: one slider while linked, one per direction otherwise
pub(crate) struct PreviewSize<'a> {
    /// The caption above it (Tiling, Size (cm), Scaling)
    pub(crate) label: &'a str,

    /// Across and down; while linked, down follows across
    pub(crate) values: &'a mut [f32; 2],

    /// What the track spans
    pub(crate) range: RangeInclusive<f32>,

    /// What a typed value may be: wider than the track
    pub(crate) typed: RangeInclusive<f32>,

    pub(crate) decimals: usize,

    pub(crate) linked: &'a mut bool,
}
