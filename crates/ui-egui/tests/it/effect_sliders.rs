//! A selected effect's sliders apply what they are dragged, clicked or typed to (a generator's global balance stands in for them all)

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use serde_json::json;
use texturecraft_engine::core::{EffectKind, LayerId, WearParameters};
use texturecraft_ui_egui::TextureCraftApp;

/// The sample session with its first generator other than a wear one selected, and that effect's id
fn selected_generator() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let (layer, effect) = session
        .doc
        .project
        .layers
        .values()
        .find_map(|layer| {
            let mask_effects = layer.mask.iter().flat_map(|mask| mask.effects.iter());
            layer
                .content_effects
                .iter()
                .chain(mask_effects)
                .find(|effect| matches!(&effect.kind, EffectKind::Generator(generator) if WearParameters::defaults(generator.kind).is_none()))
                .map(|effect| (layer.id, effect.id))
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

/// The track of the slider labelled `label`
fn track(harness: &Harness<'_, TextureCraftApp>, label: &str) -> egui::Rect {
    harness.get_by_role_and_label(Role::Slider, label).rect()
}

/// The point `share` of the way along `track`
fn along(track: egui::Rect, share: f32) -> egui::Pos2 {
    egui::pos2(track.left() + track.width() * share, track.center().y)
}

#[test]
fn dragging_a_slider_applies_it() {
    let (mut harness, effect) = selected_generator();
    assert_eq!(balance(&harness, effect), 0.5);

    let track_rect = track(&harness, "Global Balance");
    harness.drag_at(along(track_rect, 0.98));
    harness.run_steps(2);
    harness.hover_at(along(track_rect, 0.08));
    harness.run_steps(2);
    harness.drop_at(along(track_rect, 0.08));
    harness.run_steps(3);

    assert!(balance(&harness, effect) < 0.25, "the dragged balance applies: {}", balance(&harness, effect));
}

/// A generator slider applies while it is dragged (the texture follows the pointer), and the whole drag is one undo step
#[test]
fn dragging_a_generator_slider_applies_live_as_one_undo_step() {
    let (mut harness, effect) = selected_generator();
    let level = |harness: &Harness<'_, TextureCraftApp>| match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Generator(generator) => generator.level,
        _ => panic!("a generator"),
    };
    let steps = harness.state().session.history_labels().len();
    let track_rect = track(&harness, "Level");
    let point_at = |share: f32| along(track_rect, share);
    harness.drag_at(point_at(0.05));
    harness.run_steps(2);
    let mut last = 0.0;
    for share in [0.3, 0.8] {
        harness.hover_at(point_at(share));
        harness.run_steps(2);

        assert!(level(&harness) > last && level(&harness) != 0.5, "the level follows the drag before it is released: {last} -> {}", level(&harness));

        last = level(&harness);
    }
    harness.drop_at(point_at(0.8));
    harness.run_steps(3);

    assert_eq!(level(&harness), last);
    assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step for the drag");
}

#[test]
fn clicking_a_slider_track_applies_it() {
    let (mut harness, effect) = selected_generator();
    let track_rect = track(&harness, "Global Balance");
    let at = along(track_rect, 0.08);
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run_steps(3);

    assert!(balance(&harness, effect) < 0.25, "a click on the track applies: {}", balance(&harness, effect));
}

/// The number field in the row of the slider labelled `label`
fn number_field<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &str) -> egui_kittest::Node<'a> {
    let row = track(harness, label).center().y;
    harness.get_all(by().role(Role::TextInput)).find(|node| (node.rect().center().y - row).abs() < 4.0).unwrap()
}

/// Click the balance's value to type in it and press `keys`, then type `text`, one key per frame like a person
fn type_balance(harness: &mut Harness<'_, TextureCraftApp>, keys: &[egui::Key], text: &str) {
    harness.get_by_label("Global Balance value").click();
    harness.run_steps(2);
    number_field(harness, "Global Balance").focus();
    harness.run_steps(2);
    for key in keys {
        harness.key_press(*key);
        harness.run_steps(1);
    }
    for character in text.chars() {
        number_field(harness, "Global Balance").type_text(&character.to_string());
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

/// Dragging the track while the value is being typed drops what was typed: leaving the field afterwards does not undo the drag
#[test]
fn a_track_drag_drops_text_typed_in_the_number_field() {
    let (mut harness, effect) = selected_generator();
    type_balance(&mut harness, &[egui::Key::End, egui::Key::Backspace], "5");
    let track_rect = track(&harness, "Global Balance");
    harness.drag_at(along(track_rect, 0.98));
    harness.run_steps(2);
    harness.hover_at(along(track_rect, 0.08));
    harness.run_steps(2);
    harness.drop_at(along(track_rect, 0.08));
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

/// Leaving the value without typing, or with Escape, keeps it (not the rounded text shown) and makes no undo step
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

    assert_eq!(number_field(&harness, "Global Balance").value().as_deref(), Some("0.39"));
    assert!(number_field(&harness, "Global Balance").is_focused());

    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert_eq!(balance(&harness, effect), 0.333);

    // Typing the value it already has is no edit either
    type_balance(&mut harness, &[egui::Key::End, egui::Key::Backspace, egui::Key::Backspace, egui::Key::Backspace, egui::Key::Backspace], "0.333");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);

    assert_eq!(harness.state().session.history_labels().len(), steps);
}
