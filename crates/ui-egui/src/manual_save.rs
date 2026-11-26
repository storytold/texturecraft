//! Manual desktop saves share the atomic archive writer with autosave and headless saves.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use texturecraft_engine::formats::project_file::SaveProgress;

use crate::{
    TextureCraftApp,
    i18n::{tr, trf},
    state::Unsaved,
};

mod save_job;

pub(crate) use save_job::SaveJob;

const SAVE_POLL_INTERVAL: Duration = Duration::from_millis(80);

pub(crate) fn is_save(id: &str) -> bool {
    matches!(id, "file.save" | "file.saveAs" | "file.saveCopy")
}

impl TextureCraftApp {
    pub(crate) fn start_save(&mut self, path: &str, adopt: bool, for_prompt: bool, remember: bool) -> bool {
        if self.save_job.is_some() {
            self.toast(tr("A project save is already running"), true);
            return false;
        }

        if let Err(error) = self.session.end_stroke() {
            self.toast(error.to_string(), true);
            return false;
        }

        let task = self.session.save_task(path, adopt);
        let progress = Arc::new(SaveProgress::default());
        let worker_progress = progress.clone();
        let output = crate::spawn_worker("texturecraft-save", move || task.run(Some(&worker_progress)));
        self.save_job = Some(SaveJob { progress, output, project_id: self.session.project_id, for_prompt, remember, quit_requested_by: None });
        true
    }

    pub(crate) fn hold_close_for_save(&mut self) -> bool {
        let Some(job) = &mut self.save_job else { return false };

        // A close on a replacement project still waits for the outstanding archive
        job.quit_requested_by = Some(self.session.project_id);
        true
    }

    pub(crate) fn cancel_close_after_save(&mut self) {
        self.quit_after_stroke = None;
        if let Some(job) = &mut self.save_job {
            job.quit_requested_by = None;
        }
    }

    pub(crate) fn resume_close_after_stroke(&mut self, ctx: &egui::Context) {
        let Some(project_id) = self.quit_after_stroke else { return };

        if project_id == self.session.project_id && self.session.stroke_active() {
            return;
        }

        self.quit_after_stroke = None;
        if project_id == self.session.project_id && !crate::windows::ask_to_save(self, Unsaved::Quit) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(crate) fn save_progress(&self) -> Option<f32> {
        self.save_job.as_ref().map(|job| job.progress.fraction())
    }

    pub(crate) fn poll_save(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.save_job else { return };

        let result = match job.output.try_recv() {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(SAVE_POLL_INTERVAL);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(tr("The project save worker stopped").into()),
        };

        let Some(job) = self.save_job.take() else { return };
        let same_project = job.project_id == self.session.project_id;
        let quit_requested = job.quit_requested_by == Some(self.session.project_id);
        let continue_prompt = same_project && job.for_prompt && self.saving_for_prompt;
        if continue_prompt {
            self.saving_for_prompt = false;
        }

        let save_succeeded = result.is_ok();
        match result {
            Ok(output) => {
                output.apply(&mut self.session);
                if job.remember {
                    crate::windows::remember(self, &output.path);
                }

                if continue_prompt {
                    self.prompt_saved = !self.session.modified && !self.session.stroke_active() && !quit_requested;
                }

                self.toast(trf("Saved {path}", &[("path", &output.path)]), false);
            }
            Err(error) => {
                let message = trf("Could not save project: {error}", &[("error", &error)]);
                self.session.log.warn("Project", message.clone());
                self.toast(message, true);
            }
        }

        if quit_requested {
            if self.session.stroke_active() {
                self.quit_after_stroke = Some(self.session.project_id);
            } else if !self.session.modified && (save_succeeded || !same_project) {
                // A replacement project's own close only needs the previous writer to finish
                self.ui.windows.unsaved = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                crate::windows::ask_to_save(self, Unsaved::Quit);
            }
        }

        if let Some(toast) = self.toasts.last_mut() {
            toast.announce = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{ViewportCommand, ViewportEvent, ViewportId};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use texturecraft_engine::{
        Result,
        file_cmds::{SaveOutput, SaveTask},
    };

    use super::*;

    fn harness() -> Harness<'static, TextureCraftApp> {
        let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
        session.modified = true;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 960.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|cc| TextureCraftApp::new(cc, session));

        // Match English labels independently of the system locale
        h.state_mut().ui.prefs.language = "en".into();
        h.run_steps(2);
        h
    }

    fn temp_file(test: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-manual-save-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("project.tcraft")
    }

    /// A save the test finishes by hand, so the close lands while it is still running
    fn start_held_save(h: &mut Harness<'_, TextureCraftApp>, path: &std::path::Path, adopt: bool) -> (SaveTask, mpsc::Sender<Result<SaveOutput>>) {
        let app = h.state_mut();
        let task = app.session.save_task(&path.display().to_string(), adopt);
        let (sender, output) = mpsc::channel();
        app.save_job =
            Some(SaveJob { progress: Arc::default(), output, project_id: app.session.project_id, for_prompt: false, remember: false, quit_requested_by: None });
        (task, sender)
    }

    fn request_close(h: &mut Harness<'_, TextureCraftApp>) {
        h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
        h.step();
        h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.clear();
        h.run_steps(2);
    }

    /// Hands the worker's result over and reports whether the app asked to close in the next frames
    fn finish(h: &mut Harness<'_, TextureCraftApp>, task: SaveTask, sender: &mpsc::Sender<Result<SaveOutput>>) -> bool {
        sender.send(task.run(None)).unwrap();
        sends_close(h)
    }

    /// Whether the app asks to close in the next frames
    fn sends_close(h: &mut Harness<'_, TextureCraftApp>) -> bool {
        (0..3).any(|_| {
            h.step();
            h.output().viewport_output.get(&ViewportId::ROOT).is_some_and(|v| v.commands.contains(&ViewportCommand::Close))
        })
    }

    fn prompt_is_usable(h: &Harness<'_, TextureCraftApp>) -> bool {
        ["Save", "Don't save", "Cancel"].iter().all(|label| !h.get_by_label(label).accesskit_node().is_disabled())
    }

    #[test]
    fn closing_during_a_regular_save_quits_once_it_has_saved() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("close-then-saved");
        let mut h = harness();
        h.state_mut().session.path = Some(file.display().to_string());
        let (task, sender) = start_held_save(&mut h, &file, true);

        request_close(&mut h);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the changes are not written yet");
        assert!(h.get_by_label("Save").accesskit_node().is_disabled(), "the prompt waits for the running save");
        let closed = finish(&mut h, task, &sender);

        assert!(closed, "the close goes on after the save");
        assert!(file.exists());
        assert!(!h.state().session.modified);
        assert!(h.state().ui.windows.unsaved.is_none(), "no question is left once the changes are saved");
    }

    #[test]
    fn closing_during_a_failed_save_keeps_a_usable_prompt() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("close-then-failed").with_file_name("missing").join("project.tcraft");
        let mut h = harness();
        let (task, sender) = start_held_save(&mut h, &file, true);

        request_close(&mut h);
        let closed = finish(&mut h, task, &sender);

        assert!(!closed);
        assert!(h.state().session.modified);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit));
        assert!(prompt_is_usable(&h), "the prompt answers again after the failed write");
    }

    #[test]
    fn closing_during_a_save_a_copy_keeps_the_prompt() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("close-then-copied");
        let mut h = harness();
        let (task, sender) = start_held_save(&mut h, &file, false);

        request_close(&mut h);
        let closed = finish(&mut h, task, &sender);

        assert!(!closed, "a copy leaves the project's own changes unsaved");
        assert!(file.exists());
        assert!(h.state().session.modified);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit));
        assert!(prompt_is_usable(&h));
    }

    #[test]
    fn closing_during_a_save_keeps_the_prompt_for_later_edits() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("close-then-edited");
        let mut h = harness();
        h.state_mut().session.path = Some(file.display().to_string());
        let (task, sender) = start_held_save(&mut h, &file, true);
        let rename = h.state_mut().run("textureSet.rename", serde_json::json!({"ts": 0, "name": "Edited while saving"}));
        assert!(rename.is_some());

        request_close(&mut h);
        let closed = finish(&mut h, task, &sender);

        assert!(!closed, "the edit made after the snapshot is not in the file");
        assert!(h.state().session.modified);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit));
        assert!(prompt_is_usable(&h));
    }

    #[test]
    fn an_edit_after_a_clean_close_asks_once_the_save_returns() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("clean-close-then-edited");
        let mut h = harness();
        h.state_mut().session.modified = false;
        h.state_mut().session.path = Some(file.display().to_string());
        let (task, sender) = start_held_save(&mut h, &file, true);

        request_close(&mut h);
        assert!(h.state().ui.windows.unsaved.is_none(), "nothing to ask while the project is clean");
        let rename = h.state_mut().run("textureSet.rename", serde_json::json!({"ts": 0, "name": "Edited while saving"}));
        assert!(rename.is_some());
        let closed = finish(&mut h, task, &sender);

        assert!(!closed);
        assert!(h.state().session.modified);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the close asks about the edit instead of being dropped");
        assert!(prompt_is_usable(&h));
    }

    /// A clean project closed during its save, with paint still held down when the save returns
    fn stroke_open_when_the_save_returns(test: &str) -> Harness<'static, TextureCraftApp> {
        let file = temp_file(test);
        let mut h = harness();
        h.state_mut().session.modified = false;
        h.state_mut().session.path = Some(file.display().to_string());
        let (task, sender) = start_held_save(&mut h, &file, true);
        request_close(&mut h);
        h.state_mut().session.begin_stroke_uv(texturecraft_engine::geom::glam::Vec2::splat(0.5), 1.0).unwrap();

        let closed = finish(&mut h, task, &sender);

        assert!(!closed, "the open stroke may still change the project");
        assert!(h.state().session.stroke_active());
        assert!(h.state().ui.windows.unsaved.is_none());
        h
    }

    #[test]
    fn a_stroke_open_when_the_save_returns_asks_once_it_is_painted() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = stroke_open_when_the_save_returns("stroke-painted");

        h.state_mut().session.end_stroke().unwrap();
        let closed = sends_close(&mut h);

        assert!(!closed);
        assert!(h.state().session.modified);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit), "the close asks about the painted stroke");
    }

    #[test]
    fn a_stroke_open_when_the_save_returns_quits_once_it_is_canceled() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = stroke_open_when_the_save_returns("stroke-canceled");

        h.state_mut().session.cancel_stroke();
        let closed = sends_close(&mut h);

        assert!(closed, "nothing is left to save, so the close goes on");
        assert!(h.state().ui.windows.unsaved.is_none());
    }

    #[test]
    fn opening_another_project_drops_the_close_that_waited_for_a_stroke() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut h = stroke_open_when_the_save_returns("stroke-replaced");

        h.state_mut().session = texturecraft_engine::sample_session(256, 64).unwrap();
        let closed = sends_close(&mut h);

        assert!(!closed, "the old project's close does not quit the new one");
        assert!(h.state().ui.windows.unsaved.is_none());
        assert!(h.state().quit_after_stroke.is_none());
    }

    #[test]
    fn replacing_the_project_during_a_save_drops_its_pending_close() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for stroke_active in [false, true] {
            let test = if stroke_active { "replace-saving-stroke" } else { "replace-saving-dirty" };
            let file = temp_file(test);
            let mut h = harness();
            h.state_mut().session.modified = false;
            let (task, sender) = start_held_save(&mut h, &file, true);

            request_close(&mut h);
            assert_eq!(h.state().save_job.as_ref().unwrap().quit_requested_by, Some(h.state().session.project_id));
            h.state_mut().session = texturecraft_engine::sample_session(256, 64).unwrap();
            if stroke_active {
                h.state_mut().session.begin_stroke_uv(texturecraft_engine::geom::glam::Vec2::splat(0.5), 1.0).unwrap();
            } else {
                assert!(h.state_mut().run("textureSet.rename", serde_json::json!({"ts": 0, "name": "Replacement edit"})).is_some());
            }

            assert!(!finish(&mut h, task, &sender));
            assert!(file.exists(), "the old archive still finished");
            assert!(h.state().session.path.is_none(), "the replacement does not adopt the old archive");
            assert!(h.state().ui.windows.unsaved.is_none(), "the old close does not prompt about the replacement");
            if stroke_active {
                h.state_mut().session.cancel_stroke();
                assert!(!sends_close(&mut h), "the old close does not survive the replacement's stroke");
            }
        }
    }

    #[test]
    fn replacing_a_project_with_an_open_quit_prompt_discards_its_answer() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("replace-prompt");
        let mut h = harness();
        let (task, sender) = start_held_save(&mut h, &file, true);
        request_close(&mut h);
        assert_eq!(h.state().ui.windows.unsaved, Some(Unsaved::Quit));

        let ctx = h.ctx.clone();
        let result = crate::control::handle(h.state_mut(), &ctx, "command.run", &serde_json::json!({ "id": "file.close" }));
        assert_eq!(result["ok"], true);
        assert!(!finish(&mut h, task, &sender));

        assert!(h.state().ui.windows.unsaved.is_none(), "the discarded project's prompt cannot act on the replacement");
    }

    #[test]
    fn closing_the_replacement_waits_for_the_original_archive() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("close-replacement");
        let mut h = harness();
        let (task, sender) = start_held_save(&mut h, &file, true);
        let replacement = texturecraft_engine::sample_session(256, 64).unwrap();
        h.state_mut().session.switch_to(replacement);

        assert!(crate::windows::close_requested(h.state_mut()), "the new close must hold the process open for the pending write");
        let closed = finish(&mut h, task, &sender);

        assert!(file.exists());
        assert!(closed, "the replacement's own close resumes once the writer finishes");
        assert!(h.state().session.path.is_none());
        assert!(h.state().ui.windows.unsaved.is_none());
    }

    #[test]
    fn closing_the_replacement_resumes_after_the_original_save_fails() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("close-replacement-after-failure").with_file_name("missing").join("project.tcraft");
        let mut h = harness();
        let (task, sender) = start_held_save(&mut h, &file, true);
        let replacement = texturecraft_engine::sample_session(256, 64).unwrap();
        h.state_mut().session.switch_to(replacement);

        request_close(&mut h);
        assert!(!sends_close(&mut h), "the replacement waits for the original save");
        let result = task.run(None);
        assert!(result.is_err(), "the original write fails");
        sender.send(result).unwrap();

        assert!(sends_close(&mut h), "the replacement's own close resumes after the failed write");
        assert!(h.state().ui.windows.unsaved.is_none());
    }

    #[test]
    fn canceling_a_stroke_after_a_successful_save_leaves_the_saved_edits_clean() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("saved-then-cancel-stroke");
        let mut h = harness();
        let (task, sender) = start_held_save(&mut h, &file, true);
        h.state_mut().session.begin_stroke_uv(texturecraft_engine::geom::glam::Vec2::splat(0.5), 1.0).unwrap();

        assert!(!finish(&mut h, task, &sender));
        h.state_mut().session.cancel_stroke();

        assert!(!crate::windows::close_requested(h.state_mut()), "the saved edits and canceled stroke need no second save");
    }

    #[test]
    fn dismissing_the_prompt_during_a_save_cancels_the_quit() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let file = temp_file("close-then-dismissed");
        let mut h = harness();
        h.state_mut().session.path = Some(file.display().to_string());
        let (task, sender) = start_held_save(&mut h, &file, true);
        request_close(&mut h);
        h.get_by_label("Close window").click();
        h.run_steps(2);
        assert!(h.state().ui.windows.unsaved.is_none(), "the prompt is dismissed while the save runs");

        let closed = finish(&mut h, task, &sender);

        assert!(!closed, "a dismissed prompt keeps the app open");
        assert!(!h.state().session.modified);
    }
}
