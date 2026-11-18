//! Offscreen UI snapshots at 1600x960, except the taller mesh-map panel
//! Requires a GPU adapter
//!
//! Run with TC_UI_SNAPSHOTS=<dir> cargo test -p texturecraft-ui-egui --release --test ui_snapshots -- --ignored

use std::path::{Path, PathBuf};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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

fn save(harness: &mut Harness<'_, TextureCraftApp>, dir: &Path, name: &str) {
    harness.run_steps(6);
    let img = harness.render().unwrap_or_else(|e| panic!("{name}: {e}"));
    let path = dir.join(format!("{name}.png"));
    img.save(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

#[test]
#[ignore]
fn empty_start_snapshot() {
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::Session::closed().unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.prefs.language = "en".into();

    save(&mut harness, &output_dir, "empty-start");
}

/// Closing rejects a same-frame capture, then releases both rendered view targets.
#[test]
#[ignore]
fn close_releases_view_targets_and_rejects_immediate_capture() {
    let session = texturecraft_engine::Session::closed().unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(960.0, 640.0)).wgpu().build_eframe(|context| TextureCraftApp::new(context, session));
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
    let mut harness =
        Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_eframe(|context| TextureCraftApp::new(context, session));
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
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    harness.get_by_label("Output Size").scroll_to_me();
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
    harness.state_mut().session.bake_meshes.high.clear();
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
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|context| {
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
    use craft_ui::docking::Node;
    use craft_ui::layout::SplitSize;
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
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 1600.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    let mut workspace = harness.state().ui.dock.workspace().clone();
    let Some(Node::Split { second, .. }) = workspace.layout.root.as_mut() else { panic!("default columns") };
    let Node::Split { size, .. } = second.as_mut() else { panic!("viewport and right column") };
    *size = SplitSize::Ratio(0.99);
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
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.dock.focus(Panel::Properties);
    harness.run_steps(6);
    harness.get_by_label("Micro detail").scroll_to_me();
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
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
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
        let mut harness =
            Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1100.0)).wgpu().build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(6);

    harness.get_by_value("Uniform value").click();
    harness.run_steps(3);
    save(&mut harness, &output_dir, "mask-fill-image-menu");

    harness.get_by_role_and_label(egui::Role::Button, &name).click();
    harness.run_steps(4);

    let EffectKind::Fill { source, .. } = &harness.state().session.doc.project.effect(LayerId(effect)).unwrap().1.kind else { panic!("fill effect") };
    assert_eq!(*source, Source::image(name));
    assert!(harness.query_by_label("Projection").is_some(), "image projection is editable");
    save(&mut harness, &output_dir, "mask-fill-image-selected");
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn tile_generator_gradient_map_snapshot() {
    use serde_json::json;
    use texturecraft_engine::execute;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    execute(
        &mut session,
        "layer.fill.set",
        &json!({"channel": "baseColor", "procedural": "Tile Generator", "tile": {"amount": [5, 3]}, "colors": ["#1d1745", "#4e80a0", "#f0bb75"]}),
    )
    .unwrap();

    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).wgpu().build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.dock.focus(Panel::Properties);
    harness.run_steps(6);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Base color").scroll_to_me();
    harness.run_steps(30);

    assert!(harness.query_by_label_contains("Add color").is_some());
    save(&mut harness, &output_dir, "tile-generator-gradient-map");
}

/// Edit > Settings: the General page, then the Shortcuts and Libraries pages
#[test]
#[ignore]
fn settings_pages_snapshot() {
    use texturecraft_ui_egui::settings_window::SettingsPage;

    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&output_dir).unwrap();
    let session = texturecraft_engine::sample_session(128, 32).unwrap();
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
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
}
