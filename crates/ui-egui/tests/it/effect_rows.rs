//! Effect rows in the layer stack: dragging one reorders its stack, and its blend mode and opacity sit at its right.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

fn id(v: Value, key: &str) -> u64 {
    v[key].as_u64().unwrap()
}

/// Drag the row labelled `from` onto the upper (`below` false) or lower half of the row `to`.
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: &str, to: &str, below: bool) {
    let start = harness.get_by_label(from).rect().center();
    let row_rect = harness.get_by_label(to).rect();
    let end = egui::pos2(row_rect.center().x, if below { row_rect.bottom() - 4.0 } else { row_rect.top() + 4.0 });
    harness.drag_at(start);
    harness.run_steps(2);
    harness.hover_at(end);
    harness.run_steps(2);
    harness.drop_at(end);
    harness.run_steps(3);
}

#[test]
fn dragging_an_effect_row_reorders_its_stack() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &Value::Null).unwrap(), "layer"));
    let mut add =
        |name: &str| id(execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": name})).unwrap(), "effect");
    let (a, b, c) = (add("Effect A"), add("Effect B"), add("Effect C"));
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    let order =
        |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.layers[&layer].content_effects.iter().map(|e| e.id.0).collect::<Vec<_>>();
    assert_eq!(order(&harness), [a, b, c]);
    // The rows list the top first: C, B, A. A dropped on C's upper half goes to the top.
    drag(&mut harness, "Effect A", "Effect C", false);
    assert_eq!(order(&harness), [b, c, a]);
    // A dropped on B's lower half goes back to the bottom.
    drag(&mut harness, "Effect A", "Effect B", true);
    assert_eq!(order(&harness), [a, b, c]);
    // C dropped on A's upper half lands between A and B.
    drag(&mut harness, "Effect C", "Effect A", false);
    assert_eq!(order(&harness), [a, c, b]);
}

/// An effect row's blend mode menu and opacity field sit at its right, as on layer rows.
#[test]
fn an_effect_row_sets_its_blend_mode_and_opacity() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &Value::Null).unwrap(), "layer"));
    let effect = id(execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": "Effect A"})).unwrap(), "effect");
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    let effect_of = |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.layers[&layer].content_effects[0].clone();
    assert_eq!(effect, effect_of(&harness).id.0);

    // From the row's right edge: a 4-point gap, then the 82-point pair of mode menu and opacity field
    let row = harness.get_by_label("Effect A").rect();
    let pair_right = row.right() - 4.0;
    let pair_left = pair_right - 82.0;
    let mode_at = egui::pos2(pair_left + 20.0, row.center().y);
    harness.drag_at(mode_at);
    harness.drop_at(mode_at);
    harness.run_steps(2);
    harness.get_by_label("Multiply").click();
    harness.run_steps(3);
    assert_eq!(effect_of(&harness).blend, texturecraft_engine::core::BlendMode::Multiply);

    let history = harness.state().session.history_labels().len();
    let opacity_at = egui::pos2(pair_right - 20.0, row.center().y);
    harness.drag_at(opacity_at);
    harness.run_steps(2);
    harness.hover_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(2);
    harness.drop_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(3);
    assert!(effect_of(&harness).opacity < 0.9, "dragging the field to the left lowers the opacity: {}", effect_of(&harness).opacity);
    assert_eq!(harness.state().session.history_labels().len(), history + 1, "the whole drag is one undo step");
}

/// A drag whose first frame leaves the opacity unchanged still starts its own undo step: undo keeps the blend mode set before it
#[test]
fn an_opacity_drag_starting_without_a_change_is_its_own_undo_step() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &Value::Null).unwrap(), "layer"));
    let effect = id(execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": "Effect A"})).unwrap(), "effect");
    execute(&mut session, "effect.set", &json!({"effect": effect, "blend": "Multiply"})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    let effect_of = |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.layers[&layer].content_effects[0].clone();

    let history = harness.state().session.history_labels().len();
    let opacity_at = harness.get_by_label("Opacity of Effect A").rect().center();
    // Equal right and down movement adds nothing to the value: the drag starts without a change
    harness.drag_at(opacity_at);
    harness.run_steps(2);
    harness.hover_at(opacity_at + egui::vec2(8.0, 8.0));
    harness.run_steps(2);
    assert_eq!(effect_of(&harness).opacity, 1.0, "the drag started without changing the opacity");
    harness.hover_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(2);
    harness.drop_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(3);
    assert!(effect_of(&harness).opacity < 0.9, "the drag lowered the opacity: {}", effect_of(&harness).opacity);
    assert_eq!(harness.state().session.history_labels().len(), history + 1, "the drag is one new undo step");
    harness.state_mut().session.undo().unwrap();
    assert_eq!(effect_of(&harness).opacity, 1.0);
    assert_eq!(effect_of(&harness).blend, texturecraft_engine::core::BlendMode::Multiply, "undo leaves the earlier blend change");
}
