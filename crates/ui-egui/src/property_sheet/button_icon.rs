//! What a full-width button shows before its words: an icon font glyph or an icon drawn in code

use crate::drawn_icon::DrawnIcon;

/// A full-width button's icon
#[derive(Clone, Copy, Debug)]
pub(crate) enum ButtonIcon<'a> {
    /// A glyph of the icon font
    Glyph(&'a str),
    /// An icon the font has no fitting glyph for
    Drawn(DrawnIcon),
}

impl<'a> From<&'a str> for ButtonIcon<'a> {
    fn from(glyph: &'a str) -> Self {
        ButtonIcon::Glyph(glyph)
    }
}

impl From<DrawnIcon> for ButtonIcon<'_> {
    fn from(icon: DrawnIcon) -> Self {
        ButtonIcon::Drawn(icon)
    }
}
