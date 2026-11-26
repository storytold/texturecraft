//! The generator slot in Properties swaps a generator's kind through its picker

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, GeneratorKind, LayerId};
use texturecraft_engine::{StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

/// The app with a Dirt generator in a fill layer's black mask, selected so Properties shows its slot
fn app_with_dirt() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    let effect = execute(&mut session, "effect.add.generator", &json!({"generator": "Dirt"})).unwrap();
    let effect = LayerId(effect["effect"].as_u64().unwrap());
    execute(&mut session, "layer.select", &json!({"layer": layer["layer"]})).unwrap();
    session.target = StackTarget::Effect(effect);

    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    (harness, effect)
}

/// The kind of generator effect `effect` now
fn kind(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> Option<GeneratorKind> {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Generator(generator) => Some(generator.kind),
        _ => None,
    }
}

/// Clicks the Dirt slot, which opens its picker
fn open_picker(harness: &mut Harness<'_, TextureCraftApp>) {
    harness.get_by_role_and_label(Role::ComboBox, "Dirt").click();
    harness.run_steps(3);
    assert!(egui::Popup::is_any_open(&harness.ctx), "clicking the slot opens the picker");
}

#[test]
fn picking_from_the_slot_swaps_the_generator_kind() {
    let (mut harness, effect) = app_with_dirt();
    open_picker(&mut harness);

    harness.event(egui::Event::Text("curv".into()));
    harness.run_steps(3);
    assert_eq!(harness.query_all_by_label("Light").count(), 0, "the search hides generators that don't match");

    // The panel's Curvature input shares the name as a label; the picker's tile is a list option
    harness.get_by_role_and_label(Role::ListBoxOption, "Curvature").click();
    harness.run_steps(3);

    assert_eq!(kind(&harness, effect), Some(GeneratorKind::Curvature));
    assert!(!egui::Popup::is_any_open(&harness.ctx), "a pick closes the picker");
}

#[test]
fn a_former_generator_name_finds_the_reworded_picker_option() {
    let (mut harness, effect) = app_with_dirt();
    open_picker(&mut harness);
    harness.event(egui::Event::Text("fiber glass edge wear".into()));
    harness.run_steps(3);

    harness.get_by_role_and_label(Role::ListBoxOption, "Exposed Glass Fibers").click();
    harness.run_steps(3);

    assert_eq!(kind(&harness, effect), Some(GeneratorKind::FiberGlassEdgeWear));
    assert!(harness.query_all_by_role_and_label(Role::ComboBox, "Exposed Glass Fibers").next().is_some());
}

#[test]
fn the_picker_leaves_thin_areas_out() {
    let (mut harness, _) = app_with_dirt();
    open_picker(&mut harness);

    // The picker's tiles are the list options named after generators
    for kind in GeneratorKind::ALL {
        let offered = harness.query_all_by_role_and_label(Role::ListBoxOption, kind.label()).count() > 0;
        assert_eq!(offered, kind.in_picker(), "{kind:?}");
    }
}

#[test]
#[ignore = "requires an offscreen GPU adapter and TC_UI_SNAPSHOTS"]
fn generator_picker_snapshots() {
    let output = std::path::PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&output).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    let effect = execute(&mut session, "effect.add.generator", &json!({"generator": "EdgeWear"})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer["layer"]})).unwrap();
    session.target = StackTarget::Effect(LayerId(effect["effect"].as_u64().unwrap()));
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(20);
    harness.get_by_role_and_label(Role::ComboBox, "Worn Metal Edges").click();

    // The orbs render on the preview worker: give it time to bake and answer
    for _ in 0..60 {
        harness.run_steps(5);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    for (brightness, name) in
        [(texturecraft_ui_egui::theme::Brightness::MediumDark, "generator-picker"), (texturecraft_ui_egui::theme::Brightness::Light, "generator-picker-light")]
    {
        harness.state_mut().ui.prefs.brightness = brightness;
        harness.run_steps(3);
        harness.render().unwrap().save(output.join(format!("{name}.png"))).unwrap();
        std::fs::write(output.join(format!("{name}.txt")), format!("{:#?}", harness.root())).unwrap();
    }

    // The picker's tile is the list option named Dirt
    harness.get_by_role_and_label(Role::ListBoxOption, "Dirt").hover();
    for _ in 0..10 {
        harness.run_steps(5);
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    harness.render().unwrap().save(output.join("generator-picker-hover.png")).unwrap();
    std::fs::write(output.join("generator-picker-hover.txt"), format!("{:#?}", harness.root())).unwrap();
}
