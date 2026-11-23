use std::ops::RangeInclusive;

/// What sits under the size beside a preview square
pub(crate) enum PreviewLower<'a> {
    /// Offset across and down, signed
    Offset { values: &'a mut [f32; 2], range: RangeInclusive<f32>, typed: RangeInclusive<f32> },

    /// One other setting (tri-planar Hardness)
    Slider { label: &'a str, value: &'a mut f32, range: RangeInclusive<f32>, decimals: usize },
}
