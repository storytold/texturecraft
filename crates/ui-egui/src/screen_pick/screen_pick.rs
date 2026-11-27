use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use super::{PickOutcome, PickShared, ends_on_focus_loss, physical_rect, wait_for_click, window_dpi};

/// A running eyedropper pick's wait for a click outside the app, for the colour bar `bar`
pub struct ScreenPick {
    pub(super) bar: egui::Id,
    pub(super) outcomes: Receiver<PickOutcome>,
    shared: Arc<PickShared>,
    /// Whether the app losing the focus ends the pick (not while the desktop's own picker has it)
    pub(super) ends_on_focus_loss: bool,
}

impl ScreenPick {
    pub(super) fn start(bar: egui::Id, ctx: &egui::Context) -> Self {
        let (sender, outcomes) = mpsc::channel();
        let shared = Arc::new(PickShared::default());

        let waiting = Arc::clone(&shared);
        let dpi = window_dpi(ctx);
        let ends_on_focus_loss = ends_on_focus_loss(ctx);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("eyedropper".into()).spawn(move || wait_for_click(&waiting, sender, ctx, dpi));
        if let Err(error) = spawned {
            log::warn!("the eyedropper picks on the model only: {error}");
        }

        ScreenPick { bar, outcomes, shared, ends_on_focus_loss }
    }

    /// A pick for `bar` whose click outside the app already ended as `outcome` (tests and agents stand in for the system)
    pub fn ended(bar: egui::Id, outcome: PickOutcome) -> Self {
        let (sender, outcomes) = mpsc::channel();
        let _ = sender.send(outcome);
        ScreenPick { bar, outcomes, shared: Arc::default(), ends_on_focus_loss: true }
    }

    /// A pick for `bar` still waiting for its click outside the app (tests stand in for the system)
    pub fn waiting(bar: egui::Id) -> Self {
        let (_, outcomes) = mpsc::channel();
        ScreenPick { bar, outcomes, shared: Arc::default(), ends_on_focus_loss: true }
    }

    /// Tells the waiting thread where the app's windows are and whether the pointer is over one of them
    pub(super) fn share_windows(&self, ctx: &egui::Context) {
        let viewports = ctx.input(|input| input.raw.viewports.clone());
        let windows = viewports.values().filter_map(physical_rect).collect();
        self.shared.set_windows(windows);
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let pointer_inside = viewports.keys().any(|id| ctx.input_for(*id, |input| input.pointer.has_pointer()));
            self.shared.set_pointer_outside(!pointer_inside);
        }
    }
}

impl Drop for ScreenPick {
    fn drop(&mut self) {
        self.shared.stop();
    }
}
