//! The layer menu's Add bitmap mask, Copy mask, Paste into mask and Flatten all instances, and
//! the Ctrl+Alt+C / Ctrl+Alt+V shortcuts.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, LayerId, LayerKind};
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

fn id(v: Value, key: &str) -> u64 {
    v[key].as_u64().unwrap()
}

/// "Plain" (no mask) above "Masked" (a black mask with a generator), "Wear.png" imported.
fn harness() -> (Harness<'static, TextureCraftApp>, LayerId, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    session.import_resource_bytes("Wear.png", bytes).unwrap();
    let masked = LayerId(id(execute(&mut session, "layer.add.fill", &json!({"color": "#336699"})).unwrap(), "layer"));
    execute(&mut session, "layer.rename", &json!({"layer": masked.0, "name": "Masked"})).unwrap();
    execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    execute(&mut session, "effect.add.generator", &json!({"generator": "position", "name": "Effect M"})).unwrap();
    let plain = LayerId(id(execute(&mut session, "layer.add.fill", &json!({"color": "#993333"})).unwrap(), "layer"));
    execute(&mut session, "layer.rename", &json!({"layer": plain.0, "name": "Plain"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1200.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    (h, masked, plain)
}

fn right_click_layer(h: &mut Harness<'_, TextureCraftApp>, name: &str) {
    let at = h.get_by_label(name).rect().center();
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(3);
}

fn pick(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn mask_of(h: &Harness<'_, TextureCraftApp>, layer: LayerId) -> Option<texturecraft_engine::core::Mask> {
    h.state().session.doc.project.layer(layer).unwrap().mask.clone()
}

#[test]
fn add_bitmap_mask_lists_the_image_resources_and_adds_a_mask() {
    let (mut h, _, plain) = harness();
    right_click_layer(&mut h, "Plain");
    pick(&mut h, "Add bitmap mask");
    pick(&mut h, "Wear.png");
    let mask = mask_of(&h, plain).expect("the layer has a mask");
    assert!(matches!(&mask.effects[0].kind, EffectKind::Fill { .. }));
    h.state_mut().session.undo().unwrap();
    assert!(mask_of(&h, plain).is_none(), "one undo takes the mask away");
}

#[test]
fn add_bitmap_mask_is_off_on_a_layer_that_has_a_mask() {
    let (mut h, _, _) = harness();
    right_click_layer(&mut h, "Masked");
    assert!(h.get_by_label("Add bitmap mask").accesskit_node().is_disabled());
}

#[test]
fn the_layer_menu_copies_a_mask_and_pastes_it_into_another_layer() {
    let (mut h, masked, plain) = harness();
    right_click_layer(&mut h, "Plain");
    assert!(h.get_by_label("Paste into mask Ctrl+Alt+V").accesskit_node().is_disabled(), "nothing copied yet");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    right_click_layer(&mut h, "Masked");
    pick(&mut h, "Copy mask Ctrl+Alt+C");
    assert!(h.state().session.mask_clipboard.is_some());
    right_click_layer(&mut h, "Plain");
    pick(&mut h, "Paste into mask Ctrl+Alt+V");
    assert_eq!(mask_of(&h, plain).map(|m| m.effects.len()), mask_of(&h, masked).map(|m| m.effects.len()));
    h.state_mut().session.undo().unwrap();
    assert!(mask_of(&h, plain).is_none());
}

#[test]
fn ctrl_alt_c_and_ctrl_alt_v_copy_and_paste_the_mask_of_the_selected_layer() {
    let (mut h, masked, plain) = harness();
    execute(&mut h.state_mut().session, "layer.select", &json!({"layer": masked.0})).unwrap();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::C);
    h.run_steps(3);
    assert!(h.state().session.mask_clipboard.is_some(), "Ctrl+Alt+C copies the mask");
    execute(&mut h.state_mut().session, "layer.select", &json!({"layer": plain.0})).unwrap();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::V);
    h.run_steps(3);
    assert!(mask_of(&h, plain).is_some(), "Ctrl+Alt+V pastes it");
    assert!(h.state().session.clipboard.is_none(), "the layer clipboard is untouched");
}

#[test]
fn the_shortcuts_can_be_rebound() {
    let (mut h, masked, _) = harness();
    h.state_mut().ui.keymap.insert("mask.copy".into(), "Ctrl+Alt+K".into());
    execute(&mut h.state_mut().session, "layer.select", &json!({"layer": masked.0})).unwrap();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::C);
    h.run_steps(3);
    assert!(h.state().session.mask_clipboard.is_none(), "the old chord no longer copies");
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::K);
    h.run_steps(3);
    assert!(h.state().session.mask_clipboard.is_some(), "the new one does");
}

#[test]
fn flatten_all_instances_in_the_layer_menu_flattens_the_layer_and_its_instances_at_once() {
    let (mut h, masked, plain) = harness();
    let instance = LayerId(id(execute(&mut h.state_mut().session, "layer.instance", &json!({"layer": masked.0})).unwrap(), "layer"));
    execute(&mut h.state_mut().session, "layer.rename", &json!({"layer": instance.0, "name": "Linked"})).unwrap();
    h.run_steps(3);
    let steps = h.state().session.history_labels().len();

    // A layer with no instances has nothing to flatten across
    right_click_layer(&mut h, "Plain");
    assert!(h.get_by_label("Flatten all instances").accesskit_node().is_disabled());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    right_click_layer(&mut h, "Masked");
    pick(&mut h, "Flatten all instances");
    let kind = |h: &Harness<'_, TextureCraftApp>, l: LayerId| h.state().session.doc.project.layer(l).unwrap().kind.clone();
    assert!(matches!(kind(&h, masked), LayerKind::Paint) && matches!(kind(&h, instance), LayerKind::Paint));
    assert!(matches!(kind(&h, plain), LayerKind::Fill(_)));
    assert_eq!(h.state().session.history_labels().len(), steps + 1, "one undo step");
}
