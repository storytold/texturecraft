use egui_phosphor::regular as icon;

use crate::i18n::n_;
use crate::nav_view::NavView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavButton {
    Orbit,
    Pan,
    Zoom,
}

impl NavButton {
    pub(crate) fn icon(self) -> &'static str {
        match self {
            NavButton::Orbit => icon::ARROWS_CLOCKWISE,
            NavButton::Pan => icon::HAND_GRABBING,
            NavButton::Zoom => icon::MAGNIFYING_GLASS,
        }
    }

    pub(crate) fn tooltip(self, view: NavView) -> &'static str {
        match (self, view) {
            (NavButton::Orbit, _) => n_("Orbit: press and drag to turn the view around the model"),
            (NavButton::Pan, _) => n_("Pan: press and drag to move the view"),
            (NavButton::Zoom, NavView::Scene) => n_("Zoom: press and drag up to move closer, down to move away"),
            (NavButton::Zoom, NavView::Uv) => n_("Zoom: press and drag up to zoom in, down to zoom out"),
        }
    }
}
