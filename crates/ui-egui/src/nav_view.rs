use crate::nav_button::NavButton;

/// Which view a column of buttons belongs to
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavView {
    /// The 3D view: orbit, pan and zoom
    Scene,
    /// The 2D view: pan and zoom
    Uv,
}

impl NavView {
    /// The buttons of the view, top to bottom
    pub fn buttons(self) -> &'static [NavButton] {
        match self {
            NavView::Scene => &[NavButton::Orbit, NavButton::Pan, NavButton::Zoom],
            NavView::Uv => &[NavButton::Pan, NavButton::Zoom],
        }
    }
}
