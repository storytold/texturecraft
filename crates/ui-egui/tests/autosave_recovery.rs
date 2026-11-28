//! Work recovered from an autosave copy survives the next autosave of the project the copy came from. This is a test
//! binary of its own because it points the process's data folder at a temporary one

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use serde_json::{Value, json};
use texturecraft_engine::core::PROJECT_EXT;
use texturecraft_engine::formats::RamReservation;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::{TextureCraftApp, control};

/// App time each frame advances by, so a few frames pass any autosave interval
const FRAME_SECONDS: f32 = 600.0;

/// Deadline for the background archive write
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// Allow the worker to make progress between test frames.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

fn load_project(path: &Path) -> Option<Session> {
    texturecraft_engine::file_cmds::open_path(path.to_str().unwrap()).ok()
}

fn layers(session: &Session) -> usize {
    session.doc.project.layers.len()
}

fn start(session: Session) -> Harness<'static, TextureCraftApp> {
    Harness::builder()
        .with_step_dt(FRAME_SECONDS)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session))
}

fn add_layers(app: &mut TextureCraftApp, count: usize) {
    for _ in 0..count {
        execute(&mut app.session, "layer.add.paint", &json!({})).unwrap();
    }
}

/// Wait for an autosave containing the requested number of layers.
fn autosave(harness: &mut Harness<'_, TextureCraftApp>, folder: &Path, layer_count: usize) -> PathBuf {
    let started = Instant::now();
    loop {
        if let Some(reservation) = RamReservation::try_for_save() {
            // Wait for any previous archive write before advancing another frame
            let found = std::fs::read_dir(folder).into_iter().flatten().filter_map(Result::ok).map(|entry| entry.path()).find(|path| {
                path.extension().is_some_and(|extension| extension == PROJECT_EXT) && load_project(path).is_some_and(|session| layers(&session) == layer_count)
            });
            drop(reservation);
            if let Some(copy) = found {
                return copy;
            }

            harness.step();
        }

        assert!(started.elapsed() < WRITE_TIMEOUT, "autosave wrote no copy of {layer_count} layers");
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[test]
fn recovered_work_survives_the_originals_next_autosave() {
    let data = tempfile::tempdir().unwrap();
    texturecraft_ui_egui::set_data_root(data.path().to_path_buf());
    let folder = data.path().join("autosave");
    let original_path = data.path().join("hero.tcraft");
    let original_params = json!({ "path": original_path.to_str().unwrap() });

    let mut session = Session::new_project("hero", primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    execute(&mut session, "file.saveAs", &original_params).unwrap();
    let saved_layer_count = layers(&session);

    let mut harness = start(session);
    add_layers(harness.state_mut(), 1);
    let original_copy = autosave(&mut harness, &folder, saved_layer_count + 1);
    let copy_params = json!({ "path": original_copy.to_str().unwrap() });

    let opened = harness.state_mut().run("file.open", copy_params.clone()).unwrap();
    assert_eq!(harness.state().session.path, None);
    assert_eq!(opened.get("path"), Some(&Value::Null));
    assert_eq!(opened["modified"], true);
    assert!(harness.state_mut().run("file.save", json!({})).is_none(), "saving the recovered project requires a destination");

    add_layers(harness.state_mut(), 3);
    let recovered_layer_count = layers(&harness.state().session);
    let recovered_copy = autosave(&mut harness, &folder, recovered_layer_count);
    assert_ne!(recovered_copy, original_copy);

    let kept_path = data.path().join("kept.tcraft");
    harness.state_mut().run("file.saveAs", json!({ "path": kept_path })).unwrap();
    assert_eq!(harness.state().session.path.as_deref(), kept_path.to_str());

    let opened = harness.state_mut().run("file.open", original_params.clone()).unwrap();
    assert_eq!(opened["path"], original_params["path"]);
    assert_eq!(opened["modified"], false);
    add_layers(harness.state_mut(), 2);
    assert_eq!(autosave(&mut harness, &folder, saved_layer_count + 2), original_copy, "the original keeps its copy");
    assert_eq!(layers(&load_project(&recovered_copy).unwrap()), recovered_layer_count);
    assert_eq!(layers(&load_project(&kept_path).unwrap()), recovered_layer_count);

    // The control reply reports the same recovery state as session inspection
    let ctx = harness.ctx.clone();
    let reply = control::handle(harness.state_mut(), &ctx, "command.run", &json!({ "id": "file.open", "params": copy_params }));
    let inspected = control::handle(harness.state_mut(), &ctx, "session.inspect", &json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(harness.state().session.path, None);
    assert_eq!(reply["result"]["path"], inspected["result"]["path"]);
    assert_eq!(reply["result"]["modified"], inspected["result"]["modified"]);

    // A case-insensitive volume resolves this spelling to the same archive
    let alias = original_copy.with_file_name(original_copy.file_name().unwrap().to_str().unwrap().replace("-copy-", "-CoPy-")).with_extension("TCRAFT");
    let case_sensitive = !alias.exists();
    if case_sensitive {
        // Exercise the same identity check with two names for one file
        std::fs::hard_link(&original_copy, &alias).unwrap();
    }

    let opened = harness.state_mut().run("file.open", json!({ "path": alias })).unwrap();
    assert_eq!(opened.get("path"), Some(&Value::Null));
    assert_eq!(opened["modified"], true);

    if case_sensitive {
        std::fs::remove_file(&alias).unwrap();
        std::fs::copy(&original_copy, &alias).unwrap();

        let opened = harness.state_mut().run("file.open", json!({ "path": alias })).unwrap();
        assert_eq!(opened["path"], alias.to_str().unwrap());
        assert_eq!(opened["modified"], false);
    }

    // The launcher gives the app a session that has already loaded the archive
    let startup = start(load_project(&original_copy).unwrap());
    assert_eq!(startup.state().session.path, None);
    assert!(startup.state().session.modified);
}
