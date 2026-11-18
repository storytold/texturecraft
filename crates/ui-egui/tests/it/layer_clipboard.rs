//! Ctrl+C, Ctrl+X and Ctrl+V on a layer clicked in the stack. The desktop and web shells deliver
//! these keys as clipboard events, not key presses, so the tests send those events.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::LayerId;
use texturecraft_ui_egui::TextureCraftApp;

/// The sample session with its selected layer clicked in the stack, and that layer.
fn harness() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let layer = h.state().session.selected_layer().unwrap();
    let name = h.state().session.doc.project.layers[&layer].name.clone();
    h.get_by_label(&name).click();
    h.run_steps(3);
    (h, layer)
}

fn count(h: &Harness<'_, TextureCraftApp>) -> usize {
    h.state().session.doc.project.layers.len()
}

#[test]
fn ctrl_x_cuts_the_clicked_layer() {
    let (mut h, layer) = harness();
    h.event(egui::Event::Cut);
    h.run_steps(3);
    assert!(!h.state().session.doc.project.layers.contains_key(&layer), "Ctrl+X removes the layer");
    assert!(h.state().session.clipboard.is_some(), "and keeps it on the layer clipboard");
}

/// Windows reports Shift+Delete and Shift+Insert as Cut and Paste with Shift held.
#[test]
fn shift_delete_cuts_and_shift_insert_pastes() {
    let (mut h, layer) = harness();
    let before = count(&h);
    h.event_modifiers(egui::Event::Cut, egui::Modifiers::SHIFT);
    h.run_steps(3);
    assert!(!h.state().session.doc.project.layers.contains_key(&layer), "Shift+Delete cuts the layer");
    h.event_modifiers(egui::Event::Paste("Layer".into()), egui::Modifiers::SHIFT);
    h.run_steps(3);
    assert_eq!(count(&h), before, "Shift+Insert pastes it back");
}

#[test]
fn a_binding_on_shift_insert_runs_instead_of_the_paste() {
    let (mut h, _) = harness();
    let before = count(&h);
    h.event(egui::Event::Copy);
    h.run_steps(3);
    assert!(h.state().session.clipboard.is_some(), "there is something to paste");
    h.state_mut().ui.keymap.insert("ui.layout.3d".into(), "Shift+Insert".into());
    h.run_steps(2);
    h.event_modifiers(egui::Event::Paste("Layer".into()), egui::Modifiers::SHIFT);
    h.run_steps(3);
    assert_eq!(h.state().ui.layout, texturecraft_ui_egui::state::ViewLayout::Only3D, "the shortcut ran");
    assert_eq!(count(&h), before, "and nothing was pasted");
}

#[test]
fn ctrl_c_then_ctrl_v_pastes_a_copy() {
    let (mut h, _) = harness();
    let before = count(&h);
    h.event(egui::Event::Copy);
    h.run_steps(3);
    assert!(h.state().session.clipboard.is_some(), "Ctrl+C copies the layer");
    // The shell sends the system clipboard's text with the paste; Ctrl+C put the layer name there.
    h.event(egui::Event::Paste("Layer".into()));
    h.run_steps(3);
    assert!(count(&h) > before, "Ctrl+V pastes the layer");
}

#[test]
fn layer_shortcuts_act_on_a_selected_effect() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let id = |v: serde_json::Value, key: &str| LayerId(v[key].as_u64().unwrap());
    let layer = id(texturecraft_engine::execute(&mut session, "layer.add.fill", &serde_json::Value::Null).unwrap(), "layer");
    let p = serde_json::json!({"target": "content", "filter": "invert"});
    let effect = id(texturecraft_engine::execute(&mut session, "effect.add.filter", &p).unwrap(), "effect");
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let effects = |h: &Harness<'_, TextureCraftApp>| h.state().session.doc.project.layers.get(&layer).map(|l| l.content_effects.len());
    h.event(egui::Event::Copy);
    h.run_steps(3);
    assert_eq!(h.state().session.clipboard.as_ref().map(|(_, id)| *id), Some(effect), "Ctrl+C copies the selected effect");
    h.event(egui::Event::Paste("Fill".into()));
    h.run_steps(3);
    assert_eq!(effects(&h), Some(2), "Ctrl+V pastes it above");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(3);
    assert_eq!(effects(&h), Some(3), "Ctrl+D duplicates the effect");
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(effects(&h), Some(2), "Delete removes the effect, not its layer");
}

fn instances_of(h: &Harness<'_, TextureCraftApp>, source: LayerId) -> usize {
    h.state().session.doc.project.layers.values().filter(|l| l.instance_source() == Some(source)).count()
}

/// Ctrl+Shift+V pastes the copied layer as an instance, and is greyed with nothing copied.
#[test]
fn ctrl_shift_v_pastes_an_instance() {
    let (mut h, layer) = harness();
    let before = count(&h);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::V);
    h.run_steps(3);
    assert_eq!(count(&h), before, "nothing is copied, so the key does nothing");
    assert!(!texturecraft_ui_egui::menus::enabled(h.state(), "layer.pasteInstance"));
    h.event(egui::Event::Copy);
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::V);
    h.run_steps(3);
    assert_eq!(count(&h), before + 1, "one new layer");
    assert_eq!(instances_of(&h, layer), 1, "and it is an instance of the copied layer");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert_eq!(count(&h), before, "undo removes the instance");
}

/// The layer menu names the key and greys the item until a layer is copied; the key can be rebound.
#[test]
fn the_layer_menu_shows_the_paste_instance_key() {
    let (mut h, layer) = harness();
    let name = h.state().session.doc.project.layers[&layer].name.clone();
    h.get_by_label(&name).click_secondary();
    h.run_steps(3);
    let item = h.get_by_label("Paste as instance Ctrl+Shift+V");
    assert!(item.accesskit_node().is_disabled(), "nothing copied yet");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.event(egui::Event::Copy);
    h.run_steps(3);
    h.get_by_label(&name).click_secondary();
    h.run_steps(3);
    assert!(!h.get_by_label("Paste as instance Ctrl+Shift+V").accesskit_node().is_disabled(), "a copied layer enables it");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    let before = count(&h);
    h.state_mut().ui.keymap.insert("layer.pasteInstance".into(), "Ctrl+Alt+V".into());
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::V);
    h.run_steps(3);
    assert_eq!(count(&h), before, "the old key is free after rebinding");
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::V);
    h.run_steps(3);
    assert_eq!(instances_of(&h, layer), 1, "the new key pastes the instance");
}
