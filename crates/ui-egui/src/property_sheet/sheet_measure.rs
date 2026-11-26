/// What one pass of a Properties panel measured, used to place the next pass's columns
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct SheetMeasure {
    /// The widest label that sits beside its control
    pub(super) label_width: f32,

    /// Where the widest toggle label ends, from the panel's left padding
    pub(super) toggle_label_end: f32,

    /// Where the widest picture row's name ends, from the panel's left padding
    pub(super) picture_label_end: f32,

    /// The widest value at a slider's right end
    pub(super) value_width: f32,
}
