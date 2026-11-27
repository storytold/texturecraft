//! The desktop autosave writes each project's copy to a file of its own. This is a test binary of its own because it
//! points the process's data folder at a temporary one

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::TextureCraftApp;

/// App time each frame advances by, so a few frames pass any autosave interval
const FRAME_SECONDS: f32 = 600.0;

/// How long the worker thread gets to write a copy
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// A project named as one made from `hero.fbx` would be, with `layers` paint layers added since it was saved
fn unsaved_hero(layers: usize) -> Session {
    let mut session = Session::new_project("hero", primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    for _ in 0..layers {
        execute(&mut session, "layer.add.paint", &json!({})).unwrap();
    }
    session
}

fn copies(folder: &Path) -> Vec<PathBuf> {
    let files = std::fs::read_dir(folder).map(|entries| entries.flatten().map(|entry| entry.path()).collect::<Vec<_>>()).unwrap_or_default();
    files.into_iter().filter(|path| path.extension().is_some_and(|extension| extension == "tcraft")).collect()
}

/// Runs an app on `session` until the autosave folder holds `count` copies
fn autosave(session: Session, folder: &Path, count: usize) -> Harness<'static, TextureCraftApp> {
    let mut harness = Harness::builder()
        .with_step_dt(FRAME_SECONDS)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    let started = Instant::now();
    while copies(folder).len() < count {
        harness.step();
        assert!(started.elapsed() < WRITE_TIMEOUT, "autosave wrote {:?}", copies(folder));
        std::thread::sleep(Duration::from_millis(20));
    }
    harness
}

#[test]
fn two_projects_of_one_name_keep_two_autosave_copies() {
    let data = std::env::temp_dir().join(format!("tc-autosave-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    texturecraft_ui_egui::set_data_root(data.clone());
    let folder = data.join("autosave");

    let first = autosave(unsaved_hero(1), &folder, 1);
    let second = autosave(unsaved_hero(2), &folder, 2);
    drop((first, second));

    // Each copy opens as the project that wrote it
    let mut layers: Vec<usize> =
        copies(&folder).iter().map(|path| texturecraft_engine::file_cmds::open_path(path.to_str().unwrap()).unwrap().doc.project.layers.len()).collect();
    layers.sort_unstable();
    assert_eq!(layers[1], layers[0] + 1, "{layers:?}");

    std::fs::remove_dir_all(&data).unwrap();
}
