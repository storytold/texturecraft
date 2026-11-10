//! The project import's two windows: the notice must be acknowledged before anything runs, the progress window turns
//! into the error view on a failure, and programmatic calls open no window.

use egui::OutputCommand;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::import_cmds::{ImportTask, PROJECT_EXT};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::import_ui::{self, ImportUi};

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

/// Steps until the conversion is no longer running (it runs on a worker thread).
fn until_finished(h: &mut Harness<'_, TextureCraftApp>) {
    for _ in 0..600 {
        h.step();
        if !matches!(h.state().import, Some(ImportUi::Running { .. })) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the conversion did not finish");
}

fn copied(h: &Harness<'_, TextureCraftApp>) -> Option<String> {
    h.output().platform_output.commands.iter().find_map(|c| match c {
        OutputCommand::CopyText(t) => Some(t.clone()),
        _ => None,
    })
}

#[test]
fn the_notice_runs_nothing_until_acknowledged_and_cancel_closes_it() {
    let mut h = harness();
    import_ui::ask(h.state_mut(), ImportTask::bytes("broken.file", vec![0, 1, 2]));
    h.run_steps(3);
    assert!(h.query_by_label_contains("The original file is never changed").is_some());
    assert!(h.query_by_label_contains("will not match the original project 100%").is_some());
    assert!(h.query_by_label_contains("fairly large amount of memory").is_some());
    assert!(h.query_by_label_contains("expected to improve").is_some());
    assert!(h.query_by_label_contains("please open an issue").is_some());
    h.run_steps(5);
    assert!(matches!(h.state().import, Some(ImportUi::Notice { .. })), "nothing runs while the notice waits");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().import.is_none());
    assert_eq!(h.state().session.project().name, "Material Ball");
}

#[test]
fn a_failed_conversion_turns_the_progress_window_into_the_error_view() {
    let mut h = harness();
    import_ui::ask(h.state_mut(), ImportTask::bytes("broken.file", vec![0, 1, 2]));
    h.run_steps(2);
    h.get_by_label("I understand, convert").click();
    h.step();
    until_finished(&mut h);
    h.run_steps(2);
    assert!(matches!(h.state().import, Some(ImportUi::Failed { .. })), "the same window shows the error");
    assert!(h.query_by_label_contains("The conversion failed").is_some());
    assert!(h.query_by_label_contains("github.com/storytold/texturecraft/issues").is_some());
    h.get_by_label("Copy details").click();
    h.step();
    let details = copied(&h).expect("details copied");
    assert!(details.starts_with("TextureCraft ") && details.contains("\nStep: reading the file\nError: "), "{details}");
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert!(h.state().import.is_none());
    assert_eq!(h.state().session.project().name, "Material Ball", "a failed import leaves the project open");
}

#[test]
fn opening_a_project_file_of_the_reference_app_asks_with_the_notice() {
    let mut h = harness();
    h.state_mut().session.modified = false;
    texturecraft_ui_egui::windows::open_path(h.state_mut(), &format!("/nonexistent/folder/file.{PROJECT_EXT}"));
    h.run_steps(2);
    assert!(matches!(h.state().import, Some(ImportUi::Notice { .. })));
    assert!(h.query_by_label_contains("Import file.").is_some(), "the notice names the file");
}

#[test]
fn programmatic_imports_open_no_window() {
    let mut h = harness();
    assert!(h.state_mut().run("file.importProject", json!({"bytes": [0, 1, 2]})).is_none(), "the error is a toast");
    h.run_steps(2);
    assert!(h.state().import.is_none());
}
