use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use texturecraft_engine::Result;
use texturecraft_engine::bake::{BakeOutput, BakeTask, Progress};
use texturecraft_engine::core::BakeResolution;

use super::live_preview_target::LivePreviewTarget;

/// A map preview bake on its worker: its size, when it started (app time) and whether a dragged slider asked for it
pub(super) struct RunningPreview {
    pub(super) target: LivePreviewTarget,
    pub(super) size: BakeResolution,
    pub(super) started: f64,
    pub(super) dragging: bool,
    pub(super) pending: Arc<Mutex<Option<BakeTask>>>,
    pub(super) progress: Arc<Progress>,
    pub(super) receiver: Receiver<Result<BakeOutput>>,
}

impl Drop for RunningPreview {
    fn drop(&mut self) {
        self.progress.cancel();
        self.pending.lock().unwrap_or_else(|error| error.into_inner()).take();
    }
}
