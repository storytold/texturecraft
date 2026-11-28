//! Which page under jump tabs the Properties panel shows

/// A fill's sections (`mask` for a mask fill effect) or the paint tool's
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TabbedPage {
    Fill { mask: bool },
    Tool,
}
