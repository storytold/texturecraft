use egui::Pos2;

use super::Hold;

/// A drag holding the pointer, kept in egui's memory between frames
#[derive(Clone, Copy, Debug)]
pub(super) struct Lock {
    pub(super) hold: Hold,
    /// Where the press began: the pointer is kept there and comes back there
    pub(super) anchor: Pos2,
    /// Where the system's pointer is
    pub(super) seen: Pos2,
    /// Where egui sees the pointer: the anchor plus every move since, as far as it goes
    pub(super) at: Pos2,
    /// A put-back is on its way: the pointer move that brings it back is no motion
    pub(super) putting_back: bool,
    /// The drag let go: the next frame shows egui the pointer back at the anchor and forgets the lock
    pub(super) let_go: bool,
}
