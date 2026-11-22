use std::ops::RangeInclusive;

/// One captioned slider in the stack beside a picture
pub(crate) struct StackSlider<'a> {
    /// The dim caption above the track (Count, Angle span)
    pub(crate) label: &'a str,

    pub(crate) value: &'a mut f32,

    /// What the track spans
    pub(crate) range: RangeInclusive<f32>,

    /// What a typed value may reach, at least the track's span (a radial Count types past its track's most)
    pub(crate) typed: RangeInclusive<f32>,

    pub(crate) decimals: usize,

    /// A unit after the value ("°")
    pub(crate) suffix: &'a str,
}
