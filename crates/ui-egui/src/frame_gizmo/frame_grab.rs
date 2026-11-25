/// What a drag on the frame does
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FrameGrab {
    /// The centre square: slide the frame across its own plane
    Slide,
    /// Along the frame's across (0), up (1) or out (2) direction
    Axis(usize),
    /// A planar's corner: scale it about its centre
    Scale,
    /// A planar's edge dot (top, right, bottom, left): stretch that side
    Side(usize),
    /// A warp's corner (top left, top right, bottom right, bottom left): pull it
    Corner(usize),
    /// The ring: turn it round its normal
    Turn,
}

impl FrameGrab {
    pub(super) fn cursor(self, dragging: bool) -> egui::CursorIcon {
        match self {
            FrameGrab::Slide if dragging => egui::CursorIcon::Grabbing,
            FrameGrab::Slide => egui::CursorIcon::Grab,
            FrameGrab::Axis(_) | FrameGrab::Corner(_) | FrameGrab::Turn => egui::CursorIcon::Move,
            FrameGrab::Scale => egui::CursorIcon::ResizeNwSe,
            FrameGrab::Side(side) if side % 2 == 0 => egui::CursorIcon::ResizeVertical,
            FrameGrab::Side(_) => egui::CursorIcon::ResizeHorizontal,
        }
    }
}
