//! The Assets panel's refresh popup: automatic updates of the panel and of the project's images

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::brush_preset_cmds::UserBrush;
use texturecraft_ui_egui::TextureCraftApp;

/// A little more than the shortest update interval, in seconds
const PAST_A_MINUTE: f64 = 61.0;

/// The sample project with the Assets panel open and its refresh popup shown
fn refresh_popup() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1032.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(2);
    harness.get_all_by_label("Refresh").next().unwrap().click();
    harness.run_steps(2);
    harness
}

/// Moves the app's clock on by more than a minute in one frame, then lets the frames after it settle
fn pass_a_minute(harness: &mut Harness<'_, TextureCraftApp>) {
    let now = harness.ctx.input(|input| input.time);
    harness.input_mut().time = Some(now + PAST_A_MINUTE);
    harness.run_steps(2);
}

/// Writes `bytes` over the file at `path` with a later time, as a later save would (an equal-size file keeping its time reads as unchanged)
fn rewrite(path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
}

/// A grey one-pixel PNG of `value`
fn grey_png(value: f32) -> Vec<u8> {
    texturecraft_engine::formats::gray16_png(1, 1, [value].into_iter()).unwrap()
}

#[test]
fn turning_on_project_resources_reads_a_changed_image_file_again_every_minute() {
    let mut harness = refresh_popup();
    let path = std::env::temp_dir().join(format!("texturecraft-auto-update-{}.png", std::process::id()));
    std::fs::write(&path, grey_png(0.2)).unwrap();
    let name = harness.state_mut().run("resource.import", json!({"path": path.to_str().unwrap()})).unwrap()["name"].as_str().unwrap().to_string();
    let before = harness.state().session.images[&name].rgba.clone();
    rewrite(&path, &grey_png(0.8));

    pass_a_minute(&mut harness);
    let while_off = harness.state().session.images[&name].rgba.clone();
    harness.get_by_label("Resources used in project").click();
    harness.run_steps(2);
    pass_a_minute(&mut harness);
    let after = harness.state().session.images[&name].rgba.clone();
    std::fs::remove_file(path).unwrap();

    assert_eq!(while_off, before, "nothing is read again while automatic updates are off");
    assert!(harness.state().ui.shelf_updates.resources, "the switch turned the update on");
    assert_ne!(after, before, "the changed file was read again within a minute");
}

#[test]
fn turning_on_the_assets_panel_reads_the_saved_brushes_again_every_minute() {
    let mut harness = refresh_popup();
    let stale = UserBrush { name: "Stale brush marker".into(), brush: Default::default(), channels: None, file: None };
    harness.state_mut().session.brush_presets.list.push(stale);

    harness.get_by_label("Assets panel").click();
    harness.run_steps(2);
    pass_a_minute(&mut harness);

    assert!(harness.state().session.brush_presets.find("Stale brush marker").is_none(), "the brushes folder was read again");
}

#[test]
fn an_automatic_update_waits_for_the_stroke_under_way_to_end() {
    let mut harness = refresh_popup();
    let path = std::env::temp_dir().join(format!("texturecraft-auto-update-stroke-{}.png", std::process::id()));
    std::fs::write(&path, grey_png(0.2)).unwrap();
    let name = harness.state_mut().run("resource.import", json!({"path": path.to_str().unwrap()})).unwrap()["name"].as_str().unwrap().to_string();
    let before = harness.state().session.images[&name].rgba.clone();
    harness.get_by_label("Resources used in project").click();
    harness.run_steps(2);
    rewrite(&path, &grey_png(0.8));

    let steps_before_stroke = harness.state().session.history_labels().len();
    harness.state_mut().session.begin_stroke_uv(texturecraft_engine::geom::glam::Vec2::splat(0.5), 1.0).unwrap();
    pass_a_minute(&mut harness);
    let during_stroke = harness.state().session.images[&name].rgba.clone();
    harness.state_mut().session.end_stroke().unwrap();
    harness.run_steps(2);
    let after_stroke = harness.state().session.images[&name].rgba.clone();
    std::fs::remove_file(path).unwrap();

    assert_eq!(during_stroke, before, "nothing is read again while a stroke is under way");
    assert_ne!(after_stroke, before, "the update runs once the stroke ended");
    assert_eq!(harness.state().session.doc.resources[&name].as_slice(), grey_png(0.8).as_slice(), "the stroke's undo state didn't swallow the update");

    harness.state_mut().session.undo().unwrap();

    assert_eq!(harness.state().session.doc.resources[&name].as_slice(), grey_png(0.2).as_slice(), "one undo takes back the update");
    assert_eq!(harness.state().session.history_labels().len(), steps_before_stroke + 1, "the stroke is still its own step");
}

#[test]
fn a_file_that_fails_to_read_is_reported_again_in_the_next_project() {
    let mut harness = refresh_popup();
    harness.get_by_label("Resources used in project").click();
    harness.run_steps(2);

    let mut reported = Vec::new();
    let mut retained = Vec::new();
    for project in ["first", "second", "third"] {
        if project != "first" {
            harness.state_mut().session = texturecraft_engine::sample_session(256, 64).unwrap();
        }
        let folder = std::env::temp_dir().join(format!("texturecraft-failed-read-{project}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("missing-update-photo.png");
        std::fs::write(&path, grey_png(0.2)).unwrap();
        let name = harness.state_mut().run("resource.import", json!({"path": path.to_str().unwrap()})).unwrap()["name"].as_str().unwrap().to_string();
        std::fs::remove_dir_all(&folder).unwrap();
        harness.state_mut().toasts.clear();

        pass_a_minute(&mut harness);

        let allocations = harness.ctx.data(|data| {
            data.iter_temp_by_type::<Vec<String>>().filter(|names| names.contains(&name)).fold((0, 0), |(count, bytes), names| {
                (count + 1, bytes + names.capacity() * size_of::<String>() + names.iter().map(String::capacity).sum::<usize>())
            })
        });
        retained.push(allocations);
        reported.push(harness.state().toasts.iter().any(|toast| toast.error && toast.text.starts_with(&format!("{name}:"))));
    }

    println!("Cached missing-file reports (lists, bytes) for successive projects: {retained:?}");
    assert_eq!(retained[0].0, 1, "the active project remembers the report");
    assert_eq!(retained, [retained[0]; 3], "each replacement releases the previous project's failure list");
    assert_eq!(reported, [true; 3], "each project says once that its file can't be read");
}
