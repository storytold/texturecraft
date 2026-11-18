//! The app starts with no project and no popup; File > Close goes back to that empty state.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::Session;
use texturecraft_ui_egui::file_dialogs::Answer;
use texturecraft_ui_egui::{TextureCraftApp, menus};

fn harness(session: Session) -> Harness<'static, TextureCraftApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The exact-text checks below need English, whatever the OS language is
    harness.state_mut().ui.prefs.language = "en".into();

    // Menu items that ask for a file (Save UI layout, ...) are answered with Cancel instead of opening a native dialog
    harness.state_mut().file_dialogs.set_override(Some(Box::new(|_| Answer::Cancelled)));
    harness.run_steps(4);
    harness
}

fn empty() -> Harness<'static, TextureCraftApp> {
    harness(Session::closed().unwrap())
}

fn open_project() -> Harness<'static, TextureCraftApp> {
    harness(texturecraft_engine::sample_session(256, 64).unwrap())
}

/// Texts painted this frame as (text, left edge)
fn painted(harness: &Harness<'static, TextureCraftApp>) -> Vec<(String, f32)> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some((text.galley.text().to_string(), text.pos.x)),
            _ => None,
        })
        .collect()
}

fn has_camera_picker(harness: &Harness<'static, TextureCraftApp>) -> bool {
    painted(harness).iter().any(|(text, _)| text == "Default camera")
}

fn invoke(harness: &mut Harness<'static, TextureCraftApp>, id: &str) {
    let ctx = harness.ctx.clone();
    menus::invoke(harness.state_mut(), &ctx, id);
    harness.run_steps(3);
}

#[test]
fn starts_with_no_project_and_no_popup() {
    let harness = empty();
    let ui = &harness.state().ui;

    assert!(!harness.state().session.project_open);
    assert!(!ui.windows.new_project && !ui.windows.about && !ui.windows.settings && !ui.windows.export, "no window opens by itself");
    assert!(harness.query_by_label("Continue with the sample project").is_none() && harness.query_by_label("Recent projects").is_none(), "no start window");
    assert!(harness.query_by_label("No properties found").is_some(), "Properties says it has nothing");
}

#[test]
fn project_items_are_disabled_without_a_project() {
    let harness = empty();
    let app = harness.state();
    for id in [
        "file.save",
        "file.saveAs",
        "file.saveCopy",
        "file.close",
        "ui.export",
        "file.exportMesh",
        "ui.projectConfig",
        "edit.undo",
        "edit.redo",
        "file.reimportMesh",
        "ui.mode.bake",
        "ui.mode.render",
        "ui.uvChecker",
        "viewport.displayMaterial",
        "tool.paint",
        "tool.eraser",
        "viewport.quickMask",
        "layer.add.paint",
        "layer.add.fill",
        "layer.remove",
        "bake.run",
        "resource.import",
        "material.load",
    ] {
        assert!(!menus::enabled(app, id), "{id} needs a project");
    }
    for id in
        ["ui.newProject", "file.open", "project.sample", "ui.settings", "ui.about", "ui.quit", "ui.hideUi", "ui.resetUi", "viewport.frame", "ui.layout.split"]
    {
        assert!(menus::enabled(app, id), "{id} works without a project");
    }
    let open = open_project();
    for id in ["file.save", "file.close", "ui.export", "layer.add.paint", "ui.mode.bake"] {
        assert!(menus::enabled(open.state(), id), "{id} is on with a project");
    }
}

#[test]
fn the_empty_views_keep_the_view_picker_and_drop_the_camera_picker() {
    let empty = empty();

    assert!(!has_camera_picker(&empty), "no camera picker");

    // The picker shows "Material" right of the tool shelf and the Assets panel: one per view in the split layout
    let pickers = painted(&empty).iter().filter(|(text, x)| text == "Material" && *x > 400.0).count();

    assert_eq!(pickers, 2, "each of the two views keeps its view picker");

    let open = open_project();

    assert!(has_camera_picker(&open), "with a project the camera picker is there");
}

#[test]
fn the_status_bar_has_the_version_but_no_frame_time_or_layer_memory() {
    for harness in [empty(), open_project()] {
        assert!(harness.query_by_label_contains("Version:").is_some());
        assert!(harness.query_by_label_contains("Layer memory").is_none());
        assert!(harness.query_by_label_contains(" ms").is_none(), "no frame time");
    }
}

#[test]
fn close_unloads_the_project_back_to_the_empty_state() {
    let mut harness = open_project();

    assert!(harness.state().session.project_open && harness.state().session.set_count() == 2);

    invoke(&mut harness, "file.close");

    assert!(!harness.state().session.project_open, "Close leaves no project open");
    assert!(harness.state().session.path.is_none() && !harness.state().session.modified);
    assert!(harness.query_by_label("No properties found").is_some());
    assert!(!has_camera_picker(&harness));
    assert!(!menus::enabled(harness.state(), "file.save"));
}

#[test]
fn creating_or_opening_a_project_from_the_empty_state_works() {
    let mut harness = empty();

    assert!(harness.state_mut().run("project.new", json!({"mesh": "cube", "resolution": 128})).is_some());
    harness.run_steps(3);

    assert!(harness.state().session.project_open);
    assert!(menus::enabled(harness.state(), "file.save") && has_camera_picker(&harness));

    invoke(&mut harness, "file.close");
    invoke(&mut harness, "ui.newProject");

    assert!(harness.state().ui.windows.new_project, "New project opens its dialog from the empty state");

    let mut harness = empty();

    invoke(&mut harness, "project.sample");

    assert!(harness.state().session.project_open && harness.state().session.set_count() == 2, "the sample is still one menu click away");
}

#[test]
fn using_every_panel_and_menu_item_in_the_empty_state_does_not_panic() {
    let mut harness = empty();
    for item in menus::CATALOG
        .iter()
        .filter(|item| item.label != "-" && !matches!(item.id, "ui.quit" | "file.open" | "project.sample" | "ui.layout.load" | "ui.help.checkForUpdates"))
    {
        invoke(&mut harness, item.id);
    }
    for mode in ["ui.mode.bake", "ui.mode.render", "ui.mode.paint"] {
        invoke(&mut harness, mode);
    }
    for id in ["ui.layout.2d", "ui.layout.3d", "ui.layout.split"] {
        invoke(&mut harness, id);
    }
    assert!(!harness.state().session.project_open, "none of these opened a project");
    assert!(!harness.state().session.can_undo(), "and none changed it");
}

#[test]
fn close_drops_the_pending_resize_and_the_viewport_images() {
    let mut harness = open_project();
    harness.state_mut().start_resize(0, 128);

    assert!(harness.state().resize.is_some(), "a resize is running");

    let had_image = texturecraft_ui_egui::control::screenshot(harness.state(), texturecraft_ui_egui::viewport::VIEW_3D).is_some();

    invoke(&mut harness, "file.close");

    assert!(harness.state().resize.is_none(), "Close drops the resize of the closed project");
    assert!(!harness.state().session.modified, "a late resize result does not mark the empty project modified");
    if had_image {
        assert!(
            texturecraft_ui_egui::control::screenshot(harness.state(), texturecraft_ui_egui::viewport::VIEW_3D).is_none(),
            "no image of the closed project"
        );
    }
}

fn assert_eventually_released<T: ?Sized>(reference: &std::sync::Weak<T>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while reference.strong_count() > 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert_eq!(reference.strong_count(), 0, "the old project must be released after background work finishes");
}

#[test]
fn control_project_replacement_discards_the_old_resize() {
    let mut harness = open_project();
    let resource = std::sync::Arc::new(vec![0; 1_048_576]);
    let old_resource = std::sync::Arc::downgrade(&resource);
    harness.state_mut().session.doc.resources.insert("lifetime-probe.bin".into(), resource);
    let old_mesh = std::sync::Arc::downgrade(&harness.state().session.mesh);
    harness.state_mut().start_resize(0, 128);

    assert!(harness.state().resize.is_some());

    let resize_progress = std::sync::Arc::downgrade(&harness.state().resize.as_ref().unwrap().progress);
    let owners_before = resize_progress.strong_count();

    let ctx = harness.ctx.clone();
    let reply = texturecraft_ui_egui::control::handle(
        harness.state_mut(),
        &ctx,
        "command.run",
        &json!({"id": "project.new", "params": {"mesh": "cube", "resolution": 128}}),
    );

    assert_eq!(reply["ok"], true);
    assert!(harness.state().session.project_open);
    assert!(harness.state().resize.is_none(), "the previous project's resize must be discarded");

    let owners_after = resize_progress.strong_count();
    eprintln!("control resize progress owners: {owners_before} before, {owners_after} after replacement");

    assert!(owners_after < owners_before && owners_after <= 1);

    assert_eventually_released(&resize_progress);
    assert_eventually_released(&old_mesh);
    assert_eventually_released(&old_resource);
    eprintln!(
        "released old mesh and 1_048_576-byte resource after replacement: mesh={}, resource={}, progress={}",
        old_mesh.strong_count(),
        old_resource.strong_count(),
        resize_progress.strong_count()
    );

    harness.run_steps(3);

    assert!(!harness.state().session.modified && !harness.state().session.can_undo(), "the old resize must not change the new project");
}

#[test]
fn control_close_rejects_a_viewport_capture_before_the_next_frame() {
    let mut harness = open_project();
    let ctx = harness.ctx.clone();

    let reply = texturecraft_ui_egui::control::handle(harness.state_mut(), &ctx, "command.run", &json!({"id": "file.close"}));

    assert_eq!(reply["ok"], true);
    assert!(!harness.state().session.project_open);
    assert!(texturecraft_ui_egui::control::screenshot(harness.state(), texturecraft_ui_egui::viewport::VIEW_3D).is_none());
}

#[test]
fn control_close_dismisses_the_export_dialog() {
    let mut harness = open_project();
    harness.state_mut().ui.windows.export = true;
    let ctx = harness.ctx.clone();

    let reply = texturecraft_ui_egui::control::handle(harness.state_mut(), &ctx, "command.run", &json!({"id": "file.close"}));

    assert_eq!(reply["ok"], true);
    assert!(!harness.state().session.project_open);
    assert!(!harness.state().ui.windows.export, "Close dismisses the dialog before the next frame");
}

#[test]
fn uploaded_project_replacement_discards_the_old_resize() {
    let mut harness = open_project();
    let bytes = harness.state().session.to_project_bytes().unwrap();
    let resource = std::sync::Arc::new(vec![0; 2_097_152]);
    let old_resource = std::sync::Arc::downgrade(&resource);
    harness.state_mut().session.doc.resources.insert("lifetime-probe.bin".into(), resource);
    let old_mesh = std::sync::Arc::downgrade(&harness.state().session.mesh);
    harness.state_mut().start_resize(0, 128);

    assert!(harness.state().resize.is_some());

    let resize_progress = std::sync::Arc::downgrade(&harness.state().resize.as_ref().unwrap().progress);
    let owners_before = resize_progress.strong_count();

    texturecraft_ui_egui::web_files::load(harness.state_mut(), "replacement.tcraft", bytes);

    assert!(harness.state().session.project_open);
    assert!(harness.state().resize.is_none(), "the previous project's resize must be discarded");

    let owners_after = resize_progress.strong_count();
    eprintln!("upload resize progress owners: {owners_before} before, {owners_after} after replacement");

    assert!(owners_after < owners_before && owners_after <= 1);

    assert_eventually_released(&resize_progress);
    assert_eventually_released(&old_mesh);
    assert_eventually_released(&old_resource);
    eprintln!(
        "released old mesh and 2_097_152-byte resource after replacement: mesh={}, resource={}, progress={}",
        old_mesh.strong_count(),
        old_resource.strong_count(),
        resize_progress.strong_count()
    );

    harness.run_steps(3);

    assert!(!harness.state().session.modified && !harness.state().session.can_undo(), "the old resize must not change the uploaded project");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn file_new_discards_the_old_resize() {
    let path = std::env::temp_dir().join(format!("texturecraft-file-new-resize-{}.obj", std::process::id()));
    std::fs::write(&path, "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n").unwrap();
    let mut harness = open_project();
    harness.state_mut().start_resize(0, 128);

    assert!(harness.state().resize.is_some());

    let result = harness.state_mut().run("file.new", json!({"mesh": path.display().to_string(), "resolution": 128}));
    let _ = std::fs::remove_file(&path);

    assert!(result.is_some());
    assert!(harness.state().session.project_open);
    assert!(harness.state().resize.is_none(), "the previous project's resize must be discarded");

    harness.run_steps(3);

    assert!(!harness.state().session.modified && !harness.state().session.can_undo(), "the old resize must not change the mesh project");
}

#[test]
fn closed_session_rejects_project_control_operations() {
    for (method, params) in [("ui.bake.start", json!({"ts": 0})), ("debug.paintTest", json!({"from": [640, 400], "to": [650, 400], "frames": 2}))] {
        let mut harness = empty();
        let ctx = harness.ctx.clone();

        let reply = texturecraft_ui_egui::control::handle(harness.state_mut(), &ctx, method, &params);

        assert_eq!(reply["ok"], false, "{method}: {reply}");
        assert!(reply["error"].as_str().unwrap().contains("no project is open"));
        assert!(harness.state().bake.is_none());
        assert!(harness.state().session.bake_preview.is_none());

        harness.run_steps(3);

        assert!(!harness.state().session.modified && !harness.state().session.can_undo());
    }
}

#[test]
fn close_releases_the_layer_thumbnail_snapshot() {
    for _ in 0..2 {
        let mut harness = open_project();
        let old_mesh = std::sync::Arc::downgrade(&harness.state().session.mesh);
        let old_maps: Vec<_> = harness
            .state()
            .session
            .doc
            .baked
            .values()
            .flat_map(|maps| maps.maps.values())
            .map(|pixels| (std::sync::Arc::downgrade(pixels), std::mem::size_of_val(pixels.as_slice())))
            .collect();
        let tracked_bytes: usize = old_maps.iter().map(|(_, bytes)| bytes).sum();

        invoke(&mut harness, "file.close");

        assert!(!harness.state().session.project_open);
        assert_eventually_released(&old_mesh);
        for (pixels, _) in &old_maps {
            assert_eventually_released(pixels);
        }
        eprintln!(
            "closed thumbnail snapshot: mesh owners={}, {} baked-map allocations ({} bytes) released",
            old_mesh.strong_count(),
            old_maps.len(),
            tracked_bytes
        );
    }
}
