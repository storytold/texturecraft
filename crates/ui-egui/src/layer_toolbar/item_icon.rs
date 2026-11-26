/// What an Add menu item shows at its left
#[derive(Clone, Copy)]
pub(super) enum ItemIcon {
    Glyph(&'static str),
    /// A gray swatch (Add mask's White and Black)
    Swatch(u8),
}
