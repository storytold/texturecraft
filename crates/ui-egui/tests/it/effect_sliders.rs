//! A selected effect's sliders apply what they are dragged, clicked or typed to (a generator's global balance stands in for them all).

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use serde_json::json;
use texturecraft_engine::core::{EffectKind, LayerId};
use texturecraft_ui_egui::TextureCraftApp;

/// The sample session with its first generator effect selected, and that effect's id.
fn selected_generator() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let (layer, effect) = session
        .doc
        .project
        .layers
        .values()
        .find_map(|label_rect| {
            let mask = label_rect.mask.iter().flat_map(|m| m.effects.iter());
            label_rect.content_effects.iter().chain(mask).find(|e| matches!(e.kind, EffectKind::Generator(_))).map(|e| (label_rect.id, e.id))
        })
        .unwrap();
    texturecraft_engine::execute(&mut session, "layer.select", &json!({"layer": layer.0, "target": effect.0})).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    (harness, effect)
}

fn balance(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> f32 {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Generator(generator) => generator.global.balance,
        _ => panic!("a generator"),
    }
}

/// The track under the slider labelled `label` (the row below the label).
fn track(harness: &Harness<'_, TextureCraftApp>, label: &str) -> egui::Rect {
    let label_rect = harness.get_by_label(label).rect();
    egui::Rect::from_min_max(egui::pos2(label_rect.left() + 6.0, label_rect.bottom() + 6.0), egui::pos2(label_rect.left() + 200.0, label_rect.bottom() + 14.0))
}

#[test]
fn dragging_a_slider_applies_it() {
    let (mut harness, effect) = selected_generator();
    assert_eq!(balance(&harness, effect), 0.5);
    let track_rect = track(&harness, "Global balance");
    harness.drag_at(egui::pos2(track_rect.right(), track_rect.center().y));
    harness.run_steps(2);
    harness.hover_at(egui::pos2(track_rect.left() + 20.0, track_rect.center().y));
    harness.run_steps(2);
    harness.drop_at(egui::pos2(track_rect.left() + 20.0, track_rect.center().y));
    harness.run_steps(3);
    assert!(balance(&harness, effect) < 0.25, "the dragged balance applies: {}", balance(&harness, effect));
}

/// A generator slider applies while it is dragged (the texture follows the pointer), and the
/// whole drag is one undo step.
#[test]
fn dragging_a_generator_slider_applies_live_as_one_undo_step() {
    let (mut harness, effect) = selected_generator();
    let level = |harness: &Harness<'_, TextureCraftApp>| match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Generator(g) => g.level,
        _ => panic!("a generator"),
    };
    let steps = harness.state().session.history_labels().len();
    let track_rect = track(&harness, "Level");
    let at = |x: f32| egui::pos2(track_rect.left() + x, track_rect.center().y);
    harness.drag_at(at(10.0));
    harness.run_steps(2);
    let mut last = 0.0;
    for x in [60.0, 160.0] {
        harness.hover_at(at(x));
        harness.run_steps(2);
        assert!(level(&harness) > last && level(&harness) != 0.5, "the level follows the drag before it is released: {last} -> {}", level(&harness));
        last = level(&harness);
    }
    harness.drop_at(at(160.0));
    harness.run_steps(3);
    assert_eq!(level(&harness), last);
    assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step for the drag");
}

/// The combo box in the row of the levels editor's Reset button.
fn channel_picker<'a>(harness: &'a Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'a> {
    let row = harness.get_by_label("Reset").rect().center().y;
    harness.get_all(by().role(Role::ComboBox)).find(|node| (node.rect().center().y - row).abs() < 4.0).unwrap()
}

/// A content levels effect offers a channel for its histogram; picking one keeps it selected.
#[test]
fn content_levels_pick_the_histogram_channel() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"color": [0.5, 0.2, 0.1], "roughness": 0.6})).unwrap();
    texturecraft_engine::execute(&mut session, "effect.add.levels", &json!({"target": "content"})).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    assert_eq!(channel_picker(&harness).value().as_deref(), Some("Base color"), "the layer's first channel");
    channel_picker(&harness).click();
    harness.run_steps(2);
    harness.get_by_label("Roughness").click();
    harness.run_steps(3);
    assert_eq!(channel_picker(&harness).value().as_deref(), Some("Roughness"), "the picked channel stays");
}

#[test]
fn clicking_a_slider_track_applies_it() {
    let (mut harness, effect) = selected_generator();
    let track_rect = track(&harness, "Global balance");
    let at = egui::pos2(track_rect.left() + 20.0, track_rect.center().y);
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run_steps(3);
    assert!(balance(&harness, effect) < 0.25, "a click on the track applies: {}", balance(&harness, effect));
}

/// The number field in the row of the slider labelled `label`.
fn number_field<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &str) -> egui_kittest::Node<'a> {
    let row = harness.get_by_label(label).rect().center().y;
    harness.get_all(by().role(Role::TextInput)).find(|n| (n.rect().center().y - row).abs() < 4.0).unwrap()
}

/// Focus the balance field and press `keys`, then type `text`, one key per frame like a person.
fn type_balance(harness: &mut Harness<'_, TextureCraftApp>, keys: &[egui::Key], text: &str) {
    number_field(harness, "Global balance").focus();
    harness.run_steps(2);
    for key in keys {
        harness.key_press(*key);
        harness.run_steps(1);
    }
    for character in text.chars() {
        number_field(harness, "Global balance").type_text(&character.to_string());
        harness.run_steps(1);
    }
}

#[test]
fn a_value_typed_in_the_number_field_applies() {
    let (mut harness, effect) = selected_generator();
    let steps = harness.state().session.history_labels().len();
    type_balance(&mut harness, &[egui::Key::End, egui::Key::Backspace, egui::Key::Backspace, egui::Key::Backspace, egui::Key::Backspace], "0.25");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(balance(&harness, effect), 0.25);
    assert_eq!(harness.state().session.history_labels().len(), steps + 1);
}

/// Dragging the track while the number field has focus drops what was typed there: leaving the
/// field afterwards does not undo the drag.
#[test]
fn a_track_drag_drops_text_typed_in_the_number_field() {
    let (mut harness, effect) = selected_generator();
    type_balance(&mut harness, &[egui::Key::End, egui::Key::Backspace], "5");
    let track_rect = track(&harness, "Global balance");
    harness.drag_at(egui::pos2(track_rect.right(), track_rect.center().y));
    harness.run_steps(2);
    harness.hover_at(egui::pos2(track_rect.left() + 20.0, track_rect.center().y));
    harness.run_steps(2);
    harness.drop_at(egui::pos2(track_rect.left() + 20.0, track_rect.center().y));
    harness.run_steps(2);
    let dragged = balance(&harness, effect);
    assert!(dragged < 0.25, "the drag applies: {dragged}");
    // Leave whatever has focus: a click on empty space
    harness.drag_at(egui::pos2(5.0, 1390.0));
    harness.drop_at(egui::pos2(5.0, 1390.0));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(balance(&harness, effect), dragged, "the text typed before the drag stays dropped");
}

/// Leaving the number field without typing, or with Escape, keeps the value (not the rounded
/// text the field shows) and makes no undo step.
#[test]
fn leaving_the_number_field_unchanged_keeps_the_value() {
    let (mut harness, effect) = selected_generator();
    texturecraft_engine::execute(&mut harness.state_mut().session, "effect.set", &json!({"effect": effect.0, "balance": 0.333})).unwrap();
    harness.run_steps(2);
    let steps = harness.state().session.history_labels().len();
    type_balance(&mut harness, &[], "");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(balance(&harness, effect), 0.333, "left without typing");
    type_balance(&mut harness, &[egui::Key::End, egui::Key::Backspace], "9");
    assert_eq!(number_field(&harness, "Global balance").value().as_deref(), Some("0.39"));
    assert!(number_field(&harness, "Global balance").is_focused());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert_eq!(balance(&harness, effect), 0.333);
    // Typing the value it already has is no edit either
    type_balance(&mut harness, &[egui::Key::End, egui::Key::Backspace, egui::Key::Backspace, egui::Key::Backspace, egui::Key::Backspace], "0.333");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().session.history_labels().len(), steps);
}
