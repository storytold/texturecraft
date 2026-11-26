//! File > Open Sample Project starts its file dialog in a Samples folder that holds the built-in sample.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use egui_kittest::Harness;
use texturecraft_engine::Session;
use texturecraft_ui_egui::file_dialogs::{Answer, Request};
use texturecraft_ui_egui::{TextureCraftApp, menus};

const SAMPLE_FILE_NAME: &str = "Material Ball.tcraft";

fn closed_app(samples_folder: &Path) -> Harness<'static, TextureCraftApp> {
    let session = Session::closed().unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().samples_folder = samples_folder.to_path_buf();
    harness.run_steps(4);
    harness
}

/// Answers every file dialog with `pick(folder the dialog starts in)` and records that folder
fn answer_with(harness: &mut Harness<'static, TextureCraftApp>, pick: impl Fn(Option<&Path>) -> Answer + 'static) -> Rc<RefCell<Vec<Option<PathBuf>>>> {
    let started_in = Rc::new(RefCell::new(Vec::new()));
    let recorded = started_in.clone();
    harness.state_mut().file_dialogs.set_override(Some(Box::new(move |request| {
        let Request::File(dialog) = request else { return Answer::Cancelled };
        recorded.borrow_mut().push(dialog.directory.clone());
        pick(dialog.directory.as_deref())
    })));
    started_in
}

fn open_sample_menu_entry(harness: &mut Harness<'static, TextureCraftApp>) {
    let ctx = harness.ctx.clone();
    menus::invoke(harness.state_mut(), &ctx, "project.sample");
    harness.run_steps(3);
}

#[test]
fn the_menu_entry_copies_the_shipped_sample_and_the_dialog_starts_in_its_folder() {
    let root = super::temp_dir::TempDir::new("sample-project");
    let samples_folder = root.join("Samples");
    let mut harness = closed_app(&samples_folder);
    let started_in = answer_with(&mut harness, |folder| Answer::Paths(vec![folder.unwrap().join(SAMPLE_FILE_NAME)]));

    open_sample_menu_entry(&mut harness);

    assert!(samples_folder.join(SAMPLE_FILE_NAME).is_file(), "the shipped sample is copied on first use");
    assert_eq!(*started_in.borrow(), vec![Some(samples_folder.clone())], "the dialog starts in the Samples folder");
    assert!(harness.state().session.project_open, "the picked sample opens");
    assert_eq!(harness.state().session.doc.project.name, "Material Ball");
    assert_eq!(harness.state().session.set_count(), 2, "the sample keeps its texture sets");
    assert!(!harness.state().session.modified, "an opened sample is a clean file");
}

#[test]
fn a_file_already_in_the_samples_folder_is_not_overwritten() {
    let root = super::temp_dir::TempDir::new("sample-project-kept");
    let samples_folder = root.join("Samples");
    std::fs::create_dir_all(&samples_folder).unwrap();
    let existing = samples_folder.join(SAMPLE_FILE_NAME);
    std::fs::write(&existing, b"the user's own file").unwrap();
    let mut harness = closed_app(&samples_folder);
    let started_in = answer_with(&mut harness, |_| Answer::Cancelled);

    open_sample_menu_entry(&mut harness);

    assert_eq!(std::fs::read(&existing).unwrap(), b"the user's own file");
    assert_eq!(*started_in.borrow(), vec![Some(samples_folder)], "the dialog still starts in the Samples folder");
}

#[test]
fn a_samples_folder_that_cannot_be_made_is_reported_and_the_dialog_still_opens() {
    let root = super::temp_dir::TempDir::new("sample-project-blocked");
    let blocker = root.join("Samples");
    std::fs::write(&blocker, b"a file where the folder should be").unwrap();
    let mut harness = closed_app(&blocker);
    let started_in = answer_with(&mut harness, |_| Answer::Cancelled);

    open_sample_menu_entry(&mut harness);

    assert!(harness.state().toasts.iter().any(|toast| toast.error), "the failure is shown");
    assert_eq!(*started_in.borrow(), vec![None], "the dialog opens without a start folder");
    assert!(!harness.state().session.project_open);
}
