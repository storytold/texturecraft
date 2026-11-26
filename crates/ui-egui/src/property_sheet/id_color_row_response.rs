/// What one ID color's row did this frame
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IdColorRowResponse {
    /// The swatch was clicked: pick this color again
    pub(crate) repick: bool,
    /// The × was clicked
    pub(crate) remove: bool,
}
