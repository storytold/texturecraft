//! The generator panel: global adjustments, restore defaults and image inputs edit the selected generator.

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{BakedMap, EffectKind, Generator, GeneratorKind, LayerId, Source};
use texturecraft_engine::{ViewMode, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::theme::Brightness;

/// The sample session with a fill layer whose black mask holds a curvature generator, selected in the properties panel.
fn harness(render: bool) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(512, 256).unwrap();
    session.active_set = 1;
    session.camera.pitch = 0.5;
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({})).unwrap();
    let effect = execute(&mut session, "effect.add.generator", &json!({"generator": "curvature"})).unwrap()["effect"].as_u64().unwrap();
    session.view = ViewMode::Mask(LayerId(layer));
    session.update();
    let builder = Harness::builder().with_size(egui::vec2(1600.0, 2400.0));
    let builder = if render { builder.wgpu() } else { builder };
    let mut harness = builder.build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.dock.close(Panel::Layers);
    harness.state_mut().ui.dock.close(Panel::TextureSetSettings);
    harness.state_mut().ui.dock.focus(Panel::Properties);
    harness.run_steps(6);
    (harness, LayerId(effect))
}

fn generator(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> Generator {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Generator(generator) => generator.clone(),
        _ => panic!("expected a generator"),
    }
}

/// Drags the track under slider `label` to the right.
fn drag_slider(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    let rect = harness.get_by_label(label).rect();
    let start = egui::pos2(rect.left() + 20.0, rect.bottom() + 10.0);
    let end = start + egui::vec2(120.0, 0.0);
    harness.drag_at(start);
    harness.run_steps(2);
    harness.hover_at(end);
    harness.run_steps(2);
    harness.drop_at(end);
    harness.run_steps(3);
}

#[test]
fn global_blur_applies_as_one_undo_step() {
    let (mut harness, effect) = harness(false);
    let history = harness.state().session.history_labels().len();
    drag_slider(&mut harness, "Global blur");
    assert!(generator(&harness, effect).global.blur > 0.0);
    assert_eq!(harness.state().session.history_labels().len(), history + 1);
    harness.state_mut().session.undo().unwrap();
    assert_eq!(generator(&harness, effect).global.blur, 0.0);
}

#[test]
fn restore_defaults_puts_the_generator_back_to_its_kind_defaults() {
    let (mut harness, effect) = harness(false);
    let edits = json!({"effect": effect.0, "level": 0.9, "blur": 0.5, "balance": 0.2, "resolutionFactor": -2, "seed": 99});
    execute(&mut harness.state_mut().session, "effect.set", &edits).unwrap();
    harness.run_steps(3);
    harness.get_by_label("Restore defaults").scroll_to_me();
    harness.run_steps(2);
    harness.get_by_label("Restore defaults").click();
    harness.run_steps(3);
    assert_eq!(generator(&harness, effect), Generator::new(GeneratorKind::Curvature));
}

#[test]
fn an_input_can_read_a_uniform_value_and_the_mesh_map_again() {
    let (mut harness, effect) = harness(false);
    harness.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("Mesh map: Curvature")).unwrap().scroll_to_me();
    harness.run_steps(3);
    harness.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("Mesh map: Curvature")).unwrap().click();
    harness.run_steps(2);
    harness.get_by_label("Uniform value").click();
    harness.run_steps(3);
    assert!(matches!(generator(&harness, effect).inputs.get(&BakedMap::Curvature), Some(Source::Uniform(_))));

    harness.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("Uniform value")).unwrap().click();
    harness.run_steps(2);
    harness.get_by_label("Mesh map").click();
    harness.run_steps(3);
    assert!(generator(&harness, effect).inputs.is_empty());
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn generator_panel_snapshots() {
    let directory = std::path::PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&directory).unwrap();
    let (mut harness, effect) = harness(true);
    harness.run_steps(12);
    harness.render().unwrap().save(directory.join("generator-panel-default.png")).unwrap();

    for section in ["Attributes", "Resolution"] {
        harness.get_by_label(section).click();
        harness.run_steps(3);
    }
    harness.run_steps(6);
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        harness.run_steps(3);
        harness.render().unwrap().save(directory.join(format!("generator-panel-sections-open-{name}.png"))).unwrap();
    }
    harness.state_mut().ui.prefs.brightness = Brightness::MediumDark;

    let edits = json!({"effect": effect.0, "blur": 0.3, "balance": 0.6, "contrast": 0.4, "resolutionFactor": -2});
    execute(&mut harness.state_mut().session, "effect.set", &edits).unwrap();
    harness.state_mut().session.update();
    harness.run_steps(12);
    harness.render().unwrap().save(directory.join("generator-panel-adjusted.png")).unwrap();

    // The effect row with its blend mode and opacity, in the layer stack
    harness.state_mut().ui.dock.focus(Panel::Layers);
    harness.run_steps(12);
    harness.render().unwrap().save(directory.join("generator-effect-row.png")).unwrap();
}
