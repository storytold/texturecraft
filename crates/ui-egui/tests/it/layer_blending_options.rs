//! Layer menu ▸ Blending options: apply the displayed channel's blending to all channels, copy and paste it between layers.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{BlendMode, Channel, ChannelBlend, LayerId};
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

fn blending(h: &Harness<'_, TextureCraftApp>, layer: LayerId) -> Vec<(Channel, ChannelBlend)> {
    h.state().session.doc.project.layers[&layer].channels.iter().map(|(c, b)| (*c, *b)).collect()
}

/// Gives `layer` a distinct blending on base color and, when it has it, roughness.
fn set_distinct(h: &mut Harness<'_, TextureCraftApp>, layer: LayerId) {
    let s = &mut h.state_mut().session;
    texturecraft_engine::execute(s, "layer.blend", &json!({"layer": layer.0, "channel": "baseColor", "mode": "Multiply", "opacity": 0.4})).unwrap();
    texturecraft_engine::execute(s, "layer.blend", &json!({"layer": layer.0, "channel": "roughness", "mode": "Overlay", "opacity": 0.7})).unwrap();
}

fn menu_item(h: &mut Harness<'_, TextureCraftApp>, layer: LayerId, item: &str) {
    let name = h.state().session.doc.project.layers[&layer].name.clone();
    h.get_by_label(&name).click_secondary();
    h.run_steps(3);
    h.get_all_by_label("Blending options").last().unwrap().click();
    h.run_steps(3);
    h.get_all_by_label(item).last().unwrap().click();
    h.run_steps(3);
}

fn first_layer(h: &Harness<'_, TextureCraftApp>) -> LayerId {
    h.state().session.selected_layer().unwrap()
}

#[test]
fn apply_both_copies_mode_and_opacity_to_every_channel_in_one_undo_step() {
    let mut h = harness();
    let layer = first_layer(&h);
    set_distinct(&mut h, layer);
    let before = blending(&h, layer);
    assert!(before.len() > 2, "the layer has several channels");
    menu_item(&mut h, layer, "Apply both to all channels");
    for (c, b) in blending(&h, layer) {
        assert_eq!((b.mode, b.opacity), (BlendMode::Multiply, 0.4), "{c:?}");
    }
    texturecraft_engine::execute(&mut h.state_mut().session, "edit.undo", &json!({})).unwrap();
    assert_eq!(blending(&h, layer), before, "one undo restores every channel");
}

#[test]
fn apply_mode_leaves_opacity_and_apply_opacity_leaves_mode() {
    let mut h = harness();
    let layer = first_layer(&h);
    set_distinct(&mut h, layer);
    menu_item(&mut h, layer, "Apply blending mode to all channels");
    let after = blending(&h, layer);
    assert!(after.iter().all(|(_, b)| b.mode == BlendMode::Multiply));
    assert_eq!(after.iter().find(|(c, _)| *c == Channel::Roughness).unwrap().1.opacity, 0.7, "opacity stays");
    set_distinct(&mut h, layer);
    menu_item(&mut h, layer, "Apply opacity to all channels");
    let after = blending(&h, layer);
    assert!(after.iter().all(|(_, b)| b.opacity == 0.4));
    assert_eq!(after.iter().find(|(c, _)| *c == Channel::Roughness).unwrap().1.mode, BlendMode::Overlay, "mode stays");
}

#[test]
fn copy_from_one_layer_and_paste_into_another() {
    let mut h = harness();
    let a = first_layer(&h);
    set_distinct(&mut h, a);
    let b = LayerId(texturecraft_engine::execute(&mut h.state_mut().session, "layer.add.paint", &json!({"name": "Other"})).unwrap()["layer"].as_u64().unwrap());
    h.run_steps(3);
    assert_ne!(blending(&h, a), blending(&h, b));
    menu_item(&mut h, a, "Copy channel blending settings");
    menu_item(&mut h, b, "Paste channel blending settings");
    assert_eq!(blending(&h, b), blending(&h, a), "B has A's blending on every channel");
    texturecraft_engine::execute(&mut h.state_mut().session, "edit.undo", &json!({})).unwrap();
    assert_ne!(blending(&h, b), blending(&h, a), "pasting was one undo step");
}

#[test]
fn paste_is_disabled_until_something_is_copied() {
    let mut h = harness();
    let layer = first_layer(&h);
    let name = h.state().session.doc.project.layers[&layer].name.clone();
    h.get_by_label(&name).click_secondary();
    h.run_steps(3);
    h.get_all_by_label("Blending options").last().unwrap().click();
    h.run_steps(3);
    assert!(h.get_all_by_label("Paste channel blending settings").last().unwrap().accesskit_node().is_disabled(), "nothing copied yet");
    assert!(h.state().ui.blend_clipboard.is_none());
}

#[test]
fn paste_keeps_a_disabled_channel_disabled() {
    let mut h = harness();
    let a = first_layer(&h);
    set_distinct(&mut h, a);
    let b = LayerId(texturecraft_engine::execute(&mut h.state_mut().session, "layer.add.paint", &json!({"name": "Other"})).unwrap()["layer"].as_u64().unwrap());
    texturecraft_engine::execute(&mut h.state_mut().session, "layer.blend", &json!({"layer": b.0, "channel": "roughness", "enabled": false})).unwrap();
    h.run_steps(3);
    menu_item(&mut h, a, "Copy channel blending settings");
    menu_item(&mut h, b, "Paste channel blending settings");
    assert!(!blending(&h, b).iter().any(|(c, _)| *c == Channel::Roughness));
    assert_eq!(blending(&h, b).iter().find(|(c, _)| *c == Channel::BaseColor).unwrap().1.mode, BlendMode::Multiply);
}
