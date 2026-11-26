//! Tells the user when an autosave fails: the worker reports back, and one notice stays until a later
//! autosave works, so a missing backup is not mistaken for a saved one.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::TextureCraftApp;
use crate::i18n::trf;

/// How one autosave ended.
struct Outcome {
    path: PathBuf,
    result: Result<(), String>,
}

pub(crate) struct AutosaveNotice {
    sender: Sender<Outcome>,
    receiver: Receiver<Outcome>,
    /// The failure the user was last told about; cleared by an autosave that works.
    shown: Option<String>,
}

impl Default for AutosaveNotice {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { sender, receiver, shown: None }
    }
}

/// Sends one autosave's result to the app, from any thread.
pub(crate) struct Reporter {
    sender: Sender<Outcome>,
    context: egui::Context,
}

impl Reporter {
    pub(crate) fn report(self, path: PathBuf, result: Result<(), String>) {
        match &result {
            Ok(()) => log::info!("autosaved {}", path.display()),
            Err(error) => log::warn!("autosave to {} failed: {error}", path.display()),
        }

        // A closed channel means the app is gone, so there is nobody left to tell
        if self.sender.send(Outcome { path, result }).is_ok() {
            self.context.request_repaint();
        }
    }
}

impl AutosaveNotice {
    pub(crate) fn reporter(&self, context: &egui::Context) -> Reporter {
        Reporter { sender: self.sender.clone(), context: context.clone() }
    }
}

/// Take in finished autosaves. A failure is announced once, however often it repeats.
pub(crate) fn poll(app: &mut TextureCraftApp) {
    while let Ok(Outcome { path, result }) = app.autosave_notice.receiver.try_recv() {
        let message = result.err().map(|error| trf("Autosave to {path} failed: {error}", &[("path", &path.display()), ("error", &error)]));
        if message == app.autosave_notice.shown {
            continue;
        }

        if let Some(old) = app.autosave_notice.shown.take() {
            app.toasts.retain(|toast| toast.text != old);
        }

        if let Some(message) = &message {
            app.session.log.error("Autosave", message.clone());
            app.toast_sticky(message.clone());
        }

        app.autosave_notice.shown = message;
    }
}
