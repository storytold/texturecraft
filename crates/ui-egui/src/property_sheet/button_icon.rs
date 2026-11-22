//! What a full-width button shows before its words: an icon font glyph or an icon drawn in code

/// A full-width button's icon
#[derive(Clone, Copy, Debug)]
pub(crate) enum ButtonIcon<'a> {
    /// A glyph of the icon font
    Glyph(&'a str),
}

impl<'a> From<&'a str> for ButtonIcon<'a> {
    fn from(glyph: &'a str) -> Self {
        ButtonIcon::Glyph(glyph)
    }
}
