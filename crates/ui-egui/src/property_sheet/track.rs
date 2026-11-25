//! The line a slider's knob runs along

/// What a slider's knob runs along and shows
pub(super) struct Track {
    pub(super) start: f32,
    pub(super) end: f32,
    pub(super) center_y: f32,
    /// Where a two-sided range's fill starts and its tick sits (0 for signed ranges, 0.5 for a 0..1 range centred on "unchanged")
    pub(super) zero: Option<f32>,

    /// The track is a picture of its range (a gray ramp) drawn by the caller: no line and no fill
    pub(super) ramp: bool,
}
