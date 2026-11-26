//! A Move on model hold under way, kept in egui's memory between frames

use egui::Pos2;

use crate::move_on_model::MoveOnModel;

/// The Move on model button held down: the fill it moves, where the pointer was on the last frame and the session revision its last edit left (its undo step, while the history is still there)
#[derive(Clone, Copy, Debug)]
pub(crate) struct MoveHold {
    pub target: MoveOnModel,
    pub last: Pos2,
    pub edited_revision: Option<u64>,
    /// The pass that last applied it: a sheet drawn twice in one frame moves the projection once
    pub pass: u64,
}
