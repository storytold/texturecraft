/// Which strip the sections belong to: the docked panel and the right-click popup can show the same sections at once
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum SectionScope {
    /// The docked Properties panel
    Docked,
    /// The right-click Properties popup over a view
    Popup,
}
