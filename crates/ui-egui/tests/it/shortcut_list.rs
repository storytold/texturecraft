//! The shortcut list uses the reference's action names and default keys; the menus keep their own texts.

use egui::{Event, Key, Modifiers, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_ui_egui::keymap::{NOT_AVAILABLE_ACTIONS, REFERENCE_ACTIONS, bindings};
use texturecraft_ui_egui::{TextureCraftApp, menus};

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'_, TextureCraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.run_steps(3);
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
        if let Some(key) = row.default {
            assert_eq!(seen.insert(key, row.id), None, "{key} is reserved for {} and another item", row.label);
        }
    }
}

#[test]
fn the_settings_page_disables_editing_shortcuts_for_unavailable_actions() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Shortcuts").click();
    h.run_steps(4);
    for row in NOT_AVAILABLE_ACTIONS.iter().map(|(_, row)| row).chain(&REFERENCE_ACTIONS) {
        let action = row.label;
        h.get_by_label(action).scroll_to_me();
        h.run_steps(3);
        assert!(h.get_by_label(&format!("Edit the shortcut of {action}")).accesskit_node().is_disabled(), "{action} has no function yet");
    }
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
fn recording_shift_escape_warns_that_the_key_is_reserved() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Shortcuts").click();
    h.run_steps(4);
    h.get_by_label("Edit the shortcut of Save project").click();
    h.run_steps(4);

    press(&mut h, Key::Escape, Modifiers::SHIFT);

    assert!(h.query_by_label_contains("One or several errors").is_some(), "Shift+Escape is reserved for Pause Engine Computation");
    assert!(h.get_by_label("OK").accesskit_node().is_disabled());
}

#[test]
fn a_saved_shortcut_on_a_reference_key_does_not_block_settings() {
    let mut h = harness();
    h.state_mut().ui.keymap.insert("ui.hideUi".into(), "Q".into());

    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
    h.get_by_label("OK").click();
    h.run_steps(3);

    assert!(!h.state().ui.windows.settings, "an existing shortcut must not prevent confirming Settings");
    assert_eq!(h.state().ui.keymap.get("ui.hideUi").map(String::as_str), Some("Q"));
}

#[test]
fn the_standalone_shortcut_editor_lists_the_reference_keys() {
    let mut harness = harness();
    let context = harness.ctx.clone();
    let windows = texturecraft_ui_egui::state::Windows { shortcuts: true, ..Default::default() };

    let reply = texturecraft_ui_egui::control::handle(harness.state_mut(), &context, "ui.set", &json!({"windows": windows}));
    harness.run_steps(4);

    assert_eq!(reply["ok"], true);
    assert!(harness.query_by_label("Toggle transform handles").is_some());
    assert!(harness.get_by_role_and_label(Role::Button, "Q").accesskit_node().is_disabled());
}

#[test]
fn the_manipulator_rows_follow_export_textures_in_the_reference_order() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Shortcuts").click();
    h.run_steps(4);

    let rows = [
        "Export Textures...",
        "Toggle transform handles",
        "Enlarge transform handles",
        "Shrink transform handles",
        "Switch transform coordinate space",
        "Toggle warp editing",
        "Move with transform handles",
        "Rotate with transform handles",
        "Reimport mesh",
    ];
    let tops: Vec<f32> = rows.iter().map(|label| h.get_by_label(label).rect().top()).collect();

    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{rows:?} run top to bottom: {tops:?}");
}
