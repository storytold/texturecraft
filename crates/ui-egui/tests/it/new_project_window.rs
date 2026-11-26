//! New project creation, including mesh selection, disabled controls and settings passed to file.new.

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::template_cmds::DEFAULT_TEMPLATE;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::Answer;
use texturecraft_ui_egui::new_project_advanced::create_command;
use texturecraft_ui_egui::project_form::{NOT_AVAILABLE, OCIO_NOT_AVAILABLE};

use super::temp_dir::TempDir;

/// A quad whose UVs lie in UV tile 1002 and a camera-free OBJ
const TILE_1002_QUAD: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 1.1 0.1\nvt 1.9 0.1\nvt 1.9 0.9\nvt 1.1 0.9\nf 1/1 2/2 3/3 4/4\n";

/// The app with New project opened from the menu, tall enough to show the whole form
fn harness() -> Harness<'static, TextureCraftApp> {
    harness_sized(egui::vec2(1600.0, 2400.0))
}

fn harness_sized(size: egui::Vec2) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::Session::new_project("Open", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    let mut h = Harness::builder().with_size(size).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(2);
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "ui.newProject");
    h.run_steps(4);
    h
}

fn mesh_file(dir: &std::path::Path, contents: &str) -> String {
    let path = dir.join("tiles.obj");
    std::fs::write(&path, contents).unwrap();

    path.display().to_string()
}

#[test]
fn cancel_drops_the_selected_mesh_buffer() {
    let mut h = harness();
    let bytes = std::sync::Arc::new(vec![0; 1024 * 1024]);
    let buffer = std::sync::Arc::downgrade(&bytes);
    h.state_mut().ui.new_project.file = Some("cancel.obj".into());
    h.state_mut().ui.new_project.file_bytes = Some(bytes);
    h.run_steps(3);

    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run_steps(3);

    assert!(!h.state().ui.windows.new_project);
    assert!(buffer.upgrade().is_none());
}

#[test]
fn create_keeps_the_mesh_buffer_for_retry_and_drops_it_after_success() {
    let dir = TempDir::new("new-project-create-retry");
    let path = dir.join("retry.obj");
    let mut h = harness();
    let bytes = std::sync::Arc::new(vec![0; 2 * 1024 * 1024]);
    let buffer = std::sync::Arc::downgrade(&bytes);
    h.state_mut().ui.new_project.resolution = 128;
    h.state_mut().ui.new_project.file = Some(path.display().to_string());
    h.state_mut().ui.new_project.file_bytes = Some(bytes);
    h.run_steps(3);

    h.get_by_role_and_label(Role::Button, "Create").click();
    h.run_steps(3);

    assert!(h.state().ui.windows.new_project);
    assert!(buffer.upgrade().is_some());
    assert_eq!(h.state().session.doc.project.name, "Open");

    std::fs::write(&path, TILE_1002_QUAD).unwrap();
    let color_map = dir.join("retry_basecolor.png");
    image::RgbaImage::new(4, 4).save(&color_map).unwrap();
    texturecraft_ui_egui::pbr_import::begin(h.state_mut(), serde_json::json!({"files": [color_map]}));
    assert!(h.state().pbr_import.is_some());
    h.get_by_role_and_label(Role::Button, "Create").click();
    h.run_steps(3);

    assert!(!h.state().ui.windows.new_project);
    assert!(buffer.upgrade().is_none());
    assert!(h.state().pbr_import.is_none());
    assert_eq!(h.state().session.doc.project.name, "retry");
}

fn disabled(h: &Harness<'_, TextureCraftApp>, role: Role, label: &str) -> bool {
    h.get_by_role_and_label(role, label).accesskit_node().is_disabled()
}

#[test]
fn the_uv_layout_control_is_disabled_when_uv_tiles_are_off() {
    let h = harness();

    assert!(disabled(&h, Role::RadioButton, "Keep UV tiles together per material"));
}

#[test]
fn rebuild_mesh_uvs_is_off_by_default_and_toggles_with_space() {
    let mut h = harness();
    let rebuild = h.get_by_role_and_label(Role::CheckBox, "Rebuild mesh UVs");

    assert!(!rebuild.accesskit_node().is_disabled());
    assert_eq!(rebuild.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));

    rebuild.focus();
    h.run_steps(2);
    h.key_press(egui::Key::Space);
    h.run_steps(2);

    let rebuild = h.get_by_role_and_label(Role::CheckBox, "Rebuild mesh UVs");
    assert_eq!(rebuild.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert!(h.state().ui.new_project.rebuild_uvs);
}

#[test]
fn create_waits_for_a_mesh_file() {
    let mut h = harness();

    assert!(disabled(&h, Role::Button, "Create"));

    let dir = TempDir::new("new-project-create-enabled");
    h.state_mut().ui.new_project.file = Some(mesh_file(&dir, TILE_1002_QUAD));
    h.run_steps(2);

    assert!(!disabled(&h, Role::Button, "Create"));
    assert!(h.query_by_label("tiles.obj").is_some(), "the chosen file is shown");
}

#[test]
fn settings_not_available_yet_are_disabled_with_a_hint() {
    let mut h = harness();
    let unavailable = [
        (Role::CheckBox, "Compute tangent space per fragment"),
        (Role::RadioButton, "Use a separate texture set for each tile"),
        (Role::RadioButton, "Custom unit scale"),
        (Role::CheckBox, "Scale material fills in real-world units"),
    ];
    for (role, label) in unavailable {
        assert!(disabled(&h, role, label), "{label} is disabled");
    }
    assert!(!disabled(&h, Role::RadioButton, "Use the mesh file’s unit scale"));

    for (role, label) in unavailable {
        let center = h.get_by_role_and_label(role, label).rect().center();
        h.hover_at(egui::pos2(1.0, 1.0));
        h.run_steps(2);
        assert!(h.query_by_label(NOT_AVAILABLE).is_none(), "no hint before hovering {label}");
        h.hover_at(center);
        h.run_steps(30);
        assert!(h.query_by_label(NOT_AVAILABLE).is_some(), "hovering {label} says it is not available yet");
    }
}

#[test]
fn opencolorio_shows_its_settings_disabled_and_creates_legacy() {
    let mut h = harness();

    assert!(h.query_by_label("OpenColorIO configuration").is_none(), "Legacy shows only the working color space");
    assert!(h.query_by_label(OCIO_NOT_AVAILABLE).is_none(), "no OpenColorIO hint while Legacy is chosen");

    h.get_by_value("Legacy").scroll_to_me();
    h.run_steps(3);
    h.get_by_value("Legacy").click();
    h.run_steps(3);
    h.get_by_label("OpenColorIO").click();
    h.run_steps(3);

    assert!(h.query_by_label(OCIO_NOT_AVAILABLE).is_some(), "the dropdown's hover hint says OpenColorIO is not available yet");
    for label in ["OpenColorIO configuration", "Configuration file", "Standard sRGB color space", "Auto detect color spaces", "Material color space default"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    assert!(h.query_by_label("ACEScg").is_some(), "the OpenColorIO working color space");
    let ocio_config = h.get_by_value("ACES 2.0");
    assert!(ocio_config.accesskit_node().is_disabled(), "the OpenColorIO settings are disabled");
    let center = ocio_config.rect().center();
    h.hover_at(center);
    h.run_steps(30);
    assert!(h.query_by_label(NOT_AVAILABLE).is_some(), "with the not-available hint");
    let params = create_command(&texturecraft_ui_egui::state::NewProject { file: Some("a.obj".into()), ..h.state().ui.new_project.clone() }).unwrap();
    assert!(params.get("colorManagement").is_none(), "nothing about color management reaches the command");
}

#[test]
fn the_menu_opens_a_fresh_form_after_cancel() {
    let mut h = harness();
    {
        let form = &mut h.state_mut().ui.new_project;
        form.file = Some("/old/mesh.obj".into());
        form.resolution = 4096;
        form.uv_tiles = Some(true);
        form.import_cameras = false;
    }
    queue_baked_maps(&mut h);

    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run_steps(3);
    assert!(!h.state().ui.windows.new_project);
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "ui.newProject");
    h.run_steps(3);

    let form = &h.state().ui.new_project;
    assert_eq!((form.file.as_deref(), form.resolution, form.uv_tiles, form.import_cameras), (None, 1024, None, true));
    assert_eq!(form.template, DEFAULT_TEMPLATE);
    assert!(form.baked_maps.is_empty(), "canceled files stayed queued: {:?}", form.baked_maps);
    assert!(h.query_by_label("Ball_normal.png").is_none());
}

fn queue_baked_maps(h: &mut Harness<'_, TextureCraftApp>) {
    h.state_mut().file_dialogs.set_override(Some(Box::new(|_| {
        Answer::Paths(["Ball_normal.png", "Ball_ao.png", "Ball_curvature.png"].into_iter().map(std::path::PathBuf::from).collect())
    })));
    h.get_by_role_and_label(Role::Button, "Add").click();
    h.run_steps(3);

    h.get_by_label("Ball_normal.png");
    h.get_by_label("Ball_ao.png");
    h.get_by_label("Ball_curvature.png");
}

#[test]
fn closing_the_form_from_its_title_bar_discards_baked_maps() {
    let mut h = harness();
    queue_baked_maps(&mut h);

    h.get_by_role_and_label(Role::Button, "Close window").click();
    h.run_steps(3);

    assert!(!h.state().ui.windows.new_project);
    assert!(h.state().ui.new_project.baked_maps.is_empty());
}

#[test]
fn invoking_new_project_again_discards_the_open_forms_baked_maps() {
    let mut h = harness();
    queue_baked_maps(&mut h);

    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "ui.newProject");
    h.run_steps(3);

    assert!(h.state().ui.windows.new_project);
    assert!(h.query_by_label("Ball_normal.png").is_none());
    assert!(h.state().ui.new_project.baked_maps.is_empty());
}

#[test]
fn the_supported_settings_reach_the_create_command() {
    let mut form = texturecraft_ui_egui::state::NewProject::default();

    assert!(create_command(&form).is_none(), "no command without a mesh file");

    form.file = Some("/meshes/robot.fbx".into());
    let params = create_command(&form).unwrap();

    assert_eq!(params["mesh"], "/meshes/robot.fbx");
    assert_eq!(params["resolution"], 1024);
    assert_eq!(params["uvTiles"], false, "UV tiles are off unless ticked");
    assert_eq!(params["importCameras"], true);
    assert_eq!(params["template"], DEFAULT_TEMPLATE);
    assert!(params.get("name").is_none(), "the project is named after the file");

    form.uv_tiles = Some(true);
    form.import_cameras = false;
    form.resolution = 2048;
    let params = create_command(&form).unwrap();

    assert_eq!((params["uvTiles"].as_bool(), params["importCameras"].as_bool(), params["resolution"].as_u64()), (Some(true), Some(false), Some(2048)));
}

#[test]
fn ticking_uv_tiles_creates_a_uv_tile_project() {
    for (enable_uv_tiles, rebuild_uvs, expected_tiles) in [(false, false, vec![]), (true, false, vec![1002]), (true, true, vec![1001])] {
        let mut h = harness();
        let dir = TempDir::new("new-project-udim");
        h.state_mut().ui.new_project.file = Some(mesh_file(&dir, TILE_1002_QUAD));
        h.state_mut().ui.new_project.resolution = 128;
        h.run_steps(2);
        if enable_uv_tiles {
            h.get_by_role_and_label(Role::CheckBox, "Use UV Tile workflow").click();
            h.run_steps(3);
            assert!(!disabled(&h, Role::RadioButton, "Keep UV tiles together per material"));
        }

        if rebuild_uvs {
            h.get_by_role_and_label(Role::CheckBox, "Rebuild mesh UVs").click();
            h.run_steps(3);
        }
        h.get_by_role_and_label(Role::CheckBox, "Import cameras").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, "Create").click();
        h.run_steps(3);

        let app = h.state();
        assert!(!app.ui.windows.new_project, "the window closes");
        assert_eq!(app.session.doc.project.name, "tiles");
        assert_eq!(app.session.doc.project.texture_sets[0].uv_tiles, expected_tiles, "UV tiles enabled: {enable_uv_tiles}, rebuild UVs: {rebuild_uvs}");
        assert_eq!(app.session.doc.project.texture_sets[0].resolution, 128);
        if rebuild_uvs {
            let bytes = texturecraft_engine::formats::save_project_bytes(&app.session.project_file()).unwrap();
            let reopened = texturecraft_engine::Session::from_project_bytes(&bytes).unwrap();
            assert_eq!(reopened.source_mesh().uvs, app.session.source_mesh().uvs);
            assert_eq!(reopened.doc.project.format_version, texturecraft_engine::core::project::REBUILT_MESH_UVS_VERSION);
        }
    }
}

#[test]
fn create_stays_on_screen_in_a_short_window() {
    let h = harness_sized(egui::vec2(1024.0, 600.0));

    let create = h.get_by_role_and_label(Role::Button, "Create").rect();
    let cancel = h.get_by_role_and_label(Role::Button, "Cancel").rect();

    assert!(create.bottom() <= 600.0 && create.top() >= 0.0, "Create is visible without scrolling: {create:?}");
    assert!(cancel.bottom() <= 600.0, "{cancel:?}");
}

#[test]
fn picking_a_template_sets_its_resolution_and_normal_map_format() {
    let mut h = harness();
    h.state_mut().session.templates.dir = None;
    h.state_mut().run("textureSet.settings", serde_json::json!({"normalFormat": "directX"})).unwrap();
    h.state_mut().run("project.template.save", serde_json::json!({"name": "Small DirectX"})).unwrap();
    h.run_steps(2);

    h.get_by_value("Default (OpenPBR)").click();
    h.run_steps(3);
    h.get_by_label("Small DirectX (your_assets)").click();
    h.run_steps(3);

    assert!(h.query_by_value("Small DirectX (your_assets)").is_some());
    assert!(h.query_by_value("128 x 128").is_some(), "the template's resolution (the open project's 128 px)");
    assert!(h.query_by_value("DirectX").is_some(), "the template's normal map format");
    assert_eq!(h.state().ui.new_project.resolution, 128);
}

#[test]
fn create_asks_before_discarding_edits_made_after_the_form_opened() {
    let dir = TempDir::new("new-project-unsaved");
    let path = dir.join("tiles.obj");
    let mut h = harness();
    h.state_mut().ui.new_project.file = Some(path.display().to_string());
    h.state_mut().ui.new_project.resolution = 128;
    h.state_mut().run("layer.add.paint", serde_json::Value::Null).unwrap();
    h.run_steps(3);

    h.get_by_role_and_label(Role::Button, "Create").click();
    h.run_steps(3);

    assert!(h.query_by_label("Unsaved Changes").is_some());
    assert_eq!(h.state().session.doc.project.name, "Open");

    h.get_by_role_and_label(Role::Button, "Don't save").click();
    h.run_steps(3);

    assert!(h.state().ui.windows.new_project);
    assert!(h.state().session.modified, "a failed replacement keeps the old project dirty");
    assert_eq!(h.state().session.doc.project.name, "Open");

    std::fs::write(path, TILE_1002_QUAD).unwrap();
    h.get_by_role_and_label(Role::Button, "Create").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Don't save").click();
    h.run_steps(3);

    assert_eq!(h.state().session.doc.project.name, "tiles");
    assert!(!h.state().ui.windows.new_project);
}

#[test]
fn a_layout_that_closes_new_project_discards_selected_files() {
    let mut h = harness();
    queue_baked_maps(&mut h);
    let bytes = std::sync::Arc::new(vec![0; 1024 * 1024]);
    let buffer = std::sync::Arc::downgrade(&bytes);
    let ui = &mut h.state_mut().ui;
    ui.new_project.file = Some("reset.obj".into());
    ui.new_project.file_bytes = Some(bytes);

    let mut open_layout = texturecraft_ui_egui::UiState::default();
    open_layout.windows.new_project = true;
    ui.adopt_layout(open_layout);

    assert!(ui.windows.new_project);
    assert!(buffer.upgrade().is_some());
    assert!(!ui.new_project.baked_maps.is_empty());

    ui.adopt_layout(texturecraft_ui_egui::UiState::default());

    assert!(!ui.windows.new_project);
    assert!(buffer.upgrade().is_none());
    assert!(ui.new_project.baked_maps.is_empty());
}
