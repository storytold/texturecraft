use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use texturecraft_engine::{EngineError, HighPolyLoad, Result};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};

/// How often the app looks for the worker's answer while a file loads
const POLL_INTERVAL: Duration = Duration::from_millis(30);

/// High poly files loading on a worker thread, one after the other, so a large mesh does not freeze the app
pub struct HighPolyJob {
    /// The file being loaded
    pub current: String,
    /// The worker's answer for `current`
    answer: Receiver<Result<HighPolyLoad>>,
    /// The files still to load, in order
    pub queue: VecDeque<String>,
    /// The project the files are for: they are dropped once another project is open
    project: u64,
}

impl HighPolyJob {
    /// The names of the files being loaded and waiting, without their folders
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(&self.current).chain(&self.queue).map(|path| std::path::Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path))
    }
}

impl TextureCraftApp {
    /// Add the mesh files at `paths` as high polys, read and parsed on a worker thread
    pub fn load_high_polys(&mut self, paths: impl IntoIterator<Item = String>) {
        let mut paths: VecDeque<String> = paths.into_iter().collect();
        match &mut self.high_poly_job {
            Some(job) if job.project == self.session.project_id => job.queue.append(&mut paths),
            _ => self.load_next_high_poly(paths),
        }
    }

    /// Start loading the first of `queue`, on top of the high polys the session has now
    fn load_next_high_poly(&mut self, mut queue: VecDeque<String>) {
        self.high_poly_job = queue.pop_front().map(|current| {
            let mut meshes = texturecraft_engine::BakeMeshes::default();
            meshes.high.clone_from(&self.session.doc.bake_meshes.high);
            let path = current.clone();
            let answer = crate::spawn_worker("texturecraft-high-poly", move || HighPolyLoad::read(&meshes, &path));
            HighPolyJob { current, answer, queue, project: self.session.project_id }
        });
    }

    /// Add a loaded high poly to the session (once no paint stroke is running) and start on the next file; drop them all once another project is open
    pub(crate) fn poll_high_poly_job(&mut self, context: &egui::Context) {
        let Some(job) = &self.high_poly_job else { return };

        if job.project != self.session.project_id {
            self.high_poly_job = None;
            self.toast(tr("The project changed while the high polys loaded, so they were not added"), true);
            return;
        }

        context.request_repaint_after(POLL_INTERVAL);

        // An unfinished stroke can restore its starting document
        if self.session.stroke_active() {
            return;
        }

        let loaded = match job.answer.try_recv() {
            Ok(loaded) => loaded,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(EngineError::Other(tr("The mesh loading worker stopped").into())),
        };
        let Some(job) = self.high_poly_job.take() else { return };

        match loaded {
            Ok(load) => {
                self.session.apply_high_poly_load(load);
            }
            Err(error) => self.toast(trf("Couldn't import {name}: {error}", &[("name", &job.current), ("error", &error.to_string())]), true),
        }
        self.load_next_high_poly(job.queue);
    }
}
