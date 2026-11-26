use egui::Pos2;

use super::SIZE_ARROW;
use crate::frame_gizmo::distance_to_segment;

/// A handle that resizes the projector
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SizeHandle {
    pub at: Pos2,
    /// The projector directions it scales
    pub axes: [bool; 3],
    /// A double arrow's direction on screen; None for a dot
    pub along: Option<egui::Vec2>,
}

impl SizeHandle {
    /// How far screen point `p` is from the handle (a double arrow: from anywhere along it)
    pub(super) fn distance(&self, p: Pos2) -> f32 {
        match self.along {
            Some(along) => distance_to_segment(p, self.at - along * SIZE_ARROW, self.at + along * SIZE_ARROW),
            None => self.at.distance(p),
        }
    }

    /// The resize cursor along its arrow, or a diagonal one for a dot
    pub(super) fn cursor(&self) -> egui::CursorIcon {
        match self.along {
            Some(along) if along.y.abs() >= along.x.abs() => egui::CursorIcon::ResizeVertical,
            Some(_) => egui::CursorIcon::ResizeHorizontal,
            None => egui::CursorIcon::ResizeNeSw,
        }
    }
}
