/// How a baker row's glyph button shows its state
#[derive(Clone, Copy, PartialEq)]
pub(super) enum GlyphLook {
    Plain,
    /// Bright: what it stands for is on (the set shares these settings)
    Lit,
    /// Held down in a box: the view it toggles is showing
    Pressed,
}
