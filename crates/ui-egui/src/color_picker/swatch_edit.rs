/// A change the picker's Swatches row asks of the project's swatches
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SwatchEdit {
    /// The + chip: keep the picker's colour (linear) as a swatch
    Add([f32; 3]),
    /// A swatch's Remove (its right-click menu), by index
    Remove(usize),
}
