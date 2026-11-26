/// The icon of a menu button
#[derive(Clone, Copy)]
pub(super) enum MenuIcon {
    Glyph(&'static str),
    /// Add mask's square, half of it filled corner to corner
    Mask,
}
