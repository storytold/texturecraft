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

    /// An angle: a small dial before its value, as an angle row has
    pub(crate) dial: bool,
}

impl<'a> StackSlider<'a> {
    /// A 0..1 setting with two decimals
    pub(crate) fn unit(label: &'a str, value: &'a mut f32) -> StackSlider<'a> {
        StackSlider { label, value, range: 0.0..=1.0, typed: 0.0..=1.0, decimals: 2, suffix: "", dial: false }
    }

    /// A whole-number setting across `range`
    pub(crate) fn whole(label: &'a str, value: &'a mut f32, range: RangeInclusive<f32>) -> StackSlider<'a> {
        StackSlider { label, value, range: range.clone(), typed: range, decimals: 0, suffix: "", dial: false }
    }
}
