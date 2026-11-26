//! Effect rows in the layer stack: dragging one reorders its stack, its blend mode and opacity sit at its right, and its menu holds the Blending options

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

use super::layers_panel::click_selected_content_thumbnail;

fn id(v: Value, key: &str) -> u64 {
    v[key].as_u64().unwrap()
}

/// The blending the rows show and set: the displayed channel's (base color by default)
fn base_color_blend(effect: &texturecraft_engine::core::Effect) -> texturecraft_engine::core::ChannelBlend {
    effect.blend_of(Some(texturecraft_engine::core::Channel::BaseColor))
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
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    click_selected_content_thumbnail(&mut harness);
    let order =
        |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.layers[&layer].content_effects.iter().map(|e| e.id.0).collect::<Vec<_>>();
    assert_eq!(order(&harness), [a, b, c]);

    // Rows show C, B, A; dropping A above C moves it to the top
    drag(&mut harness, "Effect A", "Effect C", false);

    assert_eq!(order(&harness), [b, c, a]);

    // Dropping A below B returns it to the bottom
    drag(&mut harness, "Effect A", "Effect B", true);

    assert_eq!(order(&harness), [a, b, c]);

    // Dropping C above A places it between A and B
    drag(&mut harness, "Effect C", "Effect A", false);

    assert_eq!(order(&harness), [a, c, b]);
}

/// An effect row's blend mode menu and opacity field sit at its right and set the displayed channel's blending, as on layer rows
#[test]
fn an_effect_row_sets_its_blend_mode_and_opacity() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &Value::Null).unwrap(), "layer"));
    let effect = id(execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": "Effect A"})).unwrap(), "effect");
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    click_selected_content_thumbnail(&mut harness);
    let effect_of = |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.layers[&layer].content_effects[0].clone();

    assert_eq!(effect, effect_of(&harness).id.0);

    harness.state_mut().ui.layer_material_view = false;
    harness.run_steps(2);

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

    assert_eq!(base_color_blend(&effect_of(&harness)).mode, texturecraft_engine::core::BlendMode::Multiply, "the row sets the displayed channel's");

    let roughness = effect_of(&harness).blend_of(Some(texturecraft_engine::core::Channel::Roughness)).mode;
    assert_eq!(roughness, texturecraft_engine::core::BlendMode::Normal, "the other channels keep theirs");

    let history = harness.state().session.history_labels().len();
    let opacity_at = egui::pos2(pair_right - 20.0, row.center().y);
    harness.drag_at(opacity_at);
    harness.run_steps(2);
    harness.hover_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(2);
    harness.drop_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(3);
    let opacity = base_color_blend(&effect_of(&harness)).opacity;
    assert!(opacity < 0.9, "dragging the field to the left lowers the opacity: {opacity}");
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
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    click_selected_content_thumbnail(&mut harness);
    let effect_of = |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.layers[&layer].content_effects[0].clone();

    let history = harness.state().session.history_labels().len();
    let opacity_at = harness.get_by_label("Opacity of Effect A").rect().center();

    // Equal right and down movement adds nothing to the value: the drag starts without a change
    harness.drag_at(opacity_at);
    harness.run_steps(2);
    harness.hover_at(opacity_at + egui::vec2(8.0, 8.0));
    harness.run_steps(2);

    assert_eq!(base_color_blend(&effect_of(&harness)).opacity, 1.0, "the drag started without changing the opacity");

    harness.hover_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(2);
    harness.drop_at(opacity_at - egui::vec2(100.0, 0.0));
    harness.run_steps(3);
    let opacity = base_color_blend(&effect_of(&harness)).opacity;
    assert!(opacity < 0.9, "the drag lowered the opacity: {opacity}");
    assert_eq!(harness.state().session.history_labels().len(), history + 1, "the drag is one new undo step");

    harness.state_mut().session.undo().unwrap();

    assert_eq!(base_color_blend(&effect_of(&harness)).opacity, 1.0);
    assert_eq!(base_color_blend(&effect_of(&harness)).mode, texturecraft_engine::core::BlendMode::Multiply, "undo leaves the earlier blend change");
}

/// An effect row's menu holds Blending options: the channel the Layers panel shows lends its blend mode to every channel (none in the Material view), and Paste waits for a Copy
#[test]
fn an_effect_rows_blending_options_reach_every_channel() {
    use texturecraft_engine::core::{BlendMode, Channel};

    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = LayerId(id(execute(&mut session, "layer.add.fill", &Value::Null).unwrap(), "layer"));
    let effect = id(execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert", "name": "Effect A"})).unwrap(), "effect");
    execute(&mut session, "effect.set", &json!({"effect": effect, "channel": "baseColor", "blend": "Multiply"})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer.0})).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1400.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.run_steps(4);
    click_selected_content_thumbnail(&mut harness);
    let effect_of = |harness: &Harness<'_, TextureCraftApp>| harness.state().session.doc.project.layers[&layer].content_effects[0].clone();

    assert_eq!(effect_of(&harness).blend_of(Some(Channel::Roughness)).mode, BlendMode::Normal);

    let open_blending_options = |harness: &mut Harness<'_, TextureCraftApp>| {
        harness.get_by_label("Effect A").click_secondary();
        harness.run_steps(3);
        harness.get_by_label("Blending options").click();
        harness.run_steps(3);
    };
    open_blending_options(&mut harness);

    assert!(harness.get_by_label("Use this blend mode on every channel").accesskit_node().is_disabled(), "the Material view shows no channel to lend");
    assert!(harness.get_by_label("Paste channel blending").accesskit_node().is_disabled(), "nothing is copied yet");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    // With Base color shown, the row reads its Multiply and lends it to every channel
    harness.state_mut().ui.layer_material_view = false;
    harness.run_steps(3);
    open_blending_options(&mut harness);
    harness.get_by_label("Use this blend mode on every channel").click();
    harness.run_steps(3);

    assert_eq!(effect_of(&harness).blend_of(Some(Channel::Roughness)).mode, BlendMode::Multiply);

    open_blending_options(&mut harness);
    harness.get_by_label("Copy channel blending").click();
    harness.run_steps(3);
    open_blending_options(&mut harness);

    assert!(!harness.get_by_label("Paste channel blending").accesskit_node().is_disabled(), "a copy can be pasted");
}
