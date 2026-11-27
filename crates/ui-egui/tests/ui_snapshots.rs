//! Offscreen UI snapshots at 1600x960, except the taller mesh-map panel
//! Requires a GPU adapter
//!
//! Run with TC_UI_SNAPSHOTS=<dir> cargo test -p texturecraft-ui-egui --release --test ui_snapshots -- --ignored

use std::path::{Path, PathBuf};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::{BakeResolution, BakedMap, CurvatureMethod, HeightNormalization};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;
use texturecraft_ui_egui::theme::Brightness;
use texturecraft_ui_egui::titlebar::TitleBar;

#[path = "ui_snapshots/dropped_path.rs"]
mod dropped_path;

/// Click a map's row in Mesh Map Bakers, which shows its settings
fn select_map(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    harness.get_by_role_and_label(egui::Role::Button, name).click();
    harness.run_steps(3);
}

/// The lowest list showing `label`: Properties' (the Layers panel above it has a channel list showing "Base color" too)
fn lowest_list<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_all_by_role_and_label(egui::Role::ComboBox, label).max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap()
}

fn save(harness: &mut Harness<'_, TextureCraftApp>, dir: &Path, name: &str) {
    harness.run_steps(6);

    // Material orbs render off the UI thread: wait for their full pictures
    let start = std::time::Instant::now();
    while !texturecraft_ui_egui::orb_previews::OrbPreviews::settled(&harness.ctx) && start.elapsed() < std::time::Duration::from_secs(20) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        harness.run_steps(1);
    }
    harness.run_steps(2);
    let img = harness.render().unwrap_or_else(|e| panic!("{name}: {e}"));
    let path = dir.join(format!("{name}.png"));
    img.save(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

    let control_roles = [egui::Role::Button, egui::Role::CheckBox, egui::Role::ComboBox, egui::Role::Slider, egui::Role::SpinButton, egui::Role::TextInput];
    let control_tree = control_roles
        .into_iter()
        .flat_map(|role| harness.query_all_by_role(role).map(move |control| (role, control)))
        .filter_map(|(role, control)| {
            let node = control.accesskit_node();
            node.label().map(|label| format!("{role:?} {label}: disabled={}, checked={:?}", node.is_disabled(), node.toggled()))
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.join(format!("{name}.txt")), control_tree).unwrap();
    eprintln!("wrote {}", path.display());
}

#[test]
#[ignore]
fn empty_start_snapshot() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::Session::closed().unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.prefs.language = "en".into();

    save(&mut harness, &output_dir, "empty-start");
    for top in ["File", "Edit", "Mode", "Window", "Viewport", "Help"] {
        harness.get_by_label(top).click();
        harness.run_steps(4);
        save(&mut harness, &output_dir, &format!("empty-menu-{top}"));
        if top == "Window" {
            harness.get_by_label("Views").click();
            save(&mut harness, &output_dir, "empty-menu-views");
        }
        harness.key_press(egui::Key::Escape);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
    }
    harness.state_mut().ui.toolbar_tools = false;
    save(&mut harness, &output_dir, "empty-tools-off");
    for label in ["Snooze Autosave", "Resources Updater"] {
        assert!(harness.get_by_label(label).accesskit_node().is_disabled());
    }
    harness.get_by_label("Edit").click();
    harness.run_steps(3);
    harness.get_by_label("Settings").click();
    harness.run_steps(3);
    harness.get_by_label("Recent projects limit").scroll_to_me();
    harness.run_steps(3);
    save(&mut harness, &output_dir, "settings-recent-project-limit");
    assert!(!harness.get_by_label("Recent projects limit").accesskit_node().is_disabled());
}

/// Closing rejects a same-frame capture, then releases both rendered view targets.
#[test]
#[ignore]
fn close_releases_view_targets_and_rejects_immediate_capture() {
    let session = texturecraft_engine::Session::closed().unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(960.0, 640.0))
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.prefs.language = "en".into();

    for mesh in ["cube", "plane"] {
        assert!(harness.state_mut().run("project.new", serde_json::json!({"mesh": mesh, "resolution": 128})).is_some());
        harness.state_mut().ui.layout = texturecraft_ui_egui::state::ViewLayout::Split;
        harness.run_steps(4);
        harness.render().unwrap();
        let render_state = harness.state().render_state.as_ref().unwrap().clone();
        let report = |stage: &str| {
            render_state.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            if let Some(report) = render_state.device.generate_allocator_report() {
                eprintln!(
                    "{mesh} {stage}: {} live GPU allocations, {} allocated bytes, {} reserved bytes",
                    report.allocations.len(),
                    report.total_allocated_bytes,
                    report.total_reserved_bytes
                );
            } else {
                eprintln!("{mesh} {stage}: allocator report unavailable on this backend");
            }
        };
        assert!(texturecraft_ui_egui::control::screenshot(harness.state(), texturecraft_ui_egui::viewport::VIEW_3D).is_some());
        assert!(texturecraft_ui_egui::control::screenshot(harness.state(), texturecraft_ui_egui::viewport::VIEW_2D).is_some());
        report("before close");
        let ctx = harness.ctx.clone();

        let reply = texturecraft_ui_egui::control::handle(harness.state_mut(), &ctx, "command.run", &serde_json::json!({"id": "file.close"}));

        assert_eq!(reply["ok"], true);
        assert!(texturecraft_ui_egui::control::screenshot(harness.state(), texturecraft_ui_egui::viewport::VIEW_3D).is_none());
        {
            let renderer = render_state.renderer.read();
            let gpu = renderer.callback_resources.get::<texturecraft_ui_egui::gpu::GpuState>().unwrap();
            assert!(
                gpu.renderer.read_view(&render_state.device, &render_state.queue, texturecraft_ui_egui::viewport::VIEW_3D).is_some(),
                "the stale target still exists before the next UI frame"
            );
        }

        harness.run_steps(2);
        harness.render().unwrap();

        {
            let renderer = render_state.renderer.read();
            let gpu = renderer.callback_resources.get::<texturecraft_ui_egui::gpu::GpuState>().unwrap();
            for view in [texturecraft_ui_egui::viewport::VIEW_3D, texturecraft_ui_egui::viewport::VIEW_2D] {
                assert!(gpu.renderer.read_view(&render_state.device, &render_state.queue, view).is_none());
            }
        }
        report("after close");
    }
}

/// Check the saved-brush controls without a GPU adapter
#[test]
#[ignore]
fn brush_presets_popover_snapshot() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(128, 32).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    let brush = harness.state().session.tool.brush.clone();
    harness.state_mut().session.brush_presets.list.clear();
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().session.brush_presets.list.push(texturecraft_engine::brush_preset_cmds::UserBrush {
        name: "Snapshot brush".into(),
        brush,
        channels: None,
        file: None,
    });
    assert_eq!(harness.state().session.brush_presets.list.len(), 1);
    harness.run_steps(6);
    harness.get_by_label_contains("Brush presets").click();
    save(&mut harness, &output_dir, "brush-presets-popover-with-user");

    harness.get_by_label("Save current brush").scroll_to_me();
    save(&mut harness, &output_dir, "brush-presets-controls-with-user");
    assert!(harness.query_by_label_contains("Snapshot brush").is_some());
}

#[test]
#[ignore]
fn pbr_import_dialog_keeps_choices_after_error() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let source_dir = std::env::temp_dir().join(format!("tc_pbr_snapshot_{}", std::process::id()));
    std::fs::create_dir_all(&source_dir).unwrap();
    let bytes = texturecraft_engine::formats::gray16_png(2, 2, [0.5; 4].into_iter()).unwrap();
    for name in ["tile_albedo.png", "tile_normal_gl.png", "tile_normal_dx.png"] {
        std::fs::write(source_dir.join(name), &bytes).unwrap();
    }
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.prefs.language = "en".into();
    texturecraft_ui_egui::pbr_import::begin(harness.state_mut(), serde_json::json!({"path": source_dir.display().to_string()}));

    harness.run_steps(3);
    harness.get_all_by_role(egui::Role::ComboBox).find(|node| node.value().as_deref() == Some("tile_normal_gl.png")).unwrap().click();
    harness.run_steps(2);
    harness.get_by_label("tile_normal_dx.png").click();
    harness.run_steps(2);

    assert!(harness.get_all_by_role(egui::Role::ComboBox).any(|node| node.value().as_deref() == Some("Automatic (DirectX)")));
    save(&mut harness, &output_dir, "pbr-import-dialog");
    std::fs::remove_dir_all(&source_dir).unwrap();
    harness.get_by_role_and_label(egui::Role::Button, "Import").click();
    harness.run_steps(2);

    assert!(harness.state().pbr_import.is_some());
    assert!(harness.get_all_by_role(egui::Role::ComboBox).any(|node| node.value().as_deref() == Some("tile_normal_dx.png")));
    assert!(harness.get_all_by_role(egui::Role::ComboBox).any(|node| node.value().as_deref() == Some("Automatic (DirectX)")));
    save(&mut harness, &output_dir, "pbr-import-error");
}

#[test]
#[ignore]
fn pbr_import_uses_displayed_default_after_folder_changes() {
    let source_dir = std::env::temp_dir().join(format!("tc_pbr_pin_{}", std::process::id()));
    std::fs::create_dir_all(&source_dir).unwrap();
    let bytes = texturecraft_engine::formats::gray16_png(2, 2, [0.5; 4].into_iter()).unwrap();
    std::fs::write(source_dir.join("tile_albedo_4k.png"), &bytes).unwrap();

    let session = texturecraft_engine::sample_session(128, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    texturecraft_ui_egui::pbr_import::begin(harness.state_mut(), serde_json::json!({"path": source_dir.display().to_string()}));
    std::fs::write(source_dir.join("tile_albedo_8k.png"), &bytes).unwrap();

    harness.run_steps(3);
    harness.get_by_role_and_label(egui::Role::Button, "Import").click();
    harness.run_steps(2);

    let resources = &harness.state().session.doc.resources;
    assert!(resources.keys().any(|name| name.contains("tile_albedo_4k")), "{resources:?}");
    assert!(!resources.keys().any(|name| name.contains("tile_albedo_8k")), "{resources:?}");
    std::fs::remove_dir_all(source_dir).unwrap();
}

#[test]
#[ignore]
fn pbr_drop_candidates_and_project_replacement() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let source_dir = output_dir.join("pbr-drop-candidates");
    std::fs::create_dir_all(&source_dir).unwrap();
    let files: Vec<_> = ["m_albedo_1k.png", "m_albedo_4k.png", "reference.png"].iter().map(|name| source_dir.join(name)).collect();
    for file in &files {
        std::fs::write(file, b"fixture").unwrap();
    }
    let reference = texturecraft_engine::formats::gray16_png(2, 2, [0.5; 4].into_iter()).unwrap();
    std::fs::write(&files[2], reference).unwrap();

    let session = texturecraft_engine::sample_session(128, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.input_mut().dropped_files = files.iter().cloned().map(dropped_path::handle).collect();
    harness.run_steps(1);
    assert!(harness.state().pbr_import.is_some());
    assert!(harness.state().session.doc.resources.keys().any(|name| name.contains("reference.png")));
    assert!(!harness.state().session.doc.resources.keys().any(|name| name.contains("m_albedo")));
    harness.run_steps(3);
    assert!(harness.query_by_label("2 files ignored").is_none());
    assert!(harness.query_by_label("1 files ignored").is_some());
    assert!(harness.get_all_by_role(egui::Role::ComboBox).any(|node| node.value().as_deref() == Some("m_albedo_4k.png")));
    save(&mut harness, &output_dir, "pbr-drop-candidates");

    assert!(harness.state_mut().run("project.new", serde_json::json!({"mesh": "not-a-mesh"})).is_none());
    assert!(harness.state().pbr_import.is_some());
    assert!(harness.state_mut().run("project.new", serde_json::json!({"resolution": 128})).is_some());
    assert!(harness.state().pbr_import.is_none());
    assert!(texturecraft_ui_egui::pbr_import::dropped(harness.state_mut(), &files[1..]).is_empty());
    std::fs::remove_dir_all(source_dir).unwrap();
}

#[test]
#[ignore]
fn pbr_dialog_is_cleared_when_project_changes() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let source_dir = output_dir.join("pbr-replacement-source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("m_albedo.png"), b"fixture").unwrap();
    let session = texturecraft_engine::sample_session(128, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    texturecraft_ui_egui::pbr_import::begin(harness.state_mut(), serde_json::json!({"path": source_dir.display().to_string()}));
    assert!(harness.state().pbr_import.is_some());

    assert!(harness.state_mut().run("project.new", serde_json::json!({"resolution": 128})).is_some());
    assert!(harness.state().pbr_import.is_none());
    std::fs::remove_dir_all(source_dir).unwrap();
}

/// Capture the moving preview and settled render after submitting real GPU callbacks.
#[test]
#[ignore]
fn rendering_mode_snapshots() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Rendering;
    harness.state_mut().ui.render.max_samples = 16;
    eprintln!("Rendering-mode adapter: {}", harness.state().gpu.as_ref().unwrap().adapter);

    // Submit each frame so the snapshots exercise the tracer callback, including size switches
    for _ in 0..6 {
        harness.state_mut().session.camera.yaw += 0.08;
        harness.run_steps(1);
        harness.render().unwrap();
    }
    let moving_path = output_dir.join("rendering-moving.png");
    harness.render().unwrap().save(&moving_path).unwrap();
    eprintln!("wrote {}", moving_path.display());

    for _ in 0..45 {
        harness.run_steps(1);
        harness.render().unwrap();
    }
    save(&mut harness, &output_dir, "rendering-settled");
}

#[test]
#[ignore]
fn symmetry_snapshots() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(1024, 256).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().run("tool.brush", serde_json::json!({"symmetryY": true, "symmetryOffset": [0, 0.1, 0]})).unwrap();
    harness.state_mut().ui.symmetry.show_plane = true;
    harness.state_mut().ui.symmetry.show_manipulator = true;
    harness.run_steps(4);
    harness.get_by_label("Symmetry settings").click();

    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("symmetry-mirror-{name}"));
    }

    harness.state_mut().run("tool.brush", serde_json::json!({"symmetryY": false, "symmetryRadial": 6, "symmetryAxis": "y", "symmetryAngleSpan": 240})).unwrap();
    harness.state_mut().ui.symmetry.show_axis = true;
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    save(&mut harness, &output_dir, "symmetry-radial");

    harness.state_mut().run("tool.brush", serde_json::json!({"symmetryX": true, "symmetryY": true})).unwrap();
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("symmetry-combined-{name}"));
    }
}

#[test]
#[ignore]
fn window_snapshots() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else {
        eprintln!("set TC_UI_SNAPSHOTS=<dir> to write the pictures");
        return;
    };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(1024, 256).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    for (brightness, name) in [(Brightness::MediumDark, "main"), (Brightness::Dark, "brightness-dark"), (Brightness::Light, "brightness-light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, name);
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.state_mut().title_bar = TitleBar::Integrated;
    save(&mut harness, &output_dir, "titlebar-macos");
    harness.state_mut().title_bar = TitleBar::App(craft_window::Layout::default());
    save(&mut harness, &output_dir, "titlebar-app");

    // Check hover feedback and fractional scaling
    harness.hover_at(egui::pos2(1600.0 - 23.0, 14.0));
    save(&mut harness, &output_dir, "titlebar-app-close-hover");
    harness.hover_at(egui::pos2(800.0, 500.0));
    harness.set_pixels_per_point(1.5);
    save(&mut harness, &output_dir, "titlebar-app-150");
    harness.set_pixels_per_point(1.0);
    harness.state_mut().title_bar = TitleBar::System;

    for texture_set in &mut harness.state_mut().session.doc.project.texture_sets {
        texture_set.bake = Default::default();
    }
    harness.state_mut().ui.mode = Mode::Baking;
    save(&mut harness, &output_dir, "mode-bake");
    select_map(&mut harness, "Ambient Occlusion");
    save(&mut harness, &output_dir, "mode-bake-maps");
    harness.state_mut().ui.prefs.brightness = Brightness::Light;
    save(&mut harness, &output_dir, "mode-bake-maps-light");
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    select_map(&mut harness, "Opacity");
    save(&mut harness, &output_dir, "mode-bake-unchecked");
    harness.state_mut().ui.dock.focus(Panel::TextureSetList);
    save(&mut harness, &output_dir, "bake-texture-sets");
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);
    save(&mut harness, &output_dir, "bake-settings");
    harness.state_mut().session.add_high_poly_bytes("High Poly.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    save(&mut harness, &output_dir, "bake-settings-high-poly");
    harness.get_by_role_and_label(egui::Role::ComboBox, "Cage Type").scroll_to_me();
    save(&mut harness, &output_dir, "bake-settings-cage-skew");
    harness.get_by_label("Edge Contrast").scroll_to_me();
    save(&mut harness, &output_dir, "bake-settings-skew");
    let output_width = harness
        .get_all_by_role_and_label(egui::Role::ComboBox, "Output Size")
        .find(|choice| !choice.accesskit_node().is_disabled())
        .expect("the linked output size has an enabled width control");
    output_width.scroll_to_me();
    harness.state_mut().session.doc.project.texture_sets[0].bake.common.resolution_height = Some(BakeResolution::R1024);
    save(&mut harness, &output_dir, "bake-settings-unlocked");
    harness.set_pixels_per_point(1.5);
    save(&mut harness, &output_dir, "bake-settings-150");
    harness.set_pixels_per_point(1.0);
    {
        let settings = &mut harness.state_mut().session.doc.project.texture_sets[0].bake;
        settings.maps.select([BakedMap::Curvature, BakedMap::Height]);
        settings.maps.curvature.parameters.method = CurvatureMethod::Mesh;
        settings.maps.height.parameters.normalization = HeightNormalization::Manual;
        settings.maps.height.parameters.scaling_divisor = 100.0;
    }
    select_map(&mut harness, "Curvature");
    save(&mut harness, &output_dir, "bake-curvature-mesh");
    select_map(&mut harness, "Height");
    save(&mut harness, &output_dir, "bake-height-manual");
    harness.state_mut().session.doc.bake_meshes.high.clear();
    harness.state_mut().ui.mode = Mode::Painting;
    save(&mut harness, &output_dir, "painting-restored");

    harness.state_mut().ui.dock.focus(Panel::Display);
    save(&mut harness, &output_dir, "panel-display");
    harness.state_mut().ui.dock.close(Panel::Display);
    harness.state_mut().ui.windows.export = true;
    save(&mut harness, &output_dir, "window-export");
    harness.state_mut().ui.windows.export = false;
    harness.state_mut().ui.windows.settings = true;
    save(&mut harness, &output_dir, "window-settings");
    harness.state_mut().ui.windows.settings = false;

    let context = harness.ctx.clone();
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "ui.projectConfig");
    save(&mut harness, &output_dir, "window-project-configuration");
    harness.state_mut().ui.windows.project_config = None;

    // A taller window exposes every mesh-map slot
    harness.state_mut().ui.dock.focus(Panel::TextureSetSettings);
    harness.set_size(egui::vec2(1600.0, 1800.0));
    save(&mut harness, &output_dir, "panel-texture-set-settings");
    harness.set_size(egui::vec2(1600.0, 960.0));

    // Show the mask histogram in Properties
    let session = &mut harness.state_mut().session;
    for (id, params) in [
        ("layer.add.fill", serde_json::json!({"color": [0.8, 0.3, 0.1]})),
        ("mask.add.black", serde_json::Value::Null),
        ("effect.add.fill", serde_json::json!({"target": "mask", "procedural": "clouds"})),
        ("effect.add.levels", serde_json::json!({"target": "mask", "inLow": 0.3, "inHigh": 0.8})),
    ] {
        texturecraft_engine::execute(session, id, &params).unwrap();
    }
    save(&mut harness, &output_dir, "properties-levels");

    // Show the Layers panel thumbnails with a hidden content effect, a folder and a mask
    let session = &mut harness.state_mut().session;
    for (id, params) in [
        ("effect.add.filter", serde_json::json!({"target": "content", "filter": "hsl"})),
        ("effect.add.filter", serde_json::json!({"target": "content", "filter": "invert"})),
        ("layer.add.paint", serde_json::Value::Null),
        ("layer.group", serde_json::Value::Null),
    ] {
        texturecraft_engine::execute(session, id, &params).unwrap();
    }
    let hidden = harness
        .state()
        .session
        .doc
        .project
        .layers
        .values()
        .flat_map(|layer| layer.content_effects.iter())
        .find(|effect| effect.name == "Invert")
        .map(|effect| effect.id.0);
    if let Some(effect) = hidden {
        texturecraft_engine::execute(&mut harness.state_mut().session, "effect.set", &serde_json::json!({"effect": effect, "visible": false})).unwrap();
    }
    harness.state_mut().ui.dock.focus(Panel::Layers);
    harness.set_size(egui::vec2(1600.0, 1400.0));
    for _ in 0..400 {
        harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
        if !harness.state().layer_panel.service().busy() {
            break;
        }
    }
    save(&mut harness, &output_dir, "panel-layers");
    harness.set_size(egui::vec2(1600.0, 960.0));

    // Show the second path selected in Properties
    let session = &mut harness.state_mut().session;
    for (id, params) in [
        ("layer.add.paint", serde_json::Value::Null),
        ("tool.path", serde_json::json!({"uvPoints": [[0.2, 0.5], [0.5, 0.4], [0.8, 0.5]], "commit": true})),
        ("tool.path", serde_json::json!({"uvPoints": [[0.3, 0.7], [0.7, 0.7], [0.7, 0.9]], "closed": true, "fill": true, "commit": true})),
    ] {
        texturecraft_engine::execute(session, id, &params).unwrap();
    }

    let effect = match session.target {
        texturecraft_engine::StackTarget::Effect(e) => e.0,
        _ => 0,
    };
    texturecraft_engine::execute(session, "path.edit", &serde_json::json!({"effect": effect, "index": 1})).unwrap();
    save(&mut harness, &output_dir, "properties-paths");

    // Show the brush cursor and readout during Ctrl+right drag
    let cursor_position = harness.state().viewport_rect.center();
    let (modifiers, button) = (egui::Modifiers::COMMAND, egui::PointerButton::Secondary);
    harness.event(egui::Event::ModifiersChanged(modifiers));
    harness.event(egui::Event::PointerButton { pos: cursor_position, button, pressed: true, modifiers });
    for step in 1..=4 {
        harness.hover_at(cursor_position + egui::vec2(15.0 * step as f32, -8.0 * step as f32));
        harness.run_steps(1);
    }
    save(&mut harness, &output_dir, "brush-ctrl-drag");
    harness.event(egui::Event::PointerButton { pos: cursor_position, button, pressed: false, modifiers });
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
}

#[test]
#[ignore]
fn bake_uv_tiles_snapshot() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else {
        eprintln!("set TC_UI_SNAPSHOTS=<dir> to write the pictures");
        return;
    };

    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session =
        texturecraft_engine::Session::new_project("UV tile baking", texturecraft_engine::geom::primitives::udim_plane(), "builtin:udim_plane", None, 256)
            .unwrap();
    texturecraft_engine::execute(&mut session, "textureSet.tile.resolution", &serde_json::json!({"tile": 1002, "resolution": 128})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    save(&mut harness, &output_dir, "bake-uv-tiles");

    harness.get_by_value("128x128*").click();
    save(&mut harness, &output_dir, "bake-tile-resolution-menu");
    harness.get_by_label("Remove resolution override").click();
    save(&mut harness, &output_dir, "bake-tile-resolution-inherited");
    assert!(harness.query_by_value("128x128*").is_none());
    assert!(harness.state().session.doc.project.texture_sets[0].tile_resolutions.is_empty());
    let inherited_resolution = harness.get_by_label("UV tile 1002 resolution").rect().center();
    harness.hover_at(inherited_resolution);
    save(&mut harness, &output_dir, "bake-tile-resolution-hover");
    assert_eq!(harness.get_all_by_value("256x256").count(), 2);
    harness.hover_at(egui::pos2(900.0, 500.0));
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(3);

    harness.get_by_role_and_label(egui::Role::CheckBox, "1002").click();
    save(&mut harness, &output_dir, "bake-uv-tiles-partial");
    assert!(harness.state().session.doc.project.texture_sets[0].bake_excluded_tiles.contains(&1002));

    harness.get_by_label("Show or hide the texture set").click();
    save(&mut harness, &output_dir, "bake-uv-tiles-hidden");
    assert!(!harness.state().session.doc.project.texture_sets[0].visible);
    assert!(harness.state().session.doc.project.texture_sets[0].bake.common.is_enabled);
}

#[test]
#[ignore]
fn smart_mask_shelf_snapshot() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else {
        eprintln!("set TC_UI_SNAPSHOTS=<dir> to write the picture");
        return;
    };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session =
        texturecraft_engine::Session::new_project("Mask previews", texturecraft_engine::geom::primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.shelf_tab = 2;
    let context = harness.ctx.clone();
    let started = std::time::Instant::now();
    loop {
        harness.run_steps(1);
        let thumbnails = &mut harness.state_mut().thumbs;
        if texturecraft_engine::smart_masks::SMART_MASKS.iter().all(|preset| thumbnails.smart_mask(&context, preset.id).is_some()) {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "smart mask thumbnails did not arrive");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    eprintln!("all smart mask thumbnails ready after {:.3} s", started.elapsed().as_secs_f64());
    save(&mut harness, &output_dir, "smart-mask-shelf");

    let mask = &texturecraft_engine::smart_masks::SMART_MASKS[0];
    harness.get_by_role_and_label(egui::Role::Button, mask.label).hover();
    let preview = texturecraft_ui_egui::shelf_preview::ShelfPreview::SmartMask(mask.id);
    let started = std::time::Instant::now();
    loop {
        harness.run_steps(1);
        if harness.state_mut().thumbs.rendered_big(&context, &preview, 256).is_some() {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "smart mask hover preview did not arrive");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    harness.run_steps(60);
    assert_eq!(harness.query_all_by_label(mask.label).count(), 2, "the popup shows the mask name");
    save(&mut harness, &output_dir, "smart-mask-hover");
}

#[test]
#[ignore]
fn smart_material_shelf_snapshot() {
    use texturecraft_engine::materials::MATERIALS;
    use texturecraft_ui_egui::shelf_preview::ShelfPreview;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else {
        eprintln!("set TC_UI_SNAPSHOTS=<dir> to write the pictures");
        return;
    };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session =
        texturecraft_engine::Session::new_project("Smart material previews", texturecraft_engine::geom::primitives::cube(2.0), "builtin:cube", None, 128)
            .unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| {
            let render_state = context.wgpu_render_state.as_ref().expect("the snapshot uses wgpu");
            eprintln!("snapshot adapter: {:?}", render_state.adapter.get_info());
            TextureCraftApp::new(context, session)
        });
    harness.state_mut().ui.shelf_tab = 1;
    let context = harness.ctx.clone();
    let started = std::time::Instant::now();
    loop {
        harness.run_steps(1);
        let thumbnails = &mut harness.state_mut().thumbs;
        if MATERIALS.iter().filter(|material| material.smart).all(|material| thumbnails.material(&context, material.id).is_some()) {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(120), "smart material thumbnails did not arrive");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    eprintln!("all smart material thumbnails ready after {:.3} s", started.elapsed().as_secs_f64());
    save(&mut harness, &output_dir, "smart-material-shelf");

    let material = MATERIALS.iter().find(|material| material.id == "steel_painted_worn").unwrap();
    harness.get_by_role_and_label(egui::Role::Button, material.label).hover();
    let preview = ShelfPreview::material(material.id);
    let started = std::time::Instant::now();
    loop {
        harness.run_steps(1);
        if harness.state_mut().thumbs.rendered_big(&context, &preview, 256).is_some() {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "smart material hover preview did not arrive");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    harness.run_steps(60);
    assert_eq!(harness.query_all_by_label(material.label).count(), 2, "the popup shows the material name");
    save(&mut harness, &output_dir, "smart-material-hover");
}

/// Compact nested rows must keep their content, mask and effect controls in the panel.
#[test]
#[ignore]
fn nested_layers_snapshot() {
    use craft_behavior::dock::Node;
    use craft_behavior::dock::SplitSize;
    use serde_json::{Value, json};
    use texturecraft_engine::execute;
    use texturecraft_ui_egui::docking::Dock;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "layer.add.fill", &json!({"color": "#336699"})).unwrap();
    execute(&mut session, "mask.add.white", &Value::Null).unwrap();
    execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert"})).unwrap();
    for _ in 0..6 {
        execute(&mut session, "layer.group", &Value::Null).unwrap();
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1600.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    let workspace = harness.state().ui.dock.workspace().clone();
    let mut layout = workspace.layout().clone();
    let Some(Node::Split { second, .. }) = layout.root.as_mut() else { panic!("default columns") };
    let Node::Split { size, .. } = second.as_mut() else { panic!("viewport and right column") };
    *size = SplitSize::Ratio(0.99);
    let workspace = texturecraft_ui_egui::docking::Workspace::from_parts(layout, workspace.hidden().to_vec(), workspace.floated().to_vec());
    harness.state_mut().ui.dock = Dock::from_value(serde_json::to_value(workspace).unwrap());
    for _ in 0..100 {
        harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
        if !harness.state().layer_panel.service().busy() {
            break;
        }
    }
    save(&mut harness, &output_dir, "panel-layers-narrow-nested");
}

/// The mask anchor and generator micro controls in the integrated properties panel
#[test]
#[ignore]
fn mask_anchor_micro_detail_snapshot() {
    use serde_json::{Value, json};
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let lower = execute(&mut session, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({"layer": lower})).unwrap();
    execute(&mut session, "effect.add.fill", &json!({"layer": lower, "value": 0.4})).unwrap();
    let anchor = execute(&mut session, "effect.add.anchor", &json!({"layer": lower, "target": "mask", "name": "Wear"})).unwrap()["effect"].as_u64().unwrap();
    let upper = execute(&mut session, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap();
    let generator = execute(&mut session, "effect.add.generator", &json!({"layer": upper, "generator": "dirt", "microAnchor": anchor})).unwrap()["effect"]
        .as_u64()
        .unwrap();
    execute(&mut session, "layer.select", &json!({"layer": upper, "target": generator})).unwrap();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.dock.focus(Panel::Properties);
    harness.run_steps(6);
    harness.get_by_role_and_label(egui::Role::Button, "Micro Details").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::Role::Button, "Micro Details").click();
    harness.run_steps(3);

    let height = harness.get_by_role_and_label(egui::Role::ComboBox, "Micro Height");
    assert!(height.accesskit_node().value().is_some_and(|value| value.contains("Wear")));
    height.scroll_to_me();
    save(&mut harness, &output_dir, "mask-anchor-micro-detail");
}

#[test]
#[ignore]
fn path_tool_snapshots() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    texturecraft_engine::execute(&mut session, "layer.add.paint", &serde_json::Value::Null).unwrap();
    texturecraft_engine::execute(&mut session, "tool.path", &serde_json::json!({"uvPoints": [[0.25, 0.5], [0.5, 0.3], [0.75, 0.5]]})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.layout = texturecraft_ui_egui::state::ViewLayout::Only2D;
    harness.state_mut().session.select_path_point(1);
    harness.hover_at(egui::pos2(800.0, 600.0));
    save(&mut harness, &output_dir, "path-preview-2d");

    harness.state_mut().session.commit_path(texturecraft_engine::geom::glam::Vec2::splat(400.0)).unwrap();
    save(&mut harness, &output_dir, "path-stored-2d");
}

#[test]
#[ignore]
fn rail_window_snapshots() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else {
        eprintln!("set TC_UI_SNAPSHOTS=<dir> to write the pictures");
        return;
    };

    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.dock.close(Panel::TextureSetSettings);
    let context = harness.ctx.clone();

    for (command, name) in [
        ("ui.view.textureSetSettings", "rail-texture-set-settings"),
        ("ui.view.display", "rail-display"),
        ("ui.view.shader", "rail-shader"),
        ("ui.view.history", "rail-history"),
        ("ui.view.log", "rail-log"),
    ] {
        texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, command);
        save(&mut harness, &output_dir, name);
    }

    harness.set_size(egui::vec2(1600.0, 600.0));
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "ui.view.display");
    harness.run_steps(6);
    harness.get_by_label("Temporal anti-aliasing (refines the 3D view while it is still)").scroll_to_me();
    save(&mut harness, &output_dir, "rail-display-short");

    harness.set_size(egui::vec2(1600.0, 400.0));
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "ui.view.history");
    save(&mut harness, &output_dir, "rail-history-short");
}

/// Visual check of the migrated catalogs and the live language selector.
#[test]
#[ignore]
fn i18n_snapshots() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    for code in ["en", "es", "ar"] {
        assert!(texturecraft_ui_egui::i18n::available(code), "build with migrated catalogs for {code}");
        let session = texturecraft_engine::sample_session(128, 32).unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1600.0, 960.0))
            .with_pixels_per_point(1.0)
            .wgpu()
            .with_context_setup(texturecraft_ui_egui::framework::install_headless)
            .build_eframe(|cc| TextureCraftApp::new(cc, session));
        harness.state_mut().ui.prefs.language = code.into();
        harness.state_mut().ui.windows.settings = true;
        save(&mut harness, &output_dir, &format!("i18n-{code}"));
    }
    texturecraft_ui_egui::i18n::set_language("en");
}

/// The merged effect panel selects an image and exposes its projection controls
#[test]
#[ignore]
fn mask_fill_image_snapshot() {
    use serde_json::json;
    use texturecraft_engine::{
        core::{EffectKind, LayerId, Source},
        execute,
    };

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png_bytes = Vec::new();
    image::RgbaImage::from_fn(16, 16, |x, y| {
        let value = if (x / 4 + y / 4) % 2 == 0 { 255 } else { 32 };
        image::Rgba([value, value, value, 255])
    })
    .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
    .unwrap();
    let name = session.import_resource_bytes("Mask checker.png", png_bytes).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.2, 0.1]})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({})).unwrap();
    let effect = execute(&mut session, "effect.add.fill", &json!({"target": "mask", "value": 1.0})).unwrap()["effect"].as_u64().unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer, "target": effect})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1100.0))
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);

    // The Grayscale slot opens the source picker; its search finds the image
    harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").click();
    harness.run_steps(3);
    harness.event(egui::Event::Text("checker".into()));
    harness.run_steps(3);
    save(&mut harness, &output_dir, "mask-fill-image-menu");

    harness.get_by_role_and_label(egui::Role::ListBoxOption, &name).click();
    harness.run_steps(4);

    let EffectKind::Fill { source, .. } = &harness.state().session.doc.project.effect(LayerId(effect)).unwrap().1.kind else { panic!("fill effect") };

    assert_eq!(*source, Source::image(name));
    save(&mut harness, &output_dir, "mask-fill-image-selected");

    harness.get_by_role_and_label(egui::Role::ComboBox, "UV projection").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Tri-planar projection").click();
    harness.run_steps(3);

    let edited = &harness.state().session.doc.project.effect(LayerId(effect)).unwrap().1.kind;

    assert!(matches!(edited, EffectKind::Fill { projection: texturecraft_engine::core::Projection::Triplanar { .. }, .. }));
}

/// Every docked panel's header bar (names in capitals, a close × at the right), then the Properties × hovered
#[test]
#[ignore]
fn panel_header_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("panel-header-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    let close = harness
        .get_all_by_label("Close Panel")
        .map(|node| node.rect())
        .filter(|rect| rect.min.x > 1200.0)
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .expect("the Properties header's close button");
    harness.hover_at(close.center());
    save(&mut harness, &output_dir, "panel-header-close-hovered");
}

/// Edit > Settings: the three pages and a compact window at 150% scale
#[test]
#[ignore]
fn settings_pages_snapshot() {
    use texturecraft_ui_egui::settings_window::SettingsPage;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(128, 32).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.windows.settings = true;
    save(&mut harness, &output_dir, "settings-general");

    // The rest of the General page, a page height further down each time
    let page = harness.get_by_label("INTERFACE OPTIONS").rect().center() + egui::vec2(150.0, 100.0);
    for part in 2..=4 {
        harness.hover_at(page);
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -480.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        });
        save(&mut harness, &output_dir, &format!("settings-general-{part}"));
    }
    for (page, name) in [(SettingsPage::Shortcuts, "settings-shortcuts"), (SettingsPage::Libraries, "settings-libraries")] {
        if let Some(draft) = harness.state_mut().ui.windows.settings_draft.as_mut() {
            draft.page = page;
        }
        save(&mut harness, &output_dir, name);
    }

    harness.set_size(egui::vec2(960.0, 600.0));
    harness.state_mut().ui.prefs.ui_scale = 1.5;
    harness.state_mut().ui.windows.settings_draft.as_mut().unwrap().page = SettingsPage::General;
    save(&mut harness, &output_dir, "settings-compact");
}

#[test]
#[ignore]
fn default_layout_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for color in [[0.2, 0.4, 0.7], [0.5, 0.5, 0.5], [0.8, 0.3, 0.1]] {
        execute(&mut session, "layer.add.fill", &json!({ "color": color })).unwrap();
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("default-layout-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    let properties_close = harness
        .get_all_by_label("Close Panel")
        .map(|node| node.rect())
        .filter(|rect| rect.min.x > 1200.0)
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .expect("the Properties header's close button");
    harness.hover_at(egui::pos2(properties_close.center().x - 120.0, properties_close.min.y - 6.0));
    save(&mut harness, &output_dir, "default-layout-split-hovered");
}

#[test]
#[ignore]
fn fill_reset_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "tiling": [2.0, 2.0], "filtering": "nearest"})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("fill-reset-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    let reset = harness.get_by_label("Fill: Reset to defaults").rect().center();
    harness.hover_at(reset);
    save(&mut harness, &output_dir, "fill-reset-hovered");

    // The band sits under the fold at this height: scroll Properties down to it
    harness.hover_at(reset);
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -110.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    harness.run_steps(3);
    let edges = harness.get_by_label("Restore defaults").rect().center();
    harness.hover_at(edges);
    save(&mut harness, &output_dir, "fill-reset-band-hovered");
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 110.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    harness.run_steps(3);
    harness.get_by_label("Fill: Reset to defaults").click();
    harness.hover_at(reset);
    save(&mut harness, &output_dir, "fill-reset-at-defaults");
}

#[test]
#[ignore]
fn fill_properties_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    execute(
        &mut session,
        "layer.fill.projection",
        &json!({"layer": layer, "tiling": [2.0, 2.0], "rotation": 30.0, "offset": [0.25, 0.0], "wrap": "repeat", "filtering": "bilinearHq"}),
    )
    .unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1500.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("fill-uv-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;

    // Check the fixed ring and moving dot both inside and beyond the preview
    execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "offset": [0.0, 0.0]})).unwrap();
    harness.run_steps(2);
    let pad = harness.get_by_label("Move on model").rect().center();
    harness.event(egui::Event::PointerMoved(pad));
    harness.event(egui::Event::PointerButton { pos: pad, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.run_steps(2);
    for step in 1..=20 {
        harness.event(egui::Event::PointerMoved(pad + egui::vec2(6.0 * step as f32, 0.0)));
        harness.run_steps(1);
    }
    save(&mut harness, &output_dir, "fill-uv-pad-dragged-right");
    harness.event(egui::Event::PointerButton { pos: pad, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run_steps(2);
    execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "offset": [-1.6, 0.6]})).unwrap();
    save(&mut harness, &output_dir, "fill-uv-dot-past-edge");
    execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "offset": [0.25, 0.0]})).unwrap();
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    for (shown, hover, name) in [
        ("UV projection", "Planar projection", "fill-uv-projection-open"),
        ("Bilinear | HQ", "Nearest", "fill-uv-filtering-open"),
        ("Repeat", "Repeat horizontally", "fill-uv-wrap-open"),
    ] {
        harness.get_by_label(shown).click();
        harness.run_steps(2);
        harness.get_by_label(hover).hover();
        save(&mut harness, &output_dir, name);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
    }
    harness.get_by_label("Physical size").click();
    save(&mut harness, &output_dir, "fill-uv-physical");
    harness.get_by_label("Link").click();
    save(&mut harness, &output_dir, "fill-uv-physical-unlinked");

    // Screen 02: every other projection, with the lists that open from its new rows
    let cases = [
        ("spherical", json!({}), "fill-spherical"),
        ("cylindrical", json!({"scaleMode": "physicalSize"}), "fill-cylindrical"),
        ("cylindrical", json!({"angle": 120.0, "backfaceCulling": true}), "fill-cylindrical-angle"),
        ("triplanar", json!({"scaleMode": "physicalSize"}), "fill-triplanar"),
        ("planar", json!({"scaleMode": "physicalSize", "backfaceCulling": true, "shapeCrop": "cropped"}), "fill-planar"),
        ("warp", json!({"scaleMode": "physicalSize", "warpCorners": [[-0.55, 0.48], [0.56, 0.6], [0.48, -0.55], [-0.45, -0.45]]}), "fill-warp"),
        // The image's Rotation turns the checker inside the frame
        ("warp", json!({"scaleMode": "tiling", "tiling": [2.0, 2.0], "rotation": 30.0}), "fill-warp-rotated"),
        ("uvTile", json!({}), "fill-uvtile"),
    ];
    for (kind, settings, name) in cases {
        let session = &mut harness.state_mut().session;
        execute(session, "layer.fill.projection", &json!({"layer": layer, "projection": kind})).unwrap();
        let mut settings = settings;
        settings["layer"] = layer.clone();
        execute(session, "layer.fill.projection", &settings).unwrap();
        save(&mut harness, &output_dir, name);
    }

    // The preview square is Move on model's pad: lit on hover and while held
    execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "projection": "planar"})).unwrap();
    harness.run_steps(2);
    harness.get_by_label("Move on model").hover();
    save(&mut harness, &output_dir, "fill-planar-pad-hover");
    let pad = harness.get_by_label("Move on model").rect().center();
    harness.event(egui::Event::PointerButton { pos: pad, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    save(&mut harness, &output_dir, "fill-planar-pad-held");
    harness.event(egui::Event::PointerButton { pos: pad, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run_steps(2);

    // A slid projector: the object stays put, the ring marks Offset 0 and the filled dot where the projector went
    for (kind, offset, name) in [("planar", [0.3, 0.15, 0.0], "fill-planar-slid"), ("spherical", [0.5, 0.3, 0.0], "fill-spherical-slid")] {
        execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "projection": kind, "offset": offset})).unwrap();
        save(&mut harness, &output_dir, name);
    }

    // The Projection row's eye: hovered, then the gizmos hidden
    execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "projection": "planar", "offset": [0.0, 0.0, 0.0]})).unwrap();
    harness.run_steps(2);
    harness.get_by_label("Hide projection gizmos").hover();
    save(&mut harness, &output_dir, "fill-planar-eye-hover");
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Hide projection gizmos").click();
    harness.run_steps(2);
    save(&mut harness, &output_dir, "fill-planar-eye-hidden");
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Show projection gizmos").click();
    harness.run_steps(2);
    execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "projection": "uvTile"})).unwrap();
    harness.run_steps(2);
    harness.get_by_label("Fill (Match per UV Tile)").click();
    harness.run_steps(2);
    harness.get_by_label("Warp projection").hover();
    save(&mut harness, &output_dir, "fill-uvtile-projection-open");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    let session = &mut harness.state_mut().session;

    // Tri-planar's Shape crop sits highest in the panel: its list opens in view
    execute(session, "layer.fill.projection", &json!({"layer": layer, "projection": "triplanar"})).unwrap();
    harness.run_steps(2);
    harness.get_by_label("Extends Outside Shape").click();
    harness.run_steps(2);
    harness.get_by_label("Cropped to Shape").hover();
    save(&mut harness, &output_dir, "fill-triplanar-crop-open");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);

    // The shape previews take their colours from the theme
    for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("fill-triplanar-{name}"));
    }
}

/// Screen 03: a planar fill's Symmetry section off, mirrored (a moved Y plane with U flipped), radial (6 over a full turn, 4 over half a turn, also in the dark and light themes), its Type list open, and a UV fill's greyed switch
#[test]
#[ignore]
fn fill_symmetry_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "projection": "planar"})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1500.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Tab, "Symmetry").click();
    let cases = [
        (json!({}), "symmetry-off"),
        (json!({"symmetry": true}), "symmetry-mirror"),
        (json!({"symmetryAxis": 1, "axisPosition": [0.0, 0.4, 0.0], "flipCopy": [true, false]}), "symmetry-mirror-y"),
        (json!({"symmetryType": "radial", "symmetryAxis": 2, "axisPosition": 0, "flipCopy": [false, false]}), "symmetry-radial"),
        (json!({"symmetryCount": 4, "angleSpan": 180}), "symmetry-radial-half"),
    ];
    for (settings, name) in cases {
        let mut settings = settings;
        settings["layer"] = layer.clone();
        execute(&mut harness.state_mut().session, "layer.fill.projection", &settings).unwrap();
        save(&mut harness, &output_dir, name);
    }
    for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("symmetry-radial-half-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.get_by_role_and_label(egui::Role::ComboBox, "Radial symmetry").click();
    harness.run_steps(2);
    harness.get_by_label("Mirror symmetry").hover();
    save(&mut harness, &output_dir, "symmetry-type-open");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::RadioButton, "X").hover();
    save(&mut harness, &output_dir, "symmetry-axis-hover");

    execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": layer, "projection": "uv"})).unwrap();
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Tab, "Symmetry").click();
    harness.run_steps(4);
    harness.get_by_label("Not available with UV projection").hover();
    save(&mut harness, &output_dir, "symmetry-uv");
}

/// A material link as MaterialCraft leaves it on a fill: three float inputs, a preset and five outputs (the maps stay the fill's own sources)
fn steel_painted_worn() -> texturecraft_engine::core::MaterialLink {
    use serde_json::json;
    use texturecraft_engine::core::{LinkParam, LinkPreset, MaterialLink};
    let float = |id: &str, label: &str, value: f32| LinkParam {
        id: id.into(),
        label: label.into(),
        group: String::new(),
        value: json!({"t": "float", "v": value}),
        default: json!({"t": "float", "v": 0.5}),
        min: 0.0,
        max: 1.0,
        options: Vec::new(),
    };
    MaterialLink {
        name: "Steel Painted Worn".into(),
        package: String::new(),
        graph: "main".into(),
        params: vec![float("wear", "Wear", 0.55), float("hue", "Paint hue", 0.3), float("dirt", "Dirt", 0.2)],
        presets: vec![LinkPreset { name: "Fresh paint".into(), values: Default::default() }],
        inputs: Vec::new(),
        size: 1024,
        resolution: None,
        mapping: Default::default(),
        outputs: ["baseColor", "metallic", "roughness", "normal", "height"].map(|output| (output.to_owned(), format!("steel_{output}.png"))).into(),
    }
}

/// Screen 04: the colour picker window open on a uniform base colour with five swatches, its close, a slider, a swatch and + hovered, a swatch's menu, with its lists open (a hovered option previews nothing), in RGB and resized from its top left corner; `TC_UI_THEMES` set adds the dark and light themes
#[test]
#[ignore]
fn fill_color_picker_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let orange = [0xE5, 0x8A, 0x4E].map(|c| texturecraft_engine::texel::srgb_to_linear(f32::from(c as u8) / 255.0));
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": orange})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "layer.fill.set", &json!({"layer": layer, "channel": "height", "enabled": true})).unwrap();
    for swatch in ["#E58A4E", "#BEBEBE", "#3C4046", "#7A7AF5", "#B0623A"] {
        execute(&mut session, "swatch.add", &json!({"color": swatch})).unwrap();
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 2000.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Tab, "Material").click();

    // The tab scrolls the panel to its section (the scroll animates for about twenty frames)
    harness.run_steps(25);
    harness.get_by_role_and_label(egui::Role::Button, "Base color").click();
    harness.run_steps(2);
    save(&mut harness, &output_dir, "picker-open");
    let title = harness.get_by_role_and_label(egui::Role::TitleBar, "Base color").rect();
    harness.hover_at(egui::pos2(title.right() + 6.0, title.center().y));
    save(&mut harness, &output_dir, "picker-close-hover");
    harness.get_by_role_and_label(egui::Role::Slider, "S").hover();
    save(&mut harness, &output_dir, "picker-hover");
    harness.get_by_role_and_label(egui::Role::Button, "#7A7AF5").hover();
    save(&mut harness, &output_dir, "picker-swatch-hover");
    harness.get_by_role_and_label(egui::Role::Button, "Add swatch").hover();
    harness.run_steps(30);
    save(&mut harness, &output_dir, "picker-add-hover");
    let chip = harness.get_by_role_and_label(egui::Role::Button, "#7A7AF5").rect().center();
    harness.hover_at(chip);
    harness.run_steps(1);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: chip, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        harness.run_steps(1);
    }
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::Button, "Remove swatch").hover();
    save(&mut harness, &output_dir, "picker-swatch-menu");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ComboBox, "sRGB").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Linear").hover();
    save(&mut harness, &output_dir, "picker-space-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ComboBox, "HSV").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "RGB").hover();
    save(&mut harness, &output_dir, "picker-model-list");
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "RGB").click();
    harness.run_steps(2);
    save(&mut harness, &output_dir, "picker-rgb");

    // The window stands at the screen's bottom here, so it grows from its top left corner
    let corner = harness.get_by_role_and_label(egui::Role::TitleBar, "Base color").rect().left_top() + egui::vec2(2.0, 2.0);
    let by = egui::vec2(-120.0, -90.0);
    harness.hover_at(corner);
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton { pos: corner, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.run_steps(1);
    for step in 1..=4 {
        harness.hover_at(corner + by * (step as f32 / 4.0));
        harness.run_steps(1);
    }
    harness.event(egui::Event::PointerButton { pos: corner + by, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run_steps(2);
    save(&mut harness, &output_dir, "picker-resized");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("picker-resized-{name}"));
        }
    }
}

/// Screen 04: a fill's Material section in Channels (a procedural base colour with its Attributes open, a uniform metallic, a normal image, a signed height), its source list open with a hover, the picker on the base colour, the Material side of a plain fill (no material, its material list open with a hover, Copper picked; Copper also in the dark and light themes) and of a linked material; the linked one also in the dark and light themes
#[test]
#[ignore]
fn fill_material_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::{LayerId, LayerKind};
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_fn(16, 16, |x, y| image::Rgba([110 + (x * 8) as u8, 110 + (y * 8) as u8, 250, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let normal = session.import_resource_bytes("bricks_normal.png", png).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.55, 0.2, 0.08]})).unwrap()["layer"].as_u64().unwrap();
    for settings in [
        json!({"channel": "basecolor", "procedural": "Clouds", "low": [0.3, 0.08, 0.02], "high": [0.7, 0.3, 0.12]}),
        json!({"channel": "normal", "enabled": true}),
        json!({"channel": "normal", "image": normal}),
        json!({"channel": "height", "enabled": true}),
    ] {
        let mut settings = settings;
        settings["layer"] = json!(layer);
        execute(&mut session, "layer.fill.set", &settings).unwrap();
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1500.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Tab, "Material").click();
    save(&mut harness, &output_dir, "material-channels");

    // The orb follows the Layers panel's channel list: its list open, then one channel alone each
    harness.get_all_by_role_and_label(egui::Role::ComboBox, "Material").min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    harness.run_steps(2);
    save(&mut harness, &output_dir, "material-ball-list");
    harness.key_press(egui::Key::Escape);
    for channel in [texturecraft_engine::core::Channel::BaseColor, texturecraft_engine::core::Channel::Normal, texturecraft_engine::core::Channel::Height] {
        harness.state_mut().ui.layer_material_view = false;
        harness.state_mut().ui.layer_channel = channel;
        save(&mut harness, &output_dir, &format!("material-ball-{}", channel.id()));
    }
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("material-ball-height-{name}"));
        }
        harness.state_mut().ui.prefs.brightness = Brightness::default();
    }
    harness.state_mut().ui.layer_material_view = true;

    // The Base color slot (the lowest "Base color" list: the Layers panel has one too) with its list open
    lowest_list(&harness, "Base color").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Perlin noise").hover();
    save(&mut harness, &output_dir, "material-source-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get_all_by_role_and_label(egui::Role::Button, "Attributes").next().unwrap().click();
    save(&mut harness, &output_dir, "material-attributes");
    harness.get_all_by_role_and_label(egui::Role::Button, "Attributes").next().unwrap().click();
    harness.run_steps(2);
    harness.get_all_by_role_and_label(egui::Role::Button, "Invert").next().unwrap().hover();
    save(&mut harness, &output_dir, "material-invert-hover");

    // The procedural slot's Resolution: one row in its card, its one list holding the Relative factors and the Fixed sizes
    let texture_set = harness.state().session.doc.project.texture_sets[0].resolution;
    let resolution = format!("Texture set  ·  {texture_set}");
    harness.get_by_role_and_label(egui::Role::ComboBox, &resolution).click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "1024 × 1024").hover();
    save(&mut harness, &output_dir, "material-procedural-resolution-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);

    // The same plain fill on the Material side: one whole material, picked from the card's list
    harness.get_by_role_and_label(egui::Role::RadioButton, "Material").click();
    harness.run_steps(2);
    save(&mut harness, &output_dir, "material-whole-none");
    harness.get_by_role_and_label(egui::Role::ComboBox, "None").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Copper").hover();
    save(&mut harness, &output_dir, "material-whole-list");
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Copper").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::RadioButton, "Channels").hover();
    save(&mut harness, &output_dir, "material-whole-copper");
    for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("material-whole-copper-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.get_by_role_and_label(egui::Role::RadioButton, "Channels").click();
    harness.run_steps(2);
    save(&mut harness, &output_dir, "material-whole-copper-channels");
    harness.get_by_role_and_label(egui::Role::RadioButton, "Material").click();
    harness.run_steps(2);

    // Back to the fill the linked shots below expect (the pick was one undo step)
    execute(&mut harness.state_mut().session, "edit.undo", &json!({})).unwrap();

    let session = &mut harness.state_mut().session;
    if let Some(LayerKind::Fill(fill)) = session.doc.project.layers.get_mut(&LayerId(layer)).map(|layer| &mut layer.kind) {
        fill.link = Some(Box::new(steel_painted_worn()));
    }
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Tab, "Material").click();
    save(&mut harness, &output_dir, "material-linked");

    // The bands sit below the panel's fold: scroll each control into view (the scroll animates for about twenty frames)
    let reach = |harness: &mut Harness<'_, TextureCraftApp>, role: egui::Role, label: &str| {
        harness.get_by_role_and_label(role, label).scroll_to_me();
        harness.run_steps(25);
    };

    // The band is folded by the time it is reached in this run: unfold it first
    if harness.query_by_role_and_label(egui::Role::ComboBox, "normal").is_none() {
        reach(&mut harness, egui::Role::Button, "Channels mapping");
        harness.get_by_role_and_label(egui::Role::Button, "Channels mapping").click();
        harness.run_steps(2);
    }
    reach(&mut harness, egui::Role::ComboBox, "normal");
    harness.get_by_role_and_label(egui::Role::ComboBox, "roughness").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "normal").hover();
    save(&mut harness, &output_dir, "material-mapping-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    reach(&mut harness, egui::Role::Button, "Resolution");
    save(&mut harness, &output_dir, "material-resolution-folded");
    harness.get_by_role_and_label(egui::Role::Button, "Resolution").click();
    harness.run_steps(2);
    reach(&mut harness, egui::Role::ComboBox, "Texture set  ·  256");
    save(&mut harness, &output_dir, "material-resolution");
    harness.get_by_role_and_label(egui::Role::ComboBox, "Texture set  ·  256").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "×2  ·  512").hover();
    save(&mut harness, &output_dir, "material-resolution-relative-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::RadioButton, "Fixed").click();
    harness.run_steps(2);

    // This link has no package, so the re-cook fails with a toast over the panel
    harness.state_mut().toasts.clear();
    harness.get_by_role_and_label(egui::Role::ComboBox, "256 × 256").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "1024 × 1024").hover();
    save(&mut harness, &output_dir, "material-resolution-fixed-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::Button, "Resolution").click();
    harness.run_steps(2);
    save(&mut harness, &output_dir, "material-resolution-fixed-folded");
    for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("material-linked-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.get_by_role_and_label(egui::Role::RadioButton, "Channels").click();
    save(&mut harness, &output_dir, "material-linked-channels");
}

/// The armed eyedropper is the pointer over the panels too, its tip on the hovered pixel
#[test]
#[ignore]
fn eyedropper_pointer_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();

    // A light and a dark model: the disc's white outline and faint dark ring must read on both
    for (name, color) in [("light", [0.92, 0.92, 0.9]), ("dark", [0.01, 0.01, 0.012])] {
        let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
        execute(&mut session, "layer.add.fill", &json!({"color": color})).unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1600.0, 960.0))
            .with_pixels_per_point(1.0)
            .wgpu()
            .with_context_setup(texturecraft_ui_egui::framework::install_headless)
            .build_eframe(|cc| TextureCraftApp::new(cc, session));
        harness.state_mut().ui.prefs.language = "en".into();

        // Only the app's own pointer shows, not the test's stand-in arrow
        harness.set_render_cursor(false);
        harness.run_steps(4);
        harness.get_by_role_and_label(egui::Role::Tab, "Material").click();
        harness.run_steps(3);
        harness.get_by_role_and_label(egui::Role::Button, "Base color").click();
        harness.run_steps(3);
        harness.get_all_by_role_and_label(egui::Role::Button, "Pick color").next().unwrap().click();
        harness.run_steps(3);
        harness.hover_at(egui::pos2(520.0, 430.0));
        harness.run_steps(2);
        save(&mut harness, &output_dir, &format!("eyedropper-over-{name}-model"));
        if name == "light" {
            let over_panel = harness.get_by_role_and_label(egui::Role::Tab, "Material").rect().center();
            harness.hover_at(over_panel);
            harness.run_steps(2);
            save(&mut harness, &output_dir, "eyedropper-over-properties");
            harness.hover_at(egui::pos2(1590.0, 300.0));
            harness.run_steps(2);
            save(&mut harness, &output_dir, "eyedropper-at-right-edge");
            harness.hover_at(egui::pos2(12.0, 950.0));
            harness.run_steps(2);
            save(&mut harness, &output_dir, "eyedropper-at-bottom-left-corner");
        }
    }
}

/// Gives the shelf thumbnails rendered off the UI thread time to arrive
fn settle_thumbnails(h: &mut Harness<'_, TextureCraftApp>) {
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.run_steps(1);
    }
}

/// The channel source picker opened from each kind of source, searching, on Anchor points, and the filter picker in the same style
#[test]
#[ignore]
fn source_picker_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    let rust = image::RgbaImage::from_fn(64, 64, |x, y| image::Rgba([150 + ((x * 7 + y * 3) % 60) as u8, 70 + ((x * y) % 40) as u8, 30, 255]));
    rust.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let rust = session.import_resource_bytes("rust_albedo.png", png).unwrap();
    let below = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "effect.add.anchor", &json!({"layer": below, "name": "Edge wear"})).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.45, 0.13, 0.05]})).unwrap()["layer"].clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Tab, "Material").click();
    harness.run_steps(3);

    let shot = |harness: &mut Harness<'_, TextureCraftApp>, mut source: serde_json::Value, hover: Option<&str>, search: Option<&str>, name: &str| {
        source["layer"] = layer.clone();
        source["channel"] = json!("baseColor");
        execute(&mut harness.state_mut().session, "layer.fill.set", &source).unwrap();
        harness.run_steps(3);
        lowest_list(harness, "Base color").click();
        harness.run_steps(30);
        if let Some(text) = search {
            harness.event(egui::Event::Text(text.into()));
            harness.run_steps(3);
        }
        settle_thumbnails(harness);
        if let Some(label) = hover {
            let at = harness.get_by_role_and_label(egui::Role::ListBoxOption, label).rect().center();
            harness.hover_at(at);
        }
        save(harness, &output_dir, name);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
    };
    shot(&mut harness, json!({"color": [0.45, 0.13, 0.05]}), Some("Grunge"), None, "source-picker-top");
    shot(&mut harness, json!({"image": rust}), Some("Import..."), None, "source-picker-textures");
    shot(&mut harness, json!({"alpha": "scratches"}), Some("Cracks"), None, "source-picker-alphas");
    shot(&mut harness, json!({"alpha": "letter_y"}), None, None, "source-picker-end");
    shot(&mut harness, json!({"color": [0.45, 0.13, 0.05]}), None, Some("scr"), "source-picker-search");

    // Anchor points on the Roughness slot, scrolled into view first
    harness.get_by_role_and_label(egui::Role::ComboBox, "Roughness").scroll_to_me();
    harness.run_steps(25);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Roughness").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::Role::Tab, "Anchor points").click();
    harness.run_steps(3);
    let anchor = harness.get_all_by_role(egui::Role::ListBoxOption).next().unwrap().rect().center();
    harness.hover_at(anchor);
    save(&mut harness, &output_dir, "source-picker-anchors");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    // The picked procedural's card, then the three themes with the picker open from Uniform
    execute(&mut harness.state_mut().session, "layer.fill.set", &json!({"layer": layer, "channel": "baseColor", "procedural": "Clouds", "scale": 4.0}))
        .unwrap();
    save(&mut harness, &output_dir, "source-picker-picked");
    execute(&mut harness.state_mut().session, "layer.fill.set", &json!({"layer": layer, "channel": "baseColor", "color": [0.45, 0.13, 0.05]})).unwrap();
    harness.run_steps(3);
    lowest_list(&harness, "Base color").click();
    settle_thumbnails(&mut harness);
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("source-picker-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    // The filter picker in the same style
    let filter = execute(&mut harness.state_mut().session, "effect.add.filter", &json!({"target": "content", "filter": "invert"})).unwrap();
    let filter = texturecraft_engine::core::LayerId(filter["effect"].as_u64().unwrap());
    harness.state_mut().session.target = texturecraft_engine::StackTarget::Effect(filter);
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Invert").click();
    settle_thumbnails(&mut harness);
    let blur = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Blur").rect().center();
    harness.hover_at(blur);
    save(&mut harness, &output_dir, "filter-picker-v4");
}

/// The culling rows' i on a planar fill: each one hovered with its tooltip, in the medium theme, then the Depth culling one in the dark and light themes
#[test]
#[ignore]
fn info_icon_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "projection": "planar"})).unwrap();
    execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "scaleMode": "physicalSize", "backfaceCulling": true})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1500.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();

    // The test's stand-in arrow would hide the hovered i
    harness.set_render_cursor(false);
    harness.run_steps(4);
    let tips = [
        ("Paints only the surface the projector sees first; parts hidden behind it stay unpainted.", "depth"),
        ("Leaves out faces turned away from the projector, so the fill doesn't run onto their back.", "backface"),
    ];
    for (tip, name) in tips {
        let info = harness.get_by_label(tip).rect().center();
        harness.hover_at(info);
        harness.run_steps(60);
        save(&mut harness, &output_dir, &format!("info-{name}-medium"));
    }
    let info = harness.get_by_label(tips[0].0).rect().center();
    for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        harness.hover_at(info);
        harness.run_steps(60);
        save(&mut harness, &output_dir, &format!("info-depth-{name}"));
    }
}

/// The right-click Properties popup on a UV fill: its jump tabs with Symmetry hovered, Material clicked, then wheeled up to Symmetry; `TC_UI_THEMES` set adds the dark and light themes
#[test]
#[ignore]
fn popup_tabs_snapshots() {
    use egui::{Event, Modifiers, PointerButton};
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "projection": "uv"})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.set_render_cursor(false);
    harness.run_steps(4);

    let at = harness.state().viewport_rect.center() - egui::vec2(200.0, 250.0);
    harness.hover_at(at);
    for pressed in [true, false] {
        harness.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        harness.run_steps(2);
    }
    harness.run_steps(4);
    let popup = harness.ctx.memory(|memory| memory.area_rect(texturecraft_ui_egui::properties_popup::area_id())).unwrap();
    let tab = |harness: &Harness<'_, TextureCraftApp>, name: &str| {
        harness.get_all_by_role_and_label(egui::accesskit::Role::Tab, name).map(|tab| tab.rect()).find(|tab| popup.contains_rect(*tab)).unwrap().center()
    };
    let symmetry = tab(&harness, "Symmetry");
    harness.hover_at(symmetry);
    harness.run_steps(4);
    save(&mut harness, &output_dir, "popup-tabs-fill-medium");

    let material = tab(&harness, "Material");
    harness.hover_at(material);
    for pressed in [true, false] {
        harness.event(Event::PointerButton { pos: material, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        harness.run_steps(2);
    }
    harness.run_steps(60);
    save(&mut harness, &output_dir, "popup-tabs-material-medium");

    harness.hover_at(popup.center());
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 260.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    harness.run_steps(6);
    save(&mut harness, &output_dir, "popup-tabs-symmetry-medium");

    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            harness.run_steps(6);
            save(&mut harness, &output_dir, &format!("popup-tabs-symmetry-{name}"));
        }
    }
}

/// Screen 07: a mask fill effect's Grayscale section from each kind of source (Uniform, image, procedural, anchor), its picker open with a tile hovered, and its Fill section; `TC_UI_THEMES` set adds the dark and light themes
#[test]
#[ignore]
fn mask_fill_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    let scratches = image::RgbaImage::from_fn(64, 64, |x, y| {
        let gray = 90 + ((x * 13 + y * 7) % 90) as u8;
        image::Rgba([gray, gray, gray, 255])
    });
    scratches.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let scratches = session.import_resource_bytes("scratches.png", png).unwrap();
    let below = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    let anchor = execute(&mut session, "effect.add.anchor", &json!({"layer": below, "name": "Cavity Dirt"})).unwrap()["effect"].clone();
    let layer = execute(&mut session, "layer.add.paint", &json!({})).unwrap()["layer"].clone();
    let effect = execute(&mut session, "effect.add.fill", &json!({"layer": layer, "target": "mask"})).unwrap()["effect"].clone();
    execute(&mut session, "layer.select", &json!({"layer": layer, "target": effect})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "mask-fill-fill");
    harness.get_by_role_and_label(egui::Role::Tab, "Grayscale").click();
    harness.run_steps(30);

    let shot = |harness: &mut Harness<'_, TextureCraftApp>, mut source: serde_json::Value, name: &str| {
        source["effect"] = effect.clone();
        execute(&mut harness.state_mut().session, "layer.fill.set", &source).unwrap();
        harness.run_steps(3);
        settle_thumbnails(harness);
        save(harness, &output_dir, name);
    };
    shot(&mut harness, json!({"value": 1.0}), "mask-fill-uniform");
    shot(&mut harness, json!({"image": scratches}), "mask-fill-source-image");
    shot(&mut harness, json!({"procedural": "Clouds", "scale": 4.0, "seed": 12}), "mask-fill-procedural");

    // The procedural's Resolution list open (Relative factors and Fixed sizes) with a size hovered
    let click_in_view = |harness: &mut Harness<'_, TextureCraftApp>, role: egui::Role, label: &str| {
        harness.get_by_role_and_label(role, label).scroll_to_me();
        harness.run_steps(3);
        harness.get_by_role_and_label(role, label).click();
        harness.run_steps(3);
    };
    click_in_view(&mut harness, egui::Role::ComboBox, "Texture set  ·  256");
    let hovered = harness.get_by_role_and_label(egui::Role::ListBoxOption, "×2  ·  512").rect().center();
    harness.hover_at(hovered);
    save(&mut harness, &output_dir, "mask-fill-resolution-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    // The picker opened from the slot in view (Properties still scrolled to the card), a tile hovered
    harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").click();
    harness.run_steps(3);
    settle_thumbnails(&mut harness);
    let hovered = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Clouds").rect().center();
    harness.hover_at(hovered);
    save(&mut harness, &output_dir, "mask-fill-picker");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("mask-fill-picker-{name}"));
        }
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    shot(&mut harness, json!({"anchor": anchor}), "mask-fill-anchor");

    // Again once the anchored stack's picture has rendered
    settle_thumbnails(&mut harness);
    save(&mut harness, &output_dir, "mask-fill-anchor");

    shot(&mut harness, json!({"value": 1.0}), "mask-fill-uniform-again");
}

/// Screen 09: a compare mask at its defaults, within tolerance, its right source's list open with Constant hovered, then Constant picked
#[test]
#[ignore]
fn compare_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut layer = json!(null);
    for (low, high) in [(-0.7, 0.1), (-0.2, 0.6)] {
        layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
        let (low, high) = (json!([low, low, low]), json!([high, high, high]));
        execute(&mut session, "layer.fill.set", &json!({"layer": layer, "channel": "height", "procedural": "Clouds", "scale": 3.0, "low": low, "high": high}))
            .unwrap();
    }
    let compare = execute(&mut session, "effect.add.compare", &json!({"layer": layer})).unwrap()["effect"].clone();
    execute(&mut session, "layer.select", &json!({"layer": layer, "target": compare})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "compare-default");

    harness.get_by(|node| node.role() == egui::Role::RadioButton && node.label().as_deref() == Some("≈")).click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "compare-within");
    harness.get_by(|node| node.role() == egui::Role::RadioButton && node.label().as_deref() == Some(">")).click();
    harness.run_steps(4);

    harness.get_by_role_and_label(egui::Role::ComboBox, "Layers below").click();
    harness.run_steps(3);
    let hovered = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Constant").rect().center();
    harness.hover_at(hovered);
    save(&mut harness, &output_dir, "compare-source-list");
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Constant").click();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "compare-constant");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("compare-constant-{name}"));
        }
    }
}

/// Screen 11: a new anchor point on a fill's content being renamed, then a mask anchor point read by two layers above (a reference hovered)
#[test]
#[ignore]
fn anchor_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let low = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "layer.rename", &json!({"layer": low, "name": "Cavity Dirt"})).unwrap();
    execute(&mut session, "effect.add.generator", &json!({"layer": low, "generator": "dirt"})).unwrap();
    let anchor = execute(&mut session, "effect.add.anchor", &json!({"layer": low, "target": "mask"})).unwrap()["effect"].clone();
    for name in ["Fill layer 2", "Rust"] {
        let reader = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
        execute(&mut session, "layer.rename", &json!({"layer": reader, "name": name})).unwrap();
        execute(&mut session, "mask.add.black", &json!({"layer": reader})).unwrap();
        execute(&mut session, "effect.add.fill", &json!({"layer": reader, "anchor": anchor})).unwrap();
    }
    let fresh = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    let content = execute(&mut session, "effect.add.anchor", &json!({"layer": fresh})).unwrap()["effect"].clone();
    execute(&mut session, "layer.select", &json!({"layer": fresh, "target": content})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    harness.get_by_role_and_label(egui::Role::TextInput, "Anchor point name").click();
    harness.run_steps(3);
    save(&mut harness, &output_dir, "anchor-new-renaming");

    harness.key_press(egui::Key::Escape);
    harness.state_mut().session.target = texturecraft_engine::StackTarget::Content;
    execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": low, "target": anchor})).unwrap();
    harness.run_steps(6);
    let row = harness.get_by_role_and_label(egui::Role::Button, "Rust Mask fill").rect().center();
    harness.hover_at(row);
    save(&mut harness, &output_dir, "anchor-mask-references");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("anchor-mask-references-{name}"));
        }
    }
}

/// Screen 15: a folder of fills at rest, scaled x1.5 and turned +30 with the reset hovered, and an empty folder
#[test]
#[ignore]
fn folder_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let empty = execute(&mut session, "layer.add.group", &json!({})).unwrap()["layer"].clone();
    let fill = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "layer.move", &json!({"layer": fill, "above": empty, "inside": false})).unwrap();
    execute(&mut session, "layer.fill.set", &json!({"layer": fill, "channel": "baseColor", "procedural": "Clouds"})).unwrap();
    let folder = execute(&mut session, "layer.group", &json!({"layer": fill})).unwrap()["layer"].clone();
    execute(&mut session, "layer.select", &json!({"layer": folder, "target": "content"})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "folder-rest");

    execute(&mut harness.state_mut().session, "folder.contents.set", &json!({"layer": folder, "scale": 1.5, "rotation": 30.0})).unwrap();
    harness.run_steps(4);
    let reset = harness.get_by_role_and_label(egui::Role::Button, "Contents: Reset to defaults").rect().center();
    harness.hover_at(reset);
    save(&mut harness, &output_dir, "folder-offset-reset-hovered");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("folder-offset-{name}"));
        }
        harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    }

    execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": empty, "target": "content"})).unwrap();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "folder-empty");
}

/// Screen 16: an instance of an orange fill named Steel (at rest, its card hovered), then one whose source is gone
#[test]
#[ignore]
fn instance_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::{LayerId, LayerKind};
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let source = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "layer.rename", &json!({"layer": source, "name": "Steel"})).unwrap();
    execute(&mut session, "layer.fill.set", &json!({"layer": source, "channel": "baseColor", "color": [0.78, 0.29, 0.09]})).unwrap();
    let instance = execute(&mut session, "layer.instance", &json!({"layer": source})).unwrap()["layer"].clone();
    execute(&mut session, "layer.select", &json!({"layer": instance, "target": "content"})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "instance-rest");
    let set = harness.state().session.doc.project.texture_sets[0].name.clone();
    let card = harness.get_by_role_and_label(egui::Role::Button, &format!("Steel {set}")).rect().center();
    harness.hover_at(card);
    save(&mut harness, &output_dir, "instance-hovered");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("instance-{name}"));
        }
        harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    }

    let id = LayerId(instance.as_u64().unwrap());
    harness.state_mut().session.doc.project.layers.get_mut(&id).unwrap().kind = LayerKind::Instance { source: LayerId(999_999) };
    harness.hover_at(egui::pos2(10.0, 10.0));
    save(&mut harness, &output_dir, "instance-missing");
}

/// Screen 17: a folder's mask selected with the paint tool's Grayscale tab open: painting white, black, then an image (the slot hovered), then the slot's picker open
#[test]
#[ignore]
fn folder_mask_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let scratches = image::RgbaImage::from_fn(64, 64, |x, y| {
        let value = (((x * 7 + y * 13) ^ (x * y)) % 200 + 40) as u8;
        image::Rgba([value, value, value, 255])
    });
    let mut bytes = Vec::new();
    scratches.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    let image = session.import_resource_bytes("scratches.png", bytes).unwrap();
    let folder = execute(&mut session, "layer.add.group", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "mask.add.white", &json!({"layer": folder})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": folder, "target": "mask"})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.props_tab = texturecraft_ui_egui::state::PropsTab::Material;
    harness.run_steps(6);
    save(&mut harness, &output_dir, "foldermask-white");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("foldermask-{name}"));
        }
        harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    }
    execute(&mut harness.state_mut().session, "tool.material", &json!({"maskValue": 0.0})).unwrap();
    save(&mut harness, &output_dir, "foldermask-black");
    execute(&mut harness.state_mut().session, "tool.material", &json!({"maskImage": image})).unwrap();
    let slot = harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").rect().center();
    harness.hover_at(slot);
    save(&mut harness, &output_dir, "foldermask-image");
    harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "foldermask-picker");
}

/// Screen 10: a new color selection without an ID map, then three colors on the ID mesh map (a row hovered, a row picking again, the Background list open)
#[test]
#[ignore]
fn color_selection_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let shoot = |with_id_map: bool| {
        let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
        if !with_id_map {
            execute(&mut session, "textureSet.meshMap.clear", &json!({"map": "id"})).unwrap();
        }
        let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
        let added = execute(&mut session, "mask.add.colorSelection", &json!({"layer": layer})).unwrap();
        let effect = texturecraft_engine::core::LayerId(added["effect"].as_u64().unwrap());
        if with_id_map {
            let colors = json!([[214.0 / 255.0, 1.0, 0.0], [1.0, 49.0 / 255.0, 0.0], [1.0, 0.0, 247.0 / 255.0]]);
            execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": colors})).unwrap();
        }
        execute(&mut session, "layer.select", &json!({"layer": layer, "target": effect.0})).unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1600.0, 1200.0))
            .with_pixels_per_point(2.0)
            .wgpu()
            .with_context_setup(texturecraft_ui_egui::framework::install_headless)
            .build_eframe(|cc| TextureCraftApp::new(cc, session));
        harness.state_mut().ui.prefs.language = "en".into();
        harness.run_steps(6);
        harness
    };

    let mut harness = shoot(false);
    save(&mut harness, &output_dir, "colorsel-new");

    // Picking with an ID Mask image chosen by hand and nothing baked: the model shows that image
    let quarters = image::RgbaImage::from_fn(8, 8, |x, y| match (x < 4, y < 4) {
        (true, true) => image::Rgba([214, 255, 0, 255]),
        (false, true) => image::Rgba([255, 49, 0, 255]),
        (true, false) => image::Rgba([255, 0, 247, 255]),
        (false, false) => image::Rgba([20, 90, 255, 255]),
    });
    let mut png = Vec::new();
    quarters.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut harness = shoot(false);
    let effect = match harness.state().session.target {
        texturecraft_engine::StackTarget::Effect(effect) => effect,
        other => panic!("the color selection is selected, not {other:?}"),
    };
    harness.state_mut().session.doc.baked.clear();
    let name = harness.state_mut().session.import_resource_bytes("parts.png", png).unwrap();
    harness.state_mut().run("effect.set", json!({"effect": effect.0, "idMap": name}));
    harness.run_steps(4);
    harness.get_by_label("Pick color").click();
    harness.run_steps(8);
    save(&mut harness, &output_dir, "colorsel-image-pick");

    let mut harness = shoot(true);
    let row = harness.get_by_label("Pick color R 255 G 49 B 0").rect().center();
    harness.hover_at(row);
    harness.run_steps(2);
    save(&mut harness, &output_dir, "colorsel-hover");
    let first = harness.get_by_label("Pick color R 214 G 255 B 0").rect().center();
    harness.get_by_label("Pick color R 214 G 255 B 0").click();
    harness.run_steps(4);
    harness.hover_at(first);
    harness.run_steps(2);
    save(&mut harness, &output_dir, "colorsel-repick");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);

    harness.get_by_role_and_label(egui::Role::ComboBox, "Black").click();
    harness.run_steps(3);
    let hovered = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Transparent").rect().center();
    harness.hover_at(hovered);
    save(&mut harness, &output_dir, "colorsel-background-list");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("colorsel-{name}"));
        }
    }
}

/// Screen 08: a mask's levels, then a content levels on Base color (Colors, red only with Clamp on, its Affects list open with an option hovered) and on Roughness
#[test]
#[ignore]
fn levels_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"roughness": 0.5})).unwrap()["layer"].clone();
    for (channel, low, high) in
        [("baseColor", json!([0.1, 0.2, 0.45]), json!([0.85, 0.5, 0.2])), ("roughness", json!([0.25, 0.25, 0.25]), json!([0.7, 0.7, 0.7]))]
    {
        execute(&mut session, "layer.fill.set", &json!({"layer": layer, "channel": channel, "procedural": "Clouds", "scale": 3.0, "low": low, "high": high}))
            .unwrap();
    }
    execute(&mut session, "mask.add.black", &json!({"layer": layer})).unwrap();
    execute(&mut session, "effect.add.fill", &json!({"layer": layer, "target": "mask", "procedural": "clouds"})).unwrap();
    let mask_levels = execute(&mut session, "effect.add.levels", &json!({"layer": layer, "target": "mask"})).unwrap()["effect"].clone();
    let content_levels = execute(&mut session, "effect.add.levels", &json!({"layer": layer, "target": "content"})).unwrap()["effect"].clone();
    execute(&mut session, "layer.select", &json!({"layer": layer, "target": mask_levels})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "levels-mask");

    execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": layer, "target": content_levels})).unwrap();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "levels-colors");
    let switch = |harness: &mut Harness<'_, TextureCraftApp>, view: &str| {
        harness.get_by(|node| node.role() == egui::Role::RadioButton && node.label().as_deref() == Some(view)).click();
        harness.run_steps(3);
    };
    switch(&mut harness, "R");
    harness.get_by_role_and_label(egui::Role::CheckBox, "Clamp").click();
    harness.run_steps(3);
    save(&mut harness, &output_dir, "levels-red-clamp");
    harness.get_by_role_and_label(egui::Role::CheckBox, "Clamp").click();
    switch(&mut harness, "Colors");
    lowest_list(&harness, "Base color").click();
    harness.run_steps(3);
    let hovered = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Roughness").rect().center();
    harness.hover_at(hovered);
    save(&mut harness, &output_dir, "levels-affects-list");
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Roughness").click();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "levels-roughness");
    harness.get_by_role_and_label(egui::Role::Slider, "Gamma").focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    save(&mut harness, &output_dir, "levels-exact-entry");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("levels-roughness-{name}"));
        }
    }
}

/// Screen 14: the Layers panel's top row (Add fill layer hovered), Add effect on a layer and on its mask, Add mask, and Bitmap's texture picker; the panel's left edge goes to stderr for cropping
#[test]
#[ignore]
fn layer_toolbar_snapshots() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let dirt = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "layer.rename", &json!({"layer": dirt, "name": "Dirt"})).unwrap();
    execute(&mut session, "effect.add.generator", &json!({"layer": dirt, "generator": "dirt"})).unwrap();
    let fill = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "layer.select", &json!({"layer": fill})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    harness.state_mut().ui.dock.focus(Panel::Layers);
    harness.run_steps(3);
    eprintln!("toolbar at {:?}", harness.get_by_role_and_label(egui::Role::Button, "Add effect").rect());
    let hover = |harness: &mut Harness<'_, TextureCraftApp>, label: &str| {
        let at = harness.get_by_role_and_label(egui::Role::Button, label).rect().center();
        harness.hover_at(at);
    };
    let open = |harness: &mut Harness<'_, TextureCraftApp>, menu: &str| {
        harness.get_by_role_and_label(egui::Role::Button, menu).click();
        harness.run_steps(3);
    };
    hover(&mut harness, "Add fill layer");
    save(&mut harness, &output_dir, "toolbar-rest");
    open(&mut harness, "Add effect");
    hover(&mut harness, "Filter");
    save(&mut harness, &output_dir, "toolbar-effect-content");
    harness.key_press(egui::Key::Escape);
    execute(&mut harness.state_mut().session, "mask.add.white", &json!({"layer": fill})).unwrap();
    harness.run_steps(3);
    open(&mut harness, "Add effect");
    hover(&mut harness, "Compare mask");
    save(&mut harness, &output_dir, "toolbar-effect-mask");
    harness.key_press(egui::Key::Escape);
    execute(&mut harness.state_mut().session, "edit.undo", &json!({})).unwrap();
    harness.run_steps(3);
    open(&mut harness, "Add mask");
    hover(&mut harness, "Height combination");
    save(&mut harness, &output_dir, "toolbar-mask");
    harness.get_by_role_and_label(egui::Role::Button, "Bitmap").click();
    harness.run_steps(3);
    save(&mut harness, &output_dir, "toolbar-bitmap");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        harness.key_press(egui::Key::Escape);
        open(&mut harness, "Add effect");
        hover(&mut harness, "Filter");
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("toolbar-effect-content-{name}"));
        }
    }
}

/// Screen 12: the Filter effect: empty, Blur on a mask, Slope Blur on the content (pills, Seed, Intensity's chip), its Divider list open, Restore defaults hovered
#[test]
#[ignore]
fn filter_card_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    let add = |session: &mut texturecraft_engine::Session, params: serde_json::Value| {
        LayerId(execute(session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap())
    };
    let empty = add(&mut session, json!({"layer": layer, "target": "content", "filter": "none"}));
    let blur = add(&mut session, json!({"layer": layer, "target": "mask", "filter": "blur"}));
    let slope = add(&mut session, json!({"layer": layer, "target": "content", "filter": "blurSlope", "seed": 12}));
    execute(&mut session, "effect.set", &json!({"effect": slope.0, "divider": 10, "channels": {"metallic": false, "normal": false}})).unwrap();
    let invert = add(&mut session, json!({"layer": layer, "target": "content", "filter": "invert"}));
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(empty);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);

    // Add filter opens the picker; a filter selected from its row does not
    save(&mut harness, &output_dir, "filter-empty");
    harness.state_mut().session.target = StackTarget::Effect(blur);
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-blur-mask");
    harness.state_mut().session.target = StackTarget::Effect(invert);
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-invert-content");
    harness.state_mut().session.target = StackTarget::Effect(slope);
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-slope");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("filter-slope-{name}"));
        }
        harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    }
    harness.get_by_role_and_label(egui::Role::ComboBox, "Blur").click();
    harness.run_steps(3);
    let option = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Max").rect().center();
    harness.hover_at(option);
    save(&mut harness, &output_dir, "filter-slope-blending-open");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    let reset = harness.get_by_role_and_label(egui::Role::Button, "Restore defaults").rect().center();
    harness.hover_at(reset);
    save(&mut harness, &output_dir, "filter-reset-hovered");
    harness.get_by_role_and_label(egui::Role::ComboBox, "÷10").click();
    harness.run_steps(3);
    let option = harness.get_by_role_and_label(egui::Role::ListBoxOption, "÷100").rect().center();
    harness.hover_at(option);
    save(&mut harness, &output_dir, "filter-divider-open");
}

#[test]
#[ignore = "writes the material picker (Material mode) to TC_UI_SNAPSHOTS; run with --ignored"]
/// A whole-material fill's card opens the material picker: grouped under its categories, Copper chosen and Brass hovered, then a search
fn material_picker_snapshots() {
    use serde_json::json;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "layer.fill.material", &json!({"layer": layer, "material": "copper"})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Content;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 2000.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Tab, "Material").click();
    harness.run_steps(4);
    if harness.query_by_role_and_label(egui::Role::ComboBox, "Copper").is_none() {
        harness.get_by_role_and_label(egui::Role::RadioButton, "Material").click();
        harness.run_steps(4);
    }
    harness.get_by_role_and_label(egui::Role::ComboBox, "Copper").click();
    harness.run_steps(30);
    let brass = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Brass").rect().center();
    harness.hover_at(brass);
    harness.run_steps(4);
    save(&mut harness, &output_dir, "material-picker-open");
    harness.event(egui::Event::Text("steel".into()));
    harness.run_steps(6);
    save(&mut harness, &output_dir, "material-picker-search");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, name) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            harness.run_steps(4);
            save(&mut harness, &output_dir, &format!("material-picker-search-{name}"));
        }
    }
}

#[test]
#[ignore = "writes the colour filters (screen 18) to TC_UI_SNAPSHOTS; run with --ignored"]
/// Screen 18: Color Balance, Color Correct, HSL Perceptive and Contrast Luminosity with the mock's values, one knob hovered
fn colour_filter_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    let mut add = |params: serde_json::Value| {
        let mut params = params;
        params["layer"] = layer.clone();
        params["target"] = json!("content");
        LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap())
    };
    let effects = [
        ("colour-balance", add(json!({"filter": "colorBalance", "highlights": [0.0, 0.0, -0.2], "midtones": [0.25, 0.0, 0.0], "shadows": [0.0, -0.1, 0.0]}))),
        ("colour-correct", add(json!({"filter": "colorCorrect", "shadows": [0.0, 0.1, 0.5], "midtones": [0.2, 0.0, 0.6], "highlights": [0.0, -0.1, 0.5]}))),
        ("hsl", add(json!({"filter": "hsl", "hue": 0.12, "saturation": 0.0, "lightness": -0.1}))),
        ("contrast-luminosity", add(json!({"filter": "contrastLuminosity", "contrast": 0.3}))),
    ];
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(effects[0].1);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    for (name, effect) in effects {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(4);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    harness.state_mut().session.target = StackTarget::Effect(effects[2].1);
    harness.run_steps(4);
    let hue = harness.get_by_role_and_label(egui::Role::Slider, "Hue").rect();
    harness.hover_at(egui::pos2(hue.left() + hue.width() * 0.62, hue.center().y));
    save(&mut harness, &output_dir, "filter-hsl-hovered");
    harness.state_mut().session.target = StackTarget::Effect(effects[0].1);
    harness.run_steps(4);
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("filter-colour-balance-{theme}"));
        }
    }
}

#[test]
#[ignore = "writes the gradient filters (screen 19) to TC_UI_SNAPSHOTS; run with --ignored"]
/// Screen 19: Gradient, Gradient Curve (and its Preset list open, Bump hovered) and Gradient Dynamic
fn gradient_filter_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "mask.add.white", &json!({"layer": layer.clone()})).unwrap();
    let mut add = |params: serde_json::Value| {
        let mut params = params;
        params["layer"] = layer.clone();
        params["target"] = json!("mask");
        LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap())
    };
    let stops = json!([[0.009, 0.008, 0.013], [0.55, 0.105, 0.021], [0.87, 0.67, 0.35]]);
    let effects = [
        ("gradient", add(json!({"filter": "gradientMap", "colors": stops, "positions": [0.0, 0.45, 1.0]}))),
        ("gradient-curve", add(json!({"filter": "gradientCurve"}))),
        ("gradient-dynamic", add(json!({"filter": "gradientDynamic", "color": [0.578, 0.578, 0.578]}))),
    ];
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(effects[0].1);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    for (name, effect) in effects {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(4);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    harness.state_mut().session.target = StackTarget::Effect(effects[1].1);
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Round").click();
    harness.run_steps(3);
    let bump = harness.get_by_label("Bump").rect();
    harness.hover_at(bump.center());
    save(&mut harness, &output_dir, "filter-gradient-curve-presets");
    harness.key_press(egui::Key::Escape);
    harness.state_mut().session.target = StackTarget::Effect(effects[0].1);
    harness.run_steps(4);
    let marker = harness.get_all_by_role_and_label(egui::Role::Slider, "Color 2").next().unwrap().rect();
    harness.hover_at(marker.center());
    save(&mut harness, &output_dir, "filter-gradient-marker-hovered");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("filter-gradient-{theme}"));
        }
    }
}

#[test]
#[ignore = "writes the light and shadow filters (screen 20) to TC_UI_SNAPSHOTS; run with --ignored"]
/// Screen 20: Glow (and its Width hovered) and Drop Shadow
fn light_filter_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "mask.add.white", &json!({"layer": layer.clone()})).unwrap();
    let mut add = |filter: &str| {
        let params = json!({"layer": layer.clone(), "target": "mask", "filter": filter});
        LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap())
    };
    let effects = [("glow", add("glow")), ("drop-shadow", add("dropShadow"))];
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(effects[0].1);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    for (name, effect) in effects {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(6);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    let distance = harness.get_by_role_and_label(egui::Role::Slider, "Distance").rect();
    harness.hover_at(distance.center());
    save(&mut harness, &output_dir, "filter-drop-shadow-hovered");
    harness.get_by_role_and_label(egui::Role::Button, "Color").click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-drop-shadow-color-picker");
    harness.key_press(egui::Key::Escape);
    harness.state_mut().session.target = StackTarget::Effect(effects[0].1);
    harness.run_steps(4);
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("filter-glow-{theme}"));
        }
    }

    // Baked Lighting Stylized on content, in a window tall enough for all its cards, then its Output list open
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    let params = json!({"layer": layer.clone(), "target": "content", "filter": "bakedLighting"});
    let lighting = LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap());
    let params = json!({"layer": layer.clone(), "target": "content", "filter": "bakedLightingEnvironment"});
    let environment = LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap());
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(lighting);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 2600.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(8);
    save(&mut harness, &output_dir, "filter-baked-lighting-stylized");
    let output = harness.get_by_role_and_label(egui::Role::ComboBox, "Diffuse + Specular").rect();
    harness.hover_at(output.center());
    harness.get_by_role_and_label(egui::Role::ComboBox, "Diffuse + Specular").click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-baked-lighting-stylized-output-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("filter-baked-lighting-stylized-{theme}"));
        }
        harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    }

    // Baked Lighting Environment, then hovering its stacked Specular Occlusion Intensity
    harness.state_mut().session.target = StackTarget::Effect(environment);
    harness.run_steps(8);
    save(&mut harness, &output_dir, "filter-baked-lighting-environment");
    let specular = harness.get_by_role_and_label(egui::Role::Slider, "Specular Occlusion Intensity").rect();
    harness.hover_at(specular.center());
    save(&mut harness, &output_dir, "filter-baked-lighting-environment-hovered");
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            save(&mut harness, &output_dir, &format!("filter-baked-lighting-environment-{theme}"));
        }
    }
}

#[test]
#[ignore = "writes the shape filters (screen 21) to TC_UI_SNAPSHOTS; run with --ignored"]
/// Screen 21: Mirror in each mode (its Mode list open), Mask Outline, Threshold (its Mode hovered)
fn shape_filter_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "mask.add.white", &json!({"layer": layer.clone()})).unwrap();
    let mut add = |filter: &str, settings: serde_json::Value| {
        let params = json!({"layer": layer.clone(), "target": "mask", "filter": filter});
        let effect = execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap();
        let mut settings = settings;
        settings["effect"] = json!(effect);
        execute(&mut session, "effect.set", &settings).unwrap();
        LayerId(effect)
    };
    let effects = [
        ("mirror-axis-x", add("mirror", json!({}))),
        ("mirror-axis-y", add("mirror", json!({"mode": "axisY"}))),
        ("mirror-corner", add("mirror", json!({"mode": "corner"}))),
        ("mask-outline", add("maskOutline", json!({}))),
        ("bevel", add("bevel", json!({}))),
        ("bevel-smooth", add("bevelSmooth", json!({}))),
        ("directional-distance", add("directionalDistance", json!({}))),
        ("threshold", add("threshold", json!({}))),
        ("warp", add("warp", json!({}))),
        ("warp-directional", add("warp", json!({"warpMode": "directional"}))),
        ("warp-multi-directional", add("warp", json!({"warpMode": "multiDirectional"}))),
        ("transform", add("transform", json!({"scaling": [2.0, 2.0], "offset": [0.25, 0.0], "rotation": 30.0}))),
        ("transform-plain", add("transform", json!({"repeatMode": "none", "scaling": [1.5, 1.5], "tileSafeRotation": false, "rotation": 20.0}))),
    ];
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(effects[0].1);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    for (name, effect) in effects {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(6);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    let shown = |name: &str| effects.iter().find(|(shown, _)| *shown == name).map(|(_, effect)| *effect).unwrap();
    harness.state_mut().session.target = StackTarget::Effect(shown("threshold"));
    harness.run_steps(4);
    let mode = harness.get_by_role_and_label(egui::Role::RadioButton, "≥").rect();
    harness.hover_at(mode.center());
    save(&mut harness, &output_dir, "filter-threshold-hovered");
    harness.state_mut().session.target = StackTarget::Effect(effects[0].1);
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Mirror Axis X").click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-mirror-mode-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);

    // Warp's mode list (under the filter card, also named Warp) and Multi Directional Warp's Blending Mode list, open
    harness.state_mut().session.target = StackTarget::Effect(shown("warp"));
    harness.run_steps(4);
    let lists = harness.query_all_by_role_and_label(egui::Role::ComboBox, "Warp");
    lists.max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-warp-mode-list");
    harness.key_press(egui::Key::Escape);
    harness.state_mut().session.target = StackTarget::Effect(shown("warp-multi-directional"));
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Average").click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-warp-blending-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);

    // Transform's Flip mode and Repeat mode lists, open with a choice hovered
    harness.state_mut().session.target = StackTarget::Effect(shown("transform"));
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "None").click();
    harness.run_steps(4);
    harness.hover_at(harness.get_by_label("Vertically").rect().center());
    harness.run_steps(2);
    save(&mut harness, &output_dir, "filter-transform-flip-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Horizontally & Vertically").click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-transform-repeat-list");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            for name in ["mirror-axis-x", "mask-outline", "bevel", "bevel-smooth", "warp-multi-directional", "transform"] {
                harness.state_mut().session.target = StackTarget::Effect(shown(name));
                harness.run_steps(6);
                save(&mut harness, &output_dir, &format!("filter-{name}-{theme}"));
            }
        }
    }
}

#[test]
#[ignore = "writes the simple filters (screen 23) to TC_UI_SNAPSHOTS; run with --ignored"]
/// Screen 23: Clamp, Smoothstep, Invert, Height Adjust, Histogram Scan, Histogram Shift, Sharpen and Posterize, one hovered switch
fn simple_filter_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "mask.add.white", &json!({"layer": layer.clone()})).unwrap();
    let mut add = |filter: &str| {
        let params = json!({"layer": layer.clone(), "target": "mask", "filter": filter});
        LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap())
    };
    let effects = [
        "clamp",
        "smoothstep",
        "invert",
        "heightAdjust",
        "histogramScan",
        "histogramShift",
        "sharpen",
        "posterize",
        "pixelate",
        "fxaa",
        "blur",
        "blurDirectional",
        "highPass",
        "grayscale",
        "anisotropicKuwahara",
    ]
    .map(|id| (id, add(id)));

    // PBR Validate goes on the layer content (its Color and Metal pills)
    let params = json!({"layer": layer.clone(), "target": "content", "filter": "pbrValidate"});
    let pbr = LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap());

    // Height To Normal too (its Normal pill), once with world units and once without
    let params = json!({"layer": layer.clone(), "target": "content", "filter": "heightToNormal"});
    let normals = [(); 2].map(|_| LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap()));
    execute(&mut session, "effect.set", &json!({"effect": normals[1].0, "useWorldUnits": false})).unwrap();

    // Color Match too (its Color pill): a new one, and one reading its Input and Image Input
    let params = json!({"layer": layer.clone(), "target": "content", "filter": "colorMatch"});
    let matches = [(); 2].map(|_| LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap()));
    execute(&mut session, "effect.set", &json!({"effect": matches[1].0, "sourceColorMode": "input", "targetColorMode": "imageInput"})).unwrap();

    // Replace Color too (its two colour bars)
    let params = json!({"layer": layer.clone(), "target": "content", "filter": "replaceColor"});
    let replace = LayerId(execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap());
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(effects[0].1);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    for (name, effect) in effects {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(6);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    let shown = |name: &str| effects.iter().find(|(shown, _)| *shown == name).map(|(_, effect)| *effect).unwrap();
    harness.state_mut().session.target = StackTarget::Effect(shown("posterize"));
    harness.run_steps(4);
    let alpha = harness.get_by_role_and_label(egui::Role::CheckBox, "Apply to Alpha").rect();
    harness.hover_at(alpha.center());
    save(&mut harness, &output_dir, "filter-posterize-hovered");
    harness.state_mut().session.target = StackTarget::Effect(shown("highPass"));
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Working space / Mask").click();
    harness.run_steps(2);
    harness.get_by_label("sRGB (from ACEScg)").hover();
    save(&mut harness, &output_dir, "filter-highPass-open");
    harness.key_press(egui::Key::Escape);
    let custom = json!({"effect": shown("grayscale").0, "channelInput": "customInput"});
    execute(&mut harness.state_mut().session, "effect.set", &custom).unwrap();
    harness.state_mut().session.target = StackTarget::Effect(shown("grayscale"));
    harness.run_steps(6);
    save(&mut harness, &output_dir, "filter-grayscale-custom");
    let custom = json!({"effect": shown("anisotropicKuwahara").0, "extractDirection": "customInput"});
    execute(&mut harness.state_mut().session, "effect.set", &custom).unwrap();
    harness.state_mut().session.target = StackTarget::Effect(shown("anisotropicKuwahara"));
    harness.run_steps(6);
    save(&mut harness, &output_dir, "filter-anisotropicKuwahara-custom");
    harness.state_mut().session.target = StackTarget::Effect(pbr);
    harness.run_steps(6);
    save(&mut harness, &output_dir, "filter-pbrValidate");
    let metal = harness.get_by_role_and_label(egui::Role::RadioButton, "70-100% Reflective").rect();
    harness.hover_at(metal.center());
    save(&mut harness, &output_dir, "filter-pbrValidate-hovered");
    for (effect, name) in normals.into_iter().zip(["heightToNormal", "heightToNormal-intensity"]) {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(6);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    let world = harness.get_by_role_and_label(egui::Role::CheckBox, "Use World Units").rect();
    harness.hover_at(world.center());
    save(&mut harness, &output_dir, "filter-heightToNormal-hovered");
    for (effect, name) in matches.into_iter().zip(["colorMatch", "colorMatch-inputs"]) {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(6);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    harness.state_mut().session.target = StackTarget::Effect(matches[0]);
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Average").click();
    harness.run_steps(2);
    harness.get_by_label("Input").hover();
    save(&mut harness, &output_dir, "filter-colorMatch-open");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.state_mut().session.target = StackTarget::Effect(replace);
    harness.run_steps(6);
    save(&mut harness, &output_dir, "filter-replaceColor");
    harness.get_by_role_and_label(egui::Role::Button, "Replacement color").click();
    harness.run_steps(4);
    save(&mut harness, &output_dir, "filter-replaceColor-picker");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            for name in ["heightAdjust", "histogramScan", "pixelate", "blurDirectional", "highPass", "grayscale", "anisotropicKuwahara"] {
                harness.state_mut().session.target = StackTarget::Effect(shown(name));
                harness.run_steps(6);
                save(&mut harness, &output_dir, &format!("filter-{name}-{theme}"));
            }
            harness.state_mut().session.target = StackTarget::Effect(pbr);
            harness.run_steps(6);
            save(&mut harness, &output_dir, &format!("filter-pbrValidate-{theme}"));
            harness.state_mut().session.target = StackTarget::Effect(normals[0]);
            harness.run_steps(6);
            save(&mut harness, &output_dir, &format!("filter-heightToNormal-{theme}"));
            harness.state_mut().session.target = StackTarget::Effect(matches[0]);
            harness.run_steps(6);
            save(&mut harness, &output_dir, &format!("filter-colorMatch-{theme}"));
        }
    }
}

#[test]
#[ignore = "writes the filter image inputs (screen 22) to TC_UI_SNAPSHOTS; run with --ignored"]
/// Screen 22: Bevel Smooth's Distance Map uniform, read from an anchor point and its picker open; Warp's Custom Noise from a pattern
fn image_input_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    let anchor = execute(&mut session, "effect.add.anchor", &json!({"name": "Cavity Dirt mask"})).unwrap()["effect"].clone();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "mask.add.white", &json!({"layer": layer.clone()})).unwrap();
    let mut add = |filter: &str, settings: serde_json::Value, input: serde_json::Value| {
        let params = json!({"layer": layer.clone(), "target": "mask", "filter": filter});
        let effect = execute(&mut session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap();
        let mut settings = settings;
        settings["effect"] = json!(effect);
        execute(&mut session, "effect.set", &settings).unwrap();
        if !input.is_null() {
            let mut input = input;
            input["effect"] = json!(effect);
            execute(&mut session, "layer.fill.set", &input).unwrap();
        }
        LayerId(effect)
    };
    let effects = [
        ("bevel-smooth-uniform", add("bevelSmooth", json!({}), serde_json::Value::Null)),
        ("bevel-smooth-anchor", add("bevelSmooth", json!({}), json!({"input": "distanceMap", "anchor": anchor}))),
        ("warp-custom-noise", add("warp", json!({"sourceMode": "customNoise"}), json!({"input": "customNoise", "procedural": "clouds"}))),
        ("blur-changed", add("blur", json!({"radius": 12.0, "wrap": true}), serde_json::Value::Null)),
    ];
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(effects[0].1);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 2200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    for (name, effect) in effects {
        harness.state_mut().session.target = StackTarget::Effect(effect);
        harness.run_steps(6);
        save(&mut harness, &output_dir, &format!("filter-{name}"));
    }
    harness.state_mut().session.target = StackTarget::Effect(effects[0].1);
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Distance Map").click();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "filter-distance-map-picker");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);

    // Blur with Attributes open, then its Parameters reset hovered (Restore defaults named over it)
    harness.state_mut().session.target = StackTarget::Effect(effects[3].1);
    harness.run_steps(4);
    harness.get_by_role_and_label(egui::Role::Button, "Attributes").click();
    harness.run_steps(6);
    save(&mut harness, &output_dir, "filter-attributes-open");
    harness.get_by_role_and_label(egui::Role::Button, "Attributes").click();
    harness.run_steps(4);
    let reset = harness.get_by_role_and_label(egui::Role::Button, "Restore defaults").rect().center();
    harness.set_render_cursor(false);
    harness.hover_at(reset);
    harness.run_steps(60);
    save(&mut harness, &output_dir, "filter-restore-defaults-hover");
    harness.set_render_cursor(true);
    harness.hover_at(egui::pos2(1.0, 1.0));
    harness.run_steps(4);
    if std::env::var_os("TC_UI_THEMES").is_some() {
        for (brightness, theme) in [(Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            harness.state_mut().session.target = StackTarget::Effect(effects[3].1);
            harness.run_steps(4);
            harness.get_by_role_and_label(egui::Role::Button, "Attributes").click();
            harness.run_steps(6);
            save(&mut harness, &output_dir, &format!("filter-attributes-open-{theme}"));
            harness.get_by_role_and_label(egui::Role::Button, "Attributes").click();
            harness.run_steps(4);
            for (name, effect) in effects {
                harness.state_mut().session.target = StackTarget::Effect(effect);
                harness.run_steps(6);
                save(&mut harness, &output_dir, &format!("filter-{name}-{theme}"));
            }
        }
    }
}

#[test]
#[ignore = "writes the filter picker states (screen 13) to TC_UI_SNAPSHOTS; run with --ignored"]
/// Screen 13: the filter picker opened from an empty card, swapping (current outlined), searching, and in a mask
fn filter_picker_snapshots() {
    use serde_json::json;
    use texturecraft_engine::core::LayerId;
    use texturecraft_engine::{StackTarget, execute};

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    let add = |session: &mut texturecraft_engine::Session, params: serde_json::Value| {
        LayerId(execute(session, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap())
    };
    let empty = add(&mut session, json!({"layer": layer, "target": "content", "filter": "none"}));
    let slope = add(&mut session, json!({"layer": layer, "target": "content", "filter": "blurSlope"}));
    execute(&mut session, "effect.set", &json!({"effect": slope.0, "channels": {"metallic": false, "normal": false}})).unwrap();
    let masked = add(&mut session, json!({"layer": layer, "target": "mask", "filter": "none"}));
    execute(&mut session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(empty);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);
    let open = |harness: &mut Harness<'_, TextureCraftApp>, card: &str, hover: &str| {
        harness.get_by_role_and_label(egui::Role::ComboBox, card).click();
        settle_thumbnails(harness);
        let tile = harness.get_by_role_and_label(egui::Role::ListBoxOption, hover).rect().center();
        harness.hover_at(tile);
    };
    open(&mut harness, "Filter", "Blur");
    save(&mut harness, &output_dir, "filter-picker-new");
    harness.event(egui::Event::Text("blur".into()));
    harness.hover_at(egui::pos2(1.0, 1.0));
    save(&mut harness, &output_dir, "filter-picker-search");
    harness.key_press(egui::Key::Escape);
    harness.state_mut().session.target = StackTarget::Effect(slope);
    harness.run_steps(4);
    open(&mut harness, "Blur Slope", "Clamp");
    save(&mut harness, &output_dir, "filter-picker-swapping");
    harness.key_press(egui::Key::Escape);
    harness.state_mut().session.target = StackTarget::Effect(masked);
    harness.run_steps(4);
    open(&mut harness, "Filter", "Bevel");
    save(&mut harness, &output_dir, "filter-picker-mask");
}

/// The Properties panel at its minimum width (the right column dragged as narrow as it goes), on the Fill tab's widest row (3D projection settings) and on a filter
#[test]
#[ignore]
fn properties_min_width_snapshot() {
    use craft_behavior::dock::Node;
    use craft_behavior::dock::SplitSize;
    use serde_json::json;
    use texturecraft_engine::execute;
    use texturecraft_ui_egui::docking::Dock;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    let settings = json!({"layer": layer, "projection": "planar", "scaleMode": "physicalSize", "backfaceCulling": true, "shapeCrop": "cropped"});
    execute(&mut session, "layer.fill.projection", &settings).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1500.0))
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    let workspace = harness.state().ui.dock.workspace().clone();
    let mut layout = workspace.layout().clone();
    let Some(Node::Split { second, .. }) = layout.root.as_mut() else { panic!("default columns") };
    let Node::Split { size, .. } = second.as_mut() else { panic!("viewport and right column") };
    *size = SplitSize::FixedSecond(0.0);
    let workspace = texturecraft_ui_egui::docking::Workspace::from_parts(layout, workspace.hidden().to_vec(), workspace.floated().to_vec());
    harness.state_mut().ui.dock = Dock::from_value(serde_json::to_value(workspace).unwrap());
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        save(&mut harness, &output_dir, &format!("properties-min-width-planar-{name}"));
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;
    execute(&mut harness.state_mut().session, "effect.add.filter", &json!({"target": "content", "filter": "colorMatch"})).unwrap();
    save(&mut harness, &output_dir, "properties-min-width-filter");
}

/// The docked Properties list of a fill whose sections all fit, scrolled to its end: MATERIAL at the top, blank space under it
#[test]
#[ignore]
fn sheet_scroll_end_snapshots() {
    use egui::{Event, Modifiers};
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    execute(&mut session, "layer.fill.projection", &json!({"layer": layer, "projection": "uv"})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 2000.0))
        .wgpu()
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.set_render_cursor(false);
    harness.run_steps(4);
    save(&mut harness, &output_dir, "sheet-end-top-medium");

    let strip = harness.get_by_role_and_label(egui::accesskit::Role::Tab, "Fill").rect();
    harness.hover_at(strip.center_bottom() + egui::vec2(0.0, 100.0));
    for _ in 0..40 {
        harness.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
    harness.run_steps(30);
    save(&mut harness, &output_dir, "sheet-end-scrolled-medium");
}
