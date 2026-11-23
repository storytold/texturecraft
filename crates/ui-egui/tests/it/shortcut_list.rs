//! The shortcut list uses the reference's action names and default keys; the menus keep their own texts.

use egui::{Event, Key, Modifiers, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_ui_egui::keymap::{self, NOT_AVAILABLE_ACTIONS, bindings, shortcut_of};
use texturecraft_ui_egui::state::Mode;
use texturecraft_ui_egui::{TextureCraftApp, menus};

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'_, TextureCraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.run_steps(3);
}

#[test]
fn the_list_starts_with_the_reference_names_and_keys_in_the_reference_order() {
    let expected = [
        ("Hide UI", "Tab"),
        ("Undo", "Ctrl+Z"),
        ("Redo", "Ctrl+Y"),
        ("Open project", "Ctrl+O"),
        ("Close project", "Ctrl+F4"),
        ("Save project", "Ctrl+S"),
        ("Save as project", "Ctrl+Shift+S"),
        ("New project", "Ctrl+N"),
        ("Quit application", "Ctrl+Q"),
        ("Export Textures...", "Ctrl+Shift+E"),
        ("Reimport mesh", "Ctrl+Shift+R"),
        ("Switch to baking mode", "Ctrl+Shift+B"),
        ("Switch to edition mode", "F9"),
        ("Switch to rendering mode (path traced)", "F10"),
        ("Show mesh wireframe", ""),
        ("Camera projection: Perspective view", "F5"),
        ("Camera projection: Orthographic view", "F6"),
        ("Display next channel", "C"),
        ("Display previous channel", "Shift+C"),
        ("Display material", "M"),
        ("Display next mesh map", "B"),
        ("Display previous mesh map", "Shift+B"),
        ("Show entire mesh/path", "F"),
        ("Select next camera", ">"),
        ("Select previous camera", "<"),
        ("Enable quick mask", "Y"),
        ("Edit quick mask", "U"),
        ("Invert quick mask", "Ctrl+Shift+I"),
        ("Viewers layout: 3D/2D", "F1"),
        ("Viewers layout: 3D only", "F2"),
        ("Viewers layout: 2D only", "F3"),
        ("Viewers layout: Swap 3D/2D", "F4"),
    ];
    let actual: Vec<(&str, &str)> = bindings().iter().take(expected.len()).map(|b| (b.label, keymap::shown(b.default.unwrap_or("")))).collect();
    assert_eq!(actual, expected);
}

#[test]
fn no_two_default_shortcuts_are_the_same() {
    let mut seen = std::collections::BTreeMap::new();
    for b in bindings() {
        if let Some(key) = b.default {
            assert_eq!(seen.insert(key, b.id), None, "{key} is the default of {} and another item", b.id);
        }
    }
    for (_, row) in &NOT_AVAILABLE_ACTIONS {
        let key = row.default.unwrap();
        assert!(!seen.contains_key(key), "{} shows {key}, which another item has", row.label);
    }
}

#[test]
fn the_menus_keep_their_texts_and_quit_keeps_ctrl_q() {
    let label = |id: &str| menus::CATALOG.iter().find(|item| item.id == id).map(|item| item.label);
    assert_eq!(label("file.open"), Some("Open..."));
    assert_eq!(label("file.saveAs"), Some("Save as..."));
    assert_eq!(label("ui.quit"), Some("Quit"));
    assert_eq!(label("ui.mode.bake"), Some("Bake Mesh Maps"));
    assert_eq!(label("viewport.frame"), Some("Show entire mesh"));
    assert_eq!(shortcut_of(&keymap::Keymap::new(), "ui.quit"), Some("Ctrl+Q"));
    assert_eq!(shortcut_of(&keymap::Keymap::new(), "ui.mode.bake"), Some("Ctrl+Shift+B"));
}

#[test]
fn the_settings_page_lists_the_reference_names_and_shows_the_missing_functions_disabled() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Shortcuts").click();
    h.run_steps(4);
    for name in ["Open project", "Save as project", "Quit application", "Switch to baking mode", "Show entire mesh/path", "Viewers layout: Swap 3D/2D"] {
        assert!(h.query_by_label(name).is_some(), "{name} is listed");
    }
    assert!(h.query_by_label("Open...").is_none(), "the menu's text is not the list's");
    for (_, row) in &NOT_AVAILABLE_ACTIONS {
        let action = row.label;
        h.get_by_label(action).scroll_to_me();
        h.run_steps(3);
        assert!(h.get_by_label(&format!("Edit the shortcut of {action}")).accesskit_node().is_disabled(), "{action} has no function yet");
    }
}

#[test]
fn the_listed_keys_run_their_commands() {
    let mut h = harness();
    press(&mut h, Key::B, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(h.state().ui.mode, Mode::Baking, "Ctrl+Shift+B switches to baking mode");
    press(&mut h, Key::F9, Modifiers::NONE);
    assert_eq!(h.state().ui.mode, Mode::Painting, "F9 switches to edition mode");
    press(&mut h, Key::F10, Modifiers::NONE);
    assert_eq!(h.state().ui.mode, Mode::Rendering);
    press(&mut h, Key::F9, Modifiers::NONE);

    // Show mesh wireframe has no key until one is given
    assert!(!h.state().ui.mesh_wireframe);
    h.state_mut().ui.keymap.insert("ui.wireframe".into(), "F7".into());
    h.run_steps(2);
    press(&mut h, Key::F7, Modifiers::NONE);
    assert!(h.state().ui.mesh_wireframe, "the wireframe is shown");
    press(&mut h, Key::F7, Modifiers::NONE);
    assert!(!h.state().ui.mesh_wireframe, "and hidden again");
}

#[test]
fn greater_and_less_select_the_next_and_previous_camera() {
    let mut h = harness();
    let session = &mut h.state_mut().session;
    texturecraft_engine::execute(session, "camera.add", &json!({"name": "A"})).unwrap();
    let first = (session.camera.yaw, session.camera.pitch);
    texturecraft_engine::execute(session, "camera.snapRotate.left", &json!({})).unwrap();
    texturecraft_engine::execute(session, "camera.add", &json!({"name": "B"})).unwrap();
    let second = (session.camera.yaw, session.camera.pitch);
    assert_ne!(first, second);
    h.run_steps(2);

    // The last saved camera is shown: the next one is the first
    press(&mut h, Key::Period, Modifiers::SHIFT);
    let angles = |h: &Harness<'_, TextureCraftApp>| (h.state().session.camera.yaw, h.state().session.camera.pitch);
    assert_eq!(angles(&h), first, ">");
    press(&mut h, Key::Comma, Modifiers::SHIFT);
    assert_eq!(angles(&h), second, "<");
}

#[test]
fn a_key_listed_for_a_missing_function_is_reported_as_shared() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Shortcuts").click();
    h.run_steps(4);
    h.state_mut().ui.windows.settings_draft.as_mut().unwrap().keymap.insert("ui.hideUi".into(), "Space".into());
    h.run_steps(4);
    assert!(h.query_by_label_contains("One or several errors").is_some(), "Space is Toggle animation's");
    assert!(h.get_by_label("OK").accesskit_node().is_disabled());
}

#[test]
fn the_shortcut_window_shows_the_reference_keys_and_the_missing_functions() {
    let mut h = harness();
    h.state_mut().ui.windows.shortcuts = true;
    h.run_steps(4);
    assert!(h.query_by_label("Pause Engine Computation").is_some() && h.query_by_label("Toggle animation").is_some());
    assert!(h.query_all_by_label(">").next().is_some() && h.query_all_by_label("<").next().is_some(), "the camera keys read as the reference's");
    assert!(h.query_by_label("Shift+Period").is_none());
}
