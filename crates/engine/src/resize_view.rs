//! Preparing a texture set's replacement view on the resize worker.

use std::sync::{Mutex, PoisonError};

use texturecraft_bake::Progress;
use texturecraft_compose::graphs::Cancel;
use texturecraft_compose::{GraphStore, Inputs, composite_tiles_cached};

use crate::session::SetCache;
use crate::{EngineError, Result};

const DISPLAY_CHUNK: usize = 64;

#[derive(Default)]
pub struct ResizeProgress {
    pub(crate) paint: Progress,
    pub(crate) graphs_cancel: Cancel,
    view: Mutex<ViewProgress>,
}

#[derive(Default)]
enum ViewProgress {
    #[default]
    Paint,
    Graphs {
        store: GraphStore,
        count: usize,
    },
    Display {
        store: GraphStore,
        done: usize,
        total: usize,
    },
    Ready,
}

/// A momentary view of the resize worker for the status bar.
pub enum ResizeStage {
    Paint,
    Graphs(Option<(String, usize, usize)>),
    Display,
    Ready,
}

impl ResizeProgress {
    pub fn cancel(&self) {
        self.paint.cancel();
        self.graphs_cancel.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.graphs_cancel.is_cancelled()
    }

    pub fn status(&self) -> (f32, ResizeStage) {
        let view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        match &*view {
            ViewProgress::Paint => (0.25 * self.paint.fraction(), ResizeStage::Paint),
            ViewProgress::Graphs { store, count } => {
                let progress = (store.cook_progress() / (*count).max(1) as f32).min(1.0);
                (0.25 + 0.50 * progress, ResizeStage::Graphs(store.active_cook()))
            }
            ViewProgress::Display { store, done, total } => {
                let stage = store.active_cook().map_or(ResizeStage::Display, |cook| ResizeStage::Graphs(Some(cook)));
                (0.75 + 0.24 * (*done as f32 / (*total).max(1) as f32), stage)
            }
            ViewProgress::Ready => (1.0, ResizeStage::Ready),
        }
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(EngineError::Other("Texture set resize was cancelled".into()));
        }
        Ok(())
    }

    pub(crate) fn graphs(&self, store: &GraphStore, count: usize) {
        *self.view.lock().unwrap_or_else(PoisonError::into_inner) = ViewProgress::Graphs { store: store.clone(), count };
    }

    pub(crate) fn ready(&self) {
        *self.view.lock().unwrap_or_else(PoisonError::into_inner) = ViewProgress::Ready;
    }

    fn start_display(&self, store: &GraphStore, total: usize) {
        *self.view.lock().unwrap_or_else(PoisonError::into_inner) = ViewProgress::Display { store: store.clone(), done: 0, total };
    }

    fn display_done(&self, tiles: usize) {
        if let ViewProgress::Display { done, .. } = &mut *self.view.lock().unwrap_or_else(PoisonError::into_inner) {
            *done = tiles;
        }
    }
}

/// Uses the same compositor and padding as the viewport, in bounded chunks with cancellation between them.
pub(crate) fn prepare_cpu_view(progress: &ResizeProgress, inputs: &Inputs, cache: &mut SetCache) -> Result<()> {
    let total = cache.dirty.len();
    progress.start_display(inputs.graphs, total);
    cache.below.pass = total;
    while !cache.dirty.is_empty() {
        progress.check()?;
        let tiles: Vec<_> = (0..DISPLAY_CHUNK).filter_map(|_| cache.dirty.pop_first()).collect();
        composite_tiles_cached(inputs, &mut cache.composite, &tiles, Some(&mut cache.below));
        progress.check()?;
        let padded = cache.dilation.apply_to_tiles(&mut cache.composite, &tiles);
        cache.upload.extend(tiles.into_iter().chain(padded));
        progress.display_done(total - cache.dirty.len());
    }
    Ok(())
}
