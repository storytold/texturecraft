//! The filter slot in Properties swaps a filter through its picker, and Add filter adds one

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, Filter, LayerId};
use texturecraft_engine::{StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

/// The app with an Invert filter on a fill layer's content, selected so Properties shows its slot
fn app_with_invert() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    let effect = execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert"})).unwrap();
    let effect = LayerId(effect["effect"].as_u64().unwrap());
    execute(&mut session, "layer.select", &json!({"layer": layer["layer"]})).unwrap();
    session.target = StackTarget::Effect(effect);

    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    (harness, effect)
}

/// The kind of effect `effect` now
fn kind(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> EffectKind {
    harness.state().session.doc.project.effect(effect).unwrap().1.kind.clone()
}

/// Clicks the Invert slot, which opens its picker
fn open_picker(harness: &mut Harness<'_, TextureCraftApp>) {
    harness.get_by_role_and_label(Role::ComboBox, "Invert").click();
    harness.run_steps(3);
    assert!(egui::Popup::is_any_open(&harness.ctx), "clicking the slot opens the picker");
}

#[test]
fn picking_from_the_slot_swaps_the_filter() {
    let (mut harness, effect) = app_with_invert();
    open_picker(&mut harness);

    // The picker's search field has the focus as it opens
    harness.event(egui::Event::Text("sharp".into()));
    harness.run_steps(3);
    assert_eq!(harness.query_all_by_label("Blur").count(), 0, "the search hides filters that don't match");

    harness.get_by_label("Sharpen").click();
    harness.run_steps(3);

    assert!(matches!(kind(&harness, effect), EffectKind::Filter(Filter::Sharpen { .. })), "{:?}", kind(&harness, effect));
    assert!(!egui::Popup::is_any_open(&harness.ctx), "a pick closes the picker");
}

#[test]
fn escape_closes_the_picker_without_changes() {
    let (mut harness, effect) = app_with_invert();
    open_picker(&mut harness);

    // Without the search field's focus, the painting shortcuts see Escape first
    harness.ctx.memory_mut(|memory| memory.stop_text_input());
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert!(!egui::Popup::is_any_open(&harness.ctx));
    assert!(matches!(kind(&harness, effect), EffectKind::Filter(Filter::Invert)));
}

#[test]
fn add_filter_adds_a_filter_and_selects_it() {
    let (mut harness, _) = app_with_invert();
    harness.get_by_label(egui_phosphor::regular::MAGIC_WAND).click();
    harness.run_steps(3);

    harness.get_by_label("Add filter").click();
    harness.run_steps(3);

    let StackTarget::Effect(added) = harness.state().session.target else { panic!("the new filter is selected") };
    assert!(matches!(kind(&harness, added), EffectKind::Filter(Filter::Hsl { .. })), "{:?}", kind(&harness, added));
    assert!(harness.query_by_role_and_label(Role::ComboBox, "HSL Perceptive").is_some(), "Properties shows the new filter's slot");
}
