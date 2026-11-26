//! Bake mode: high poly files load on a worker thread, and the list names them while they do

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

/// A triangle the importer accepts as a high poly
const TRIANGLE_OBJ: &str = "o Part_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
/// Longest the test waits for the worker
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// Frames (5 ms apart) a stroke stays open while a high poly loads
const STROKE_FRAMES: usize = 200;

/// A bake-mode app with Common Settings in front
fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    harness.run_steps(3);
    harness
}

fn high_names(harness: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    harness.state().session.doc.bake_meshes.high.iter().map(|high| high.name.clone()).collect()
}

fn run_until_loaded(harness: &mut Harness<'_, TextureCraftApp>) {
    let started = Instant::now();
    while harness.state().high_poly_job.is_some() {
        assert!(started.elapsed() < LOAD_TIMEOUT, "the high polys never finished loading");
        std::thread::sleep(Duration::from_millis(5));
        harness.run_steps(1);
    }
}

#[test]
fn picked_high_polys_load_in_the_background_and_each_is_an_undo_step() {
    let folder = std::env::temp_dir().join(format!("texturecraft-high-poly-loading-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let paths: Vec<String> = ["first.obj", "second.obj"].map(|name| folder.join(name).display().to_string()).into();
    for path in &paths {
        std::fs::write(path, TRIANGLE_OBJ).unwrap();
    }
    let missing = folder.join("missing.obj").display().to_string();

    let mut harness = harness();
    let steps = harness.state().session.history_labels().len();
    harness.state_mut().load_high_polys([paths[0].clone(), missing, paths[1].clone()]);
    assert!(high_names(&harness).is_empty(), "the call returns before a file is read");
    assert_eq!(harness.state().high_poly_job.as_ref().unwrap().file_names().collect::<Vec<_>>(), ["first.obj", "missing.obj", "second.obj"]);
    harness.run_steps(1);
    harness.get_by_label("Loading second.obj...");

    run_until_loaded(&mut harness);
    harness.run_steps(2);

    assert_eq!(high_names(&harness), ["first.obj", "second.obj"], "the unreadable file is skipped and the rest load in order");
    assert_eq!(harness.state().session.history_labels().len(), steps + 2);
    assert!(harness.state().toasts.iter().any(|toast| toast.text.contains("missing.obj")), "the unreadable file is reported");
    assert!(harness.query_by_label("Loading second.obj...").is_none());
    assert_eq!(harness.state().session.doc.bake_meshes.merged_high().unwrap().mesh.triangle_count(), 2);

    harness.state_mut().run("edit.undo", serde_json::Value::Null);

    assert_eq!(high_names(&harness), ["first.obj"]);
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn high_polys_still_loading_when_another_project_opens_are_not_added_to_it() {
    let folder = std::env::temp_dir().join(format!("texturecraft-high-poly-moved-on-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("part.obj").display().to_string();
    std::fs::write(&path, TRIANGLE_OBJ).unwrap();

    let mut harness = harness();
    harness.state_mut().load_high_polys([path.clone(), path]);
    assert!(harness.state_mut().run("project.new", serde_json::json!({"mesh": "cube", "resolution": 128})).is_some());
    run_until_loaded(&mut harness);

    assert!(high_names(&harness).is_empty());
    assert!(harness.state().toasts.iter().any(|toast| toast.text.contains("not added")));
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn a_high_poly_that_finishes_after_the_list_changed_joins_the_list_as_it_is() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let file = |name: &str| texturecraft_engine::formats::MeshFile { name: name.into(), bytes: TRIANGLE_OBJ.as_bytes().to_vec(), warnings: Vec::new() };
    let load = texturecraft_engine::HighPolyLoad::prepare(&session.doc.bake_meshes, file("part.obj")).unwrap();

    session.add_high_poly_bytes("part.obj", TRIANGLE_OBJ.as_bytes().to_vec()).unwrap();

    assert_eq!(session.apply_high_poly_load(load), "2_part.obj", "the name is made unique against the list of now");
    assert_eq!(session.doc.bake_meshes.merged_high().unwrap().mesh.triangle_count(), 2, "both meshes bake");
}

#[test]
fn a_high_poly_read_during_a_paint_stroke_is_added_after_it_and_survives_its_cancel() {
    let folder = std::env::temp_dir().join(format!("texturecraft-high-poly-stroke-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("part.obj").display().to_string();
    std::fs::write(&path, TRIANGLE_OBJ).unwrap();

    let mut harness = harness();

    // Bake mode finishes strokes at once: the files go on loading after a switch back to painting
    harness.state_mut().ui.mode = Mode::Painting;
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.add.paint", &serde_json::json!({})).unwrap();
    harness.state_mut().session.begin_stroke_uv(texturecraft_engine::geom::glam::Vec2::splat(0.5), 1.0).unwrap();
    harness.state_mut().load_high_polys([path]);

    // Time for the worker to read one triangle
    for _ in 0..STROKE_FRAMES {
        std::thread::sleep(Duration::from_millis(5));
        harness.run_steps(1);
    }

    assert!(harness.state().session.stroke_active());
    assert!(high_names(&harness).is_empty(), "nothing is added while the stroke runs");

    harness.state_mut().session.cancel_stroke();
    run_until_loaded(&mut harness);
    assert_eq!(high_names(&harness), ["part.obj"]);
    let _ = std::fs::remove_dir_all(folder);
}
