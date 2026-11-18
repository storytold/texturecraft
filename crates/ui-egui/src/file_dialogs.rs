//! Native file dialogs that never block the frame (craft-dialogs).
//!
//! An action that needs a path calls [`TextureCraftApp::ask_file`] and returns. The dialog shows at
//! the end of the frame, and the continuation runs with the answer on a later frame. The browser
//! build has no native dialogs (`web_files.rs`).

use std::panic::Location;
use std::path::PathBuf;
use std::sync::Arc;

pub use craft_dialogs::{Answer, FileDialog, Override, Request};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};

/// Whether an answer still needs the project that was open when the dialog was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bound {
    /// The continuation reads or changes the open project, and does not run on another one.
    Project,
    /// The continuation does not depend on the open project.
    App,
}

/// What runs with the answer. The list is empty when nothing was chosen.
type Then = Box<dyn FnOnce(&mut TextureCraftApp, Vec<PathBuf>)>;

/// Which dialog was asked for, for which project and by which code. Asking again with all three
/// the same brings the open panel to the front instead of queuing a second one.
#[derive(Clone, Debug, PartialEq)]
struct AskKey {
    request: Request,
    project: Option<u64>,
    origin: &'static Location<'static>,
}

/// A dialog's continuation.
pub struct Asked {
    key: AskKey,
    then: Then,
}

/// The app's dialogs and the requests not yet answered.
pub struct FileDialogs {
    dialogs: craft_dialogs::Dialogs<Asked>,
    asked: Vec<AskKey>,
    /// Requests are answered by an override instead of a native dialog.
    overridden: bool,
    /// The app has no native window (the test harness): a native dialog must never open.
    headless: bool,
}

impl FileDialogs {
    /// The dialogs of the app whose context is `ctx`: an answer wakes its event loop.
    pub fn new(ctx: egui::Context) -> FileDialogs {
        FileDialogs { dialogs: craft_dialogs::Dialogs::new(Arc::new(move || ctx.request_repaint())), asked: Vec::new(), overridden: false, headless: false }
    }

    /// Answers every request with `answer_with` instead of a native dialog (tests).
    pub fn set_override(&mut self, answer_with: Option<Override>) {
        self.overridden = answer_with.is_some();
        self.dialogs.set_override(answer_with);
    }

    /// For an app without a native window (the test harness): never open a native dialog. Every
    /// request is answered Cancelled until a test sets its own override.
    pub(crate) fn without_window(&mut self) {
        self.headless = true;
        self.set_override(Some(Box::new(|_| Answer::Cancelled)));
    }

    /// Requests that have not been answered yet.
    pub fn pending(&self) -> usize {
        self.asked.len()
    }

    /// Shows the next dialog asked for. Call at the end of every frame with the window.
    #[expect(clippy::panic, reason = "only an app without a window whose test removed the override gets here")]
    pub fn show_pending(&mut self, frame: &eframe::Frame) {
        // Guards the user's desktop: whatever was asked (open, save, folder), a window-less app never shows a system dialog
        if self.headless && !self.overridden && !self.asked.is_empty() {
            panic!("a file dialog would open in an app without a window and no file dialog override: set one with `file_dialogs.set_override`");
        }
        self.dialogs.show_pending(Some(frame));
    }
}

impl TextureCraftApp {
    /// Queues `dialog`. `then` runs on a later frame with the chosen paths, or with none after
    /// Cancel, after a dialog that could not be shown, and when `bound` is [`Bound::Project`] and
    /// another project has been opened since. Returns false, and drops `then`, when the same dialog
    /// is already showing or waiting.
    #[track_caller]
    #[expect(clippy::panic, reason = "only an app without a window whose test removed the override gets here")]
    pub(crate) fn ask_file(&mut self, dialog: FileDialog, bound: Bound, then: impl FnOnce(&mut TextureCraftApp, Vec<PathBuf>) + 'static) -> bool {
        // Guards the user's desktop: a test without an override would open a real system dialog
        if self.file_dialogs.headless && !self.file_dialogs.overridden {
            panic!("a file dialog was asked for in an app without a window and no file dialog override: set one with `file_dialogs.set_override`");
        }
        let project = (bound == Bound::Project).then_some(self.session.project_id);
        let key = AskKey { request: Request::File(dialog.clone()), project, origin: Location::caller() };
        if self.file_dialogs.asked.contains(&key) {
            self.file_dialogs.dialogs.raise();
            return false;
        }
        self.file_dialogs.asked.push(key.clone());
        self.file_dialogs.dialogs.ask(dialog, Asked { key, then: Box::new(then) });
        true
    }

    /// Runs the continuations of the dialogs that have answered.
    pub(crate) fn run_file_dialog_answers(&mut self) {
        for (asked, answer) in self.file_dialogs.dialogs.take_answers() {
            let Asked { key, then } = asked;
            if let Some(i) = self.file_dialogs.asked.iter().position(|k| *k == key) {
                self.file_dialogs.asked.remove(i);
            }
            let paths = match answer {
                Answer::Paths(paths) => paths,
                Answer::Unavailable(why) => {
                    self.toast(trf("The file dialog couldn't be shown: {error}", &[("error", &why)]), true);
                    Vec::new()
                }
                Answer::Cancelled | Answer::Message(_) => Vec::new(),
            };
            let moved_on = key.project.is_some_and(|project| project != self.session.project_id);
            if moved_on && !paths.is_empty() {
                self.toast(tr("The project changed while the dialog was open, so nothing was done"), true);
            }
            then(self, if moved_on { Vec::new() } else { paths });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> TextureCraftApp {
        let session = texturecraft_engine::Session::closed().unwrap();
        TextureCraftApp::new(&eframe::CreationContext::_new_kittest(egui::Context::default()), session)
    }

    #[test]
    fn an_app_without_a_window_answers_cancelled_instead_of_opening_a_dialog() {
        let mut app = app();
        let answered = std::rc::Rc::new(std::cell::Cell::new(None));
        let seen = answered.clone();
        assert!(app.ask_file(FileDialog::save().file_name("layout.json"), Bound::App, move |_, paths| seen.set(Some(paths.len()))));
        let frame = eframe::Frame::_new_kittest();
        for _ in 0..100 {
            app.file_dialogs.show_pending(&frame);
            app.run_file_dialog_answers();
            if answered.get().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(answered.get(), Some(0), "Cancelled: no paths");
    }

    #[test]
    #[should_panic(expected = "no file dialog override")]
    fn an_app_without_a_window_and_without_an_override_refuses_to_open_a_dialog() {
        let mut app = app();
        app.file_dialogs.set_override(None);
        app.ask_file(FileDialog::folder(), Bound::App, |_, _| {});
    }

    #[test]
    #[should_panic(expected = "no file dialog override")]
    fn a_dialog_queued_before_the_override_was_removed_still_never_opens() {
        let mut app = app();
        app.ask_file(FileDialog::save().file_name("layout.json"), Bound::App, |_, _| {});
        app.file_dialogs.set_override(None);
        app.file_dialogs.show_pending(&eframe::Frame::_new_kittest());
    }
}
