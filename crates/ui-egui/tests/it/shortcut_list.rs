//! The shortcut list uses the reference's action names and default keys; the menus keep their own texts.

use egui::{Event, Key, Modifiers, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_ui_egui::keymap::{NOT_AVAILABLE_ACTIONS, bindings};
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
fn the_settings_page_disables_editing_shortcuts_for_unavailable_actions() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Shortcuts").click();
    h.run_steps(4);
    for (_, row) in &NOT_AVAILABLE_ACTIONS {
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
