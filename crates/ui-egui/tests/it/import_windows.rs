//! The project import's two windows: the notice must be acknowledged before anything runs, the progress window turns
//! into the error view on a failure, and programmatic calls open no window.

use egui::OutputCommand;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use std::sync::{Arc, Mutex};

use texturecraft_engine::import_cmds::{ImportOutput, ImportProgress, ImportTask, PROJECT_EXT, Step};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::import_ui::{self, ImportUi};

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
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

/// The progress window stays until the converted project's view is composited (the run's last step, on the worker),
/// the bar never goes back, and the project opens with nothing left to composite.
#[test]
fn the_progress_window_stays_until_the_view_is_composited() {
    let mut h = harness();
    let mut converted = texturecraft_engine::sample_session(1024, 256).unwrap();
    converted.doc.project.name = "Ring".into();
    let tiles = converted.pending_tiles();
    assert!(tiles > 0, "a fresh session has its view to composite");
    let seen = Arc::new(Mutex::new(Vec::<(f32, Step)>::new()));
    let log = seen.clone();
    let progress = Arc::new(ImportProgress::with_listener(move |f, step| log.lock().unwrap().push((f, step))));
    progress.report(0.88, Step::Saving);
    // the worker composites the view only when let go, so that the window can be watched waiting for it
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = progress.clone();
    std::thread::spawn(move || {
        let mut out = ImportOutput::from_session(converted);
        let _ = wait.recv();
        let _ = tx.send(out.compute_view(&worker).map(|()| out));
    });
    let started = h.ctx.input(|i| i.time);
    h.state_mut().import = Some(ImportUi::Running { file: "Ring.file".into(), path: None, progress: progress.clone(), rx, started });
    h.run_steps(5);
    assert!(matches!(h.state().import, Some(ImportUi::Running { .. })), "the window waits for the view");
    assert!(h.query_by_label_contains("Converting Ring.file").is_some());
    assert!(h.query_by_label_contains("Saving").is_some());
    assert_eq!(h.state().session.project().name, "Material Ball", "nothing opens before the view is done");
    go.send(()).unwrap();
    // the window shows every frame until the worker hands the project over
    let mut fractions = Vec::new();
    for _ in 0..3000 {
        h.step();
        match &h.state().import {
            Some(ImportUi::Running { progress, .. }) => {
                fractions.push(progress.fraction());
                assert!(h.query_by_label_contains("Converting Ring.file").is_some(), "the window shows while the view is computed");
            }
            _ => break,
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(h.state().import.is_none(), "the project opened");
    assert!(fractions.windows(2).all(|w| w[0] <= w[1]), "the bar never goes back: {fractions:?}");
    let seen = seen.lock().unwrap().clone();
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "reports never go back: {seen:?}");
    assert!(seen.iter().any(|(_, s)| *s == Step::ComputingView), "the view step is reported: {seen:?}");
    assert_eq!(seen.last(), Some(&(1.0, Step::ComputingView)), "the bar ends full once the view is done");
    assert_eq!(h.state().session.pending_tiles(), 0, "the project opens with its view composited");
    h.run_steps(3);
    assert_eq!(h.state().session.pending_tiles(), 0, "and nothing is composited again");
    assert_eq!(h.state().session.project().name, "Ring");
}

/// The control channel drives the import windows as a person would: open the file, acknowledge the notice, read the
/// status.
#[test]
fn the_control_channel_drives_the_import_windows() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let status = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.import.status", &json!({}));
    assert_eq!(status["result"]["window"], "none", "{status}");
    let opened = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.import.open", &json!({"path": format!("/nonexistent/x.{PROJECT_EXT}")}));
    assert_eq!(opened["result"]["window"], "notice", "{opened}");
    let started = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.import.convert", &json!({}));
    assert_eq!(started["result"]["window"], "running", "{started}");
    until_finished(&mut h);
    let failed = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.import.status", &json!({}));
    assert_eq!(failed["result"]["window"], "failed", "{failed}");
    let again = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.import.convert", &json!({}));
    assert_eq!(again["ok"], false, "no notice showing: {again}");
}
