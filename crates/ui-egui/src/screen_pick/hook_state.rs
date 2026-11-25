use std::sync::Arc;
use std::sync::mpsc::Sender;

use winsafe::POINT;

use super::release_wait::ReleaseWait;
use super::{PickOutcome, PickShared};

/// The running pick the hook procedure reports to (a hook procedure has no context of its own)
pub(super) struct HookState {
    /// Which pick this is: a pick that ends after the next one started leaves the next one's state alone
    pub(super) generation: u64,
    pub(super) shared: Arc<PickShared>,
    pub(super) outcomes: Sender<PickOutcome>,
    pub(super) ctx: egui::Context,
    /// Where the press it took was: the hook's thread reads that pixel once the hook procedure has answered
    pub(super) pending_pick: Option<POINT>,
    /// The wait for the release of the press it took, which is taken too
    pub(super) release_wait: Option<ReleaseWait>,
    pub(super) done: bool,
}

impl HookState {
    /// Pick `generation`'s state before any click
    pub(super) fn new(generation: u64, shared: Arc<PickShared>, outcomes: Sender<PickOutcome>, ctx: egui::Context) -> Self {
        HookState { generation, shared, outcomes, ctx, pending_pick: None, release_wait: None, done: false }
    }
}
