//! The rail's side panels as in the reference: Undock and Dock, the Export panel and its two exports, the Log's channel
//! tabs, the Display and Shader Settings rows, the shader choice and the section tabs that follow the scrolling

use egui::{Event, Modifiers, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::file_dialogs::Answer;
use texturecraft_ui_egui::state::UiState;
use texturecraft_ui_egui::unavailable::NOT_AVAILABLE;

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// The rail is the last 38 points of the window
const RAIL_LEFT_EDGE: f32 = WINDOW_SIZE.x - 38.0;

/// Frames for the first layout and fonts to settle, and for a click to land
const STARTUP_FRAMES: usize = 4;
const SETTLE_FRAMES: usize = 3;

/// Frames for a tooltip to appear under a resting pointer
const TOOLTIP_FRAMES: usize = 30;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// The rail icon of the panel named `name`
fn rail_icon<'h>(harness: &'h Harness<'_, TextureCraftApp>, name: &str) -> Option<egui_kittest::Node<'h>> {
    let is_panel = |label: &str| label == name || label.starts_with(&format!("{name} ("));
    harness
        .get_all_by_role(Role::Button)
        .find(|button| button.rect().min.x >= RAIL_LEFT_EDGE && button.accesskit_node().label().is_some_and(|label| is_panel(&label)))
}

fn click_rail(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    rail_icon(harness, name).expect("the panel's icon on the rail").click();
    harness.run_steps(SETTLE_FRAMES);
}

/// The button labelled `label` left of the rail (the rail has its own Export icon)
fn button_left_of_rail<'h>(harness: &'h Harness<'_, TextureCraftApp>, label: &str) -> egui_kittest::Node<'h> {
    harness
        .get_all_by_role(Role::Button)
        .find(|button| button.rect().max.x < RAIL_LEFT_EDGE && button.accesskit_node().label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("the {label} button"))
}

fn is_floating(harness: &Harness<'_, TextureCraftApp>, panel: Panel) -> bool {
    harness.state().ui.dock.workspace().layout.floating.iter().any(|window| window.contains(&panel))
}

/// Hover the control labelled `label` until its tooltip shows, and tell whether that says it is not available yet
fn says_not_available(harness: &mut Harness<'_, TextureCraftApp>, label: &str) -> bool {
    let at = harness.get_all_by_label(label).next().unwrap_or_else(|| panic!("{label}")).rect().center();
    harness.hover_at(at);
    harness.run_steps(TOOLTIP_FRAMES);
    harness.query_by_label(NOT_AVAILABLE).is_some()
}

fn is_on(node: &egui_kittest::Node<'_>) -> bool {
    node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn undock_floats_the_panel_dock_returns_it_to_the_rail_and_the_float_persists() {
    let mut harness = app();
    click_rail(&mut harness, "History");

    button_left_of_rail(&harness, "Undock").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().ui.dock.is_visible(Panel::History) && is_floating(&harness, Panel::History), "History floats on its own");
    assert!(harness.state().ui.rail_panel.is_none(), "the popup closed");
    assert!(rail_icon(&harness, "History").is_none(), "a floating panel leaves the rail");

    // The floating window is part of the saved interface state
    let saved = serde_json::to_value(&harness.state().ui).unwrap();
    let restored: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.dock, harness.state().ui.dock);
    assert!(restored.dock.workspace().layout.floating.iter().any(|window| window.contains(&Panel::History)));

    button_left_of_rail(&harness, "Dock").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(!harness.state().ui.dock.is_open(Panel::History), "Dock closes the floating window");
    assert!(rail_icon(&harness, "History").is_some(), "History is back on the rail");
    click_rail(&mut harness, "History");
    assert_eq!(harness.state().ui.rail_panel, Some(Panel::History), "and opens from it again");
}

#[test]
fn the_export_panel_leads_to_export_textures_and_the_mesh_options() {
    let mut harness = app();
    click_rail(&mut harness, "Export");

    assert_eq!(harness.state().ui.rail_panel, Some(Panel::Export));
    assert!(!harness.state().ui.windows.export, "the rail opens the Export panel, not a window");
    for label in ["SEND TO", "EXPORT", "Export to image editor", "Send to compositing app", "Send to 3D staging app"] {
        assert!(harness.query_all_by_label(label).next().is_some(), "{label}");
    }
    assert!(says_not_available(&mut harness, "Send to compositing app"));

    button_left_of_rail(&harness, "Export Textures...").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().ui.windows.export, "Export Textures... opens the Export textures window");
    for label in
        ["SETTINGS", "OUTPUT TEMPLATES", "LIST OF EXPORTS", "Include all", "Global settings", "General Export Parameters", "Reset all", "Save settings"]
    {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    assert!(harness.query_by_label_contains("textures selected for export").is_some());
    assert!(says_not_available(&mut harness, "Export USD asset"));

    harness.get_by_label("Save settings").click();
    harness.run_steps(SETTLE_FRAMES);
    assert!(!harness.state().ui.windows.export, "Save settings closes the window");

    click_rail(&mut harness, "Export");
    button_left_of_rail(&harness, "Export Mesh...").click();
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.query_by_label("Mesh Export Options").is_some(), "Export Mesh... asks for the options first");
}

#[test]
fn export_mesh_asks_the_options_then_the_file() {
    let mut harness = app();
    let dir = std::env::temp_dir().join(format!("texturecraft-side-panels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mesh.obj");
    let _ = std::fs::remove_file(&file);
    let answer = file.clone();
    harness.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![answer.clone()]))));

    let ctx = harness.ctx.clone();
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &ctx, "file.exportMesh");
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_label("Mesh Export Options").is_some());
    assert_eq!(harness.state().file_dialogs.pending(), 0, "no file is asked for before the options");
    for label in ["Without displacement/tessellation", "With displacement/tessellation", "Apply triangulation", "Recompute vertex normals"] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    assert!(says_not_available(&mut harness, "With displacement/tessellation"));

    button_left_of_rail(&harness, "Export").click();
    harness.run_steps(SETTLE_FRAMES * 3);

    assert!(harness.query_by_label("Mesh Export Options").is_none(), "Export closes the options");
    assert!(file.exists(), "the mesh went to the picked file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether the Log lists `line` (the status bar under it repeats the last message)
fn log_lists(harness: &Harness<'_, TextureCraftApp>, line: &str) -> bool {
    harness.query_all_by_label(line).any(|node| node.rect().bottom() < STATUS_BAR_TOP)
}

/// The status bar is the window's bottom 22 points
const STATUS_BAR_TOP: f32 = WINDOW_SIZE.y - 22.0;

#[test]
fn the_log_tabs_show_one_channel_each() {
    let mut harness = app();
    harness.state_mut().session.log.push("Saved /somewhere/project.tcraft".into());
    harness.state_mut().session.log.push("resource stone.png: not an image".into());
    click_rail(&mut harness, "Log");

    assert!(harness.query_by_label("General").is_some());
    assert!(log_lists(&harness, "Saved /somewhere/project.tcraft"));
    assert!(!log_lists(&harness, "resource stone.png: not an image"), "asset messages wait under Assets");

    // The Log's own Assets tab, right of the dock's Assets panel tab
    harness.get_all_by_label("Assets").max_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(log_lists(&harness, "resource stone.png: not an image"));
    assert!(!log_lists(&harness, "Saved /somewhere/project.tcraft"));
}

#[test]
fn display_settings_show_the_reference_rows() {
    let mut harness = app();
    harness.state_mut().ui.dock.focus(Panel::Display);
    harness.run_steps(STARTUP_FRAMES);

    for label in [
        "View mode",
        "ENVIRONMENT SETTINGS",
        "Environment",
        "Environment Map",
        "Override environment map color space",
        "Environment Opacity",
        "Environment Exposure (EV)",
        "Environment Blur",
        "Environment Rotation",
        "Environment Alignment",
        "Shadows",
        "Computation mode",
        "Shadows opacity",
        "CAMERA SETTINGS",
        "Preset",
        "Restore",
    ] {
        assert!(harness.query_all_by_label(label).next().is_some(), "{label}");
    }

    let combo_value = |harness: &Harness<'_, TextureCraftApp>, value: &str| {
        harness.get_all_by_role(Role::ComboBox).any(|combo| combo.accesskit_node().value().as_deref() == Some(value))
    };
    for value in ["Auto", "World", "Lightweight", "Default camera"] {
        assert!(combo_value(&harness, value), "a dropdown shows {value}");
    }
    assert!(!harness.state().session.doc.project.display.shadows, "shadows start off");
    assert!(says_not_available(&mut harness, "Override environment map color space"));
}

#[test]
fn shader_settings_show_the_reference_rows_and_the_shader_choice() {
    let mut harness = app();
    harness.state_mut().ui.dock.focus(Panel::Shader);
    harness.run_steps(STARTUP_FRAMES);

    for label in [
        "Undo",
        "Redo",
        "open-pbr",
        "Instance name",
        "SHADER PARAMETERS",
        "Base Surface",
        "AO Intensity",
        "Specular",
        "Enable anisotropy",
        "Index of refraction",
        "Enable specular color",
        "Transmission",
        "Enable transmission",
        "Subsurface",
        "Subsurface radius",
        "Subsurface scatter anisotropy",
        "Use thickness map",
    ] {
        assert!(harness.query_all_by_label(label).next().is_some(), "{label}");
    }
    assert_eq!(harness.state().session.doc.project.display.ior, 1.5, "the reference's index of refraction");
    assert!(says_not_available(&mut harness, "Enable anisotropy"));

    harness.get_by_role_and_label(Role::Button, "open-pbr").click();
    harness.run_steps(SETTLE_FRAMES);

    let tiles: Vec<String> =
        texturecraft_ui_egui::shader_settings::SHADER_TYPES.iter().map(|name| format!("{}\n{name}", egui_phosphor::regular::SPHERE)).collect();
    for name in &tiles {
        assert!(harness.query_all_by_label(name).next().is_some(), "{name:?} is listed");
    }
    assert_eq!(harness.query_all_by_label(&tiles[7]).count(), 6, "six entries read pbr-m...");
    let query = harness
        .get_all_by_role(Role::TextInput)
        .any(|field| field.accesskit_node().value().as_deref() == Some(texturecraft_ui_egui::shader_settings::SHADER_QUERY));
    assert!(query, "the search starts at shaders");
    let open_pbr = harness.get_by_label(&tiles[4]);
    assert!(!open_pbr.accesskit_node().is_disabled() && is_on(&open_pbr), "open-pbr is the current shader");
    assert!(harness.get_by_label(&tiles[1]).accesskit_node().is_disabled(), "the others are not available yet");
}

#[test]
fn scrolling_highlights_the_section_tab_and_a_tab_jumps_to_its_section() {
    let mut harness = app();
    click_rail(&mut harness, "Display Settings");
    let tab = |harness: &Harness<'_, TextureCraftApp>, name: &str| is_on(&harness.get_by_role_and_label(Role::Button, name));

    assert!(tab(&harness, "Environment settings"), "the first section is highlighted at the top");

    harness.get_by_role_and_label(Role::Button, "Viewer settings").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(tab(&harness, "Viewer settings") && !tab(&harness, "Environment settings"), "the clicked tab is highlighted");
    let heading = harness.get_by_label("VIEWER SETTINGS").rect();
    assert!(harness.ctx.content_rect().contains_rect(heading), "the page scrolled to Viewer settings");

    // Scroll back up a little: the Camera or Post effects heading is the one passed last
    let over_page = heading.center();
    harness.hover_at(over_page);
    for _ in 0..40 {
        harness.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 60.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        harness.run_steps(1);
    }
    harness.run_steps(12);

    assert!(tab(&harness, "Environment settings"), "back at the top, Environment settings is highlighted again");
    assert!(!tab(&harness, "Viewer settings"));
}
