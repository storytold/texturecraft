//! The compare mask in Properties: channel, the two sources with the operator between them, the constant, tolerance and hardness rows

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{Channel, CompareOp, CompareSource, EffectKind, LayerId};
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, FRAME_SECONDS, PROPERTIES_WINDOW, REDRAW_FRAMES, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// A fill over a fill, a compare mask on the top one, selected: (harness, compare effect)
fn compare_harness() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    execute(&mut session, "layer.add.fill", &json!({"roughness": 0.2})).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &json!({"roughness": 0.6})).unwrap()["layer"].clone();
    let effect = LayerId(execute(&mut session, "effect.add.compare", &json!({"layer": layer})).unwrap()["effect"].as_u64().unwrap());
    execute(&mut session, "layer.select", &json!({"layer": layer, "target": effect.0})).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).with_step_dt(FRAME_SECONDS).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    (harness, effect)
}

/// (left, op, right, channel, constant, tolerance, hardness) of compare mask `effect`
fn compare(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> (CompareSource, CompareOp, CompareSource, Channel, f32, f32, f32) {
    match harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Compare { left, op, right, channel, value, tolerance, hardness } => (left, op, right, channel, value, tolerance, hardness),
        ref other => panic!("a compare mask, found {other:?}"),
    }
}

fn undo_steps(harness: &Harness<'_, TextureCraftApp>) -> usize {
    harness.state().session.history_labels().len()
}

fn has_slider(harness: &Harness<'_, TextureCraftApp>, label: &str) -> bool {
    harness.query_by_role_and_label(Role::Slider, label).is_some()
}

/// Opens the list of the source showing `name` and picks `option`
fn pick_source(harness: &mut Harness<'_, TextureCraftApp>, name: &str, option: &str) {
    harness.get_by_role_and_label(Role::ComboBox, name).click();
    harness.run_steps(SETTLE_FRAMES);
    harness.get_by_role_and_label(Role::ListBoxOption, option).click();
    harness.run_steps(REDRAW_FRAMES);
}

#[test]
fn the_rows_show_and_edit_the_comparison_one_undo_step_each() {
    let (mut harness, effect) = compare_harness();
    assert!(crate::levels_editor::painted_text(&harness, "COMPARE MASK"), "the section caption");
    assert_eq!(compare(&harness, effect).0, CompareSource::ThisLayer);
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Layers below").is_some(), "the right source");
    assert!(has_slider(&harness, "Hardness") && !has_slider(&harness, "Constant") && !has_slider(&harness, "Tolerance"));

    let steps = undo_steps(&harness);
    pick_source(&mut harness, "Layers below", "Constant");

    assert_eq!(compare(&harness, effect).2, CompareSource::Constant);
    assert_eq!(undo_steps(&harness), steps + 1, "one undo step");
    assert!(has_slider(&harness, "Constant"), "a constant side shows its value");

    pick_source(&mut harness, "This layer", "This mask");

    assert_eq!(compare(&harness, effect).0, CompareSource::ThisMask);

    harness.get_by(|node| node.role() == Role::RadioButton && node.label().as_deref() == Some("≈")).click();
    harness.run_steps(REDRAW_FRAMES);

    assert_eq!(compare(&harness, effect).1, CompareOp::WithinTolerance);
    assert!(has_slider(&harness, "Tolerance"), "within tolerance shows its tolerance");

    pick_source(&mut harness, "Height", "Roughness");

    assert_eq!(compare(&harness, effect).3, Channel::Roughness);
    assert_eq!(undo_steps(&harness), steps + 4);
}

#[test]
fn a_source_list_leaves_the_view_its_wheel_and_closes_on_escape_or_a_click_outside() {
    let (mut harness, effect) = compare_harness();
    crate::assert_list_leaves_the_wheel_and_closes(&mut harness, |harness| {
        harness.get_by_role_and_label(Role::ComboBox, "Layers below").click();
        harness.run_steps(SETTLE_FRAMES);
        harness.get_by_role_and_label(Role::ComboBox, "Layers below").rect()
    });

    assert_eq!(compare(&harness, effect).2, CompareSource::LayersBelow, "nothing changed");
}
