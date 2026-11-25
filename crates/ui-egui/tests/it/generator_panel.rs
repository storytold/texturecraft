//! The generator panel: global adjustments, restore defaults and image inputs edit the selected generator

use std::path::PathBuf;
use std::time::Duration;

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{AxisBlend, BakedMap, EffectKind, Generator, GeneratorKind, KindControls, LayerId, SectionParameters, Source, WearParameters};
use texturecraft_engine::{ViewMode, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::theme::Brightness;

/// The sample session with a fill layer whose black mask holds a curvature generator, selected in the properties panel
fn harness(render: bool) -> (Harness<'static, TextureCraftApp>, LayerId) {
    harness_with(render, json!({"generator": "curvature"}))
}

/// The sample session with a fill layer whose black mask holds the generator `added` describes, selected in the properties panel
fn harness_with(render: bool, added: serde_json::Value) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(512, 256).unwrap();
    session.active_set = 1;
    session.camera.pitch = 0.5;
    let layer = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({})).unwrap();
    let effect = execute(&mut session, "effect.add.generator", &added).unwrap()["effect"].as_u64().unwrap();
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

#[test]
fn the_runoff_panel_exposes_and_applies_the_reworded_streak_control() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "Dripping Rust"}));
    let Some(KindControls::DrippingRust(before)) = generator(&harness, effect).kind_controls() else { panic!("expected runoff controls") };

    drag_slider(&mut harness, "Streak Reach");

    let Some(KindControls::DrippingRust(after)) = generator(&harness, effect).kind_controls() else { panic!("expected runoff controls") };
    assert!(after.drips_intensity > before.drips_intensity);
}

/// Drags slider `label` from near its left end to the right
fn drag_slider(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    harness.get_by_role_and_label(Role::Slider, label).scroll_to_me();
    harness.run_steps(3);
    let rect = harness.get_by_role_and_label(Role::Slider, label).rect();
    let start = egui::pos2(rect.left() + 12.0, rect.center().y);

    // End past the midpoint to distinguish the edit from the default
    let end = egui::pos2(rect.left() + rect.width() * 0.75, rect.center().y);
    harness.drag_at(start);
    harness.run_steps(2);
    harness.hover_at(end);
    harness.run_steps(2);
    harness.drop_at(end);
    harness.run_steps(3);
}

/// A press and release at `at`
fn click_at(harness: &mut Harness<'_, TextureCraftApp>, at: egui::Pos2) {
    harness.drag_at(at);
    harness.run_steps(1);
    harness.drop_at(at);
    harness.run_steps(3);
}

/// Clicks the one live control named `label` with `role` (the panel's Restore defaults and a slot's share a name)
fn click_live(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    let live = |harness: &Harness<'_, TextureCraftApp>| {
        harness.get_all_by_role_and_label(role, label).find(|node| !node.accesskit_node().is_disabled()).expect("a live control").rect()
    };
    let rect = live(harness);
    harness.hover_at(rect.center());
    harness.run_steps(2);
    let at = live(harness).center();
    click_at(harness, at);
}

#[test]
fn global_blur_applies_as_one_undo_step() {
    let (mut harness, effect) = harness(false);
    let history = harness.state().session.history_labels().len();
    drag_slider(&mut harness, "Global Blur");
    assert!(generator(&harness, effect).global.blur > 0.0);
    assert_eq!(harness.state().session.history_labels().len(), history + 1);
    harness.state_mut().session.undo().unwrap();
    assert_eq!(generator(&harness, effect).global.blur, 0.0);
}

#[test]
fn restore_defaults_puts_the_generator_back_to_its_kind_defaults() {
    const SET_FACTOR: i8 = -2;
    let (mut harness, effect) = harness(false);
    assert!(harness.get_by_role_and_label(Role::Button, "Restore defaults").accesskit_node().is_disabled(), "at its defaults it does nothing");
    let edits = json!({"effect": effect.0, "level": 0.9, "blur": 0.5, "balance": 0.2, "resolutionFactor": SET_FACTOR, "seed": 99});
    execute(&mut harness.state_mut().session, "effect.set", &edits).unwrap();
    harness.run_steps(3);

    click_live(&mut harness, Role::Button, "Restore defaults");

    // Resolution has its own band: restoring the parameters leaves it as set
    let restored = generator(&harness, effect);
    assert_eq!(restored.resolution.factor, SET_FACTOR);
    assert_eq!(Generator { resolution: Generator::new(GeneratorKind::Curvature).resolution, ..restored }, Generator::new(GeneratorKind::Curvature));
}

#[test]
fn picking_a_kind_in_the_panel_is_one_undo_step() {
    let (mut harness, effect) = harness(false);
    let history = harness.state().session.history_labels().len();
    harness.get_by_role_and_label(Role::ComboBox, "Curvature").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::ListBoxOption, "Dirt").click();
    harness.run_steps(3);

    assert_eq!(generator(&harness, effect).kind, GeneratorKind::Dirt);
    assert_eq!(harness.state().session.history_labels().len(), history + 1);
}

#[test]
fn a_wear_control_moves_an_earlier_wear_generator_onto_the_wear_controls() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "edgeWear", "level": 0.4}));
    assert_eq!(generator(&harness, effect).wear, None, "level keeps it on the earlier model");

    drag_slider(&mut harness, "Wear Coverage");

    let wear = generator(&harness, effect).wear.expect("on the wear controls now");
    let defaults = WearParameters::defaults(GeneratorKind::EdgeWear).unwrap();
    assert_ne!(wear.level, defaults.level, "the drag set the level");
    assert_eq!(WearParameters { level: defaults.level, ..wear }, defaults, "the other controls start at their defaults");
}

#[test]
fn restore_defaults_keeps_a_wear_generator_on_its_wear_controls() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "dirt", "grungeAmount": 0.9, "seed": 3}));
    click_live(&mut harness, Role::Button, "Restore defaults");
    assert_eq!(generator(&harness, effect), Generator::new(GeneratorKind::Dirt));
}

/// The slot reading `value`: an image input's card
fn slot_reading<'a>(harness: &'a Harness<'_, TextureCraftApp>, value: &str) -> egui_kittest::Node<'a> {
    harness.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(value)).expect("a slot reading it")
}

#[test]
fn an_input_can_read_a_uniform_value_and_the_mesh_map_again() {
    let (mut harness, effect) = harness(false);
    harness.get_by_role_and_label(Role::Button, "Image inputs").click();
    harness.run_steps(3);
    slot_reading(&harness, "Mesh map: Curvature").scroll_to_me();
    harness.run_steps(3);
    slot_reading(&harness, "Mesh map: Curvature").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(Role::ListBoxOption, "Uniform value").click();
    harness.run_steps(3);
    assert!(matches!(generator(&harness, effect).inputs.get(&BakedMap::Curvature), Some(Source::Uniform(_))));
    assert!(slot_reading(&harness, "Uniform value").rect().is_positive());

    click_live(&mut harness, Role::Button, "Restore defaults");
    assert!(generator(&harness, effect).inputs.is_empty(), "the slot's reset reads the mesh map again");
}

/// How many nodes carry `label`
fn count(harness: &Harness<'_, TextureCraftApp>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

#[test]
fn each_kind_shows_its_section_rows_and_no_earlier_ones() {
    let rows = [
        ("curvature", &["Sharp", "Huge", "Brightness", "Global Contrast"][..], &["Balance", "Blending Mode", "Level"][..]),
        ("ambientOcclusion", &["Blur", "Balance", "Global Contrast"][..], &["Brightness", "Blending Mode", "Level"][..]),
        ("position", &["Balance", "Brightness", "Right to Left", "Front to Back"][..], &["Blur", "Level", "Sharp"][..]),
        ("worldSpaceNormals", &["Blur", "Balance", "Brightness", "Top To Bottom"][..], &["Level", "Softness"][..]),
        (
            "direction",
            &["Horizontal Angle", "Vertical Angle", "Highlight Glossiness", "Highlight Level", "Light Attenuation"][..],
            &["Global Blur", "Level"][..],
        ),
    ];
    for (kind, shown, hidden) in rows {
        let (harness, _) = harness_with(false, json!({"generator": kind}));
        for label in shown {
            assert!(count(&harness, label) > 0, "{kind} shows {label}");
        }
        for label in hidden {
            assert_eq!(count(&harness, label), 0, "{kind} hides {label}");
        }
    }

    let (harness, _) = harness_with(false, json!({"generator": "worldSpaceNormals"}));
    assert_eq!(harness.get_all_by_role_and_label(Role::ComboBox, "Linear Dodge (Add)").count(), 3, "a blending mode per axis");
    let (harness, _) = harness_with(false, json!({"generator": "position", "level": 0.4}));
    assert!(count(&harness, "Level") > 0 && count(&harness, "Balance") > 0, "the earlier model keeps its level beside the section at its defaults");
}

#[test]
fn each_axis_has_its_own_blending_mode_dropdown() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "worldSpaceNormals"}));
    let lowest = |harness: &Harness<'_, TextureCraftApp>| {
        let dropdowns = harness.get_all_by_role_and_label(Role::ComboBox, "Linear Dodge (Add)");
        dropdowns.max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().rect()
    };
    harness.get_by_role_and_label(Role::Slider, "Front to Back").scroll_to_me();
    harness.run_steps(3);
    let at = lowest(&harness).center();
    click_at(&mut harness, at);
    harness.get_by_role_and_label(Role::ListBoxOption, "Multiply").click();
    harness.run_steps(3);

    let axes = generator(&harness, effect).section.unwrap().axes;
    assert_eq!(axes.map(|axis| axis.blend), [AxisBlend::Add, AxisBlend::Add, AxisBlend::Multiply], "only front to back changed");
}

#[test]
fn a_section_control_moves_an_earlier_generator_onto_the_section_controls() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "position", "level": 0.4}));
    assert_eq!(generator(&harness, effect).section, None);

    drag_slider(&mut harness, "Balance");

    let section = generator(&harness, effect).section.expect("on the section controls now");
    let defaults = SectionParameters::defaults(GeneratorKind::Position).unwrap();
    assert_ne!(section.balance, defaults.balance, "the drag set the balance");
    assert_eq!(SectionParameters { balance: defaults.balance, ..section }, defaults, "the other controls start at their defaults");
    assert_eq!(count(&harness, "Level"), 0, "the earlier rows are gone");
}

#[test]
fn a_curvature_size_column_sets_its_size_alone_in_one_step() {
    let (mut harness, effect) = harness(false);
    let before = generator(&harness, effect).detail.unwrap().weights;
    let history = harness.state().session.history_labels().len();
    harness.get_by_role_and_label(Role::Slider, "Huge").scroll_to_me();
    harness.run_steps(3);
    let column = harness.get_by_role_and_label(Role::Slider, "Huge").rect();
    let (low, high) = (egui::pos2(column.center().x, column.bottom() - 30.0), egui::pos2(column.center().x, column.top() + 24.0));
    harness.drag_at(low);
    harness.run_steps(2);
    harness.hover_at(high);
    harness.run_steps(2);
    harness.drop_at(high);
    harness.run_steps(3);

    let after = generator(&harness, effect).detail.unwrap().weights;
    let last = before.len() - 1;
    assert!(after[last] > before[last], "dragged up, Huge rose: {} -> {}", before[last], after[last]);
    assert_eq!(after[..last], before[..last], "the other sizes kept theirs");
    assert_eq!(harness.state().session.history_labels().len(), history + 1);
}

#[test]
fn a_focused_curvature_size_column_steps_with_the_arrow_keys() {
    let (mut harness, effect) = harness(false);
    let last = generator(&harness, effect).detail.unwrap().weights.len() - 1;
    harness.get_by_role_and_label(Role::Slider, "Huge").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Slider, "Huge").focus();
    harness.run_steps(2);
    let before = generator(&harness, effect).detail.unwrap().weights;
    let history = harness.state().session.history_labels().len();

    harness.key_press(egui::Key::ArrowDown);
    harness.run_steps(3);
    let lowered = generator(&harness, effect).detail.unwrap().weights;
    harness.key_press(egui::Key::ArrowUp);
    harness.run_steps(3);

    assert!(lowered[last] < before[last], "down lowered Huge: {} -> {}", before[last], lowered[last]);
    assert_eq!(lowered[..last], before[..last], "the other sizes kept theirs");
    assert!((generator(&harness, effect).detail.unwrap().weights[last] - before[last]).abs() < 1e-6, "up raised it back");
    assert_eq!(harness.state().session.history_labels().len(), history + 2, "each press is a step");
}

#[test]
fn restore_defaults_puts_a_light_back_on_its_defaults() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "direction", "lightHorizontal": 10.0, "highlightLevel": 0.7, "invert": true}));
    click_live(&mut harness, Role::Button, "Restore defaults");
    assert_eq!(generator(&harness, effect), Generator::new(GeneratorKind::Direction));
}

#[test]
fn micro_details_show_their_settings_once_switched_on() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "ambientOcclusion"}));
    assert_eq!(count(&harness, "AO Radius"), 0, "they wait for Use Micro Details");
    harness.get_by_role_and_label(Role::CheckBox, "Use Micro Details").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::CheckBox, "Use Micro Details").click();
    harness.run_steps(3);
    assert!(generator(&harness, effect).micro.is_some_and(|micro| micro.enabled));
    for label in ["Micro Height", "Micro Normal", "AO Radius", "AO Depth"] {
        assert!(count(&harness, label) > 0, "{label}");
    }
    assert_eq!(count(&harness, "Curvature Type"), 0, "Ambient Occlusion's micro details have no curvature");

    let (mut harness, _) = harness_with(false, json!({"generator": "dirt", "microHeight": true}));
    harness.get_by_role_and_label(Role::Button, "Micro Details").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Micro Details").click();
    harness.run_steps(3);
    assert!(slot_reading(&harness, "Empty").rect().is_positive(), "Micro Height on: its slot, empty");
}

/// An instance showing its source's generator offers its Micro Height only the anchor points below both: one between the source and the instance is refused by the source's mask
#[test]
fn an_inherited_generators_micro_height_lists_only_anchors_below_its_owner() {
    const BELOW: &str = "Height below";
    const BETWEEN: &str = "Height between";
    let mut session = texturecraft_engine::sample_session(512, 256).unwrap();
    session.active_set = 1;
    let low = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "effect.add.anchor", &json!({"layer": low, "target": "content", "name": BELOW})).unwrap();
    let source = execute(&mut session, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1]})).unwrap()["layer"].clone();
    execute(&mut session, "mask.add.black", &json!({"layer": source})).unwrap();
    let added = execute(&mut session, "effect.add.generator", &json!({"layer": source, "generator": "dirt", "microHeight": true})).unwrap();
    let effect = LayerId(added["effect"].as_u64().unwrap());
    let between = execute(&mut session, "layer.add.fill", &json!({})).unwrap()["layer"].clone();
    execute(&mut session, "effect.add.anchor", &json!({"layer": between, "target": "content", "name": BETWEEN})).unwrap();
    let instance = execute(&mut session, "layer.instance", &json!({"layer": source})).unwrap()["layer"].clone();
    execute(&mut session, "layer.select", &json!({"layer": instance})).unwrap();
    session.target = texturecraft_engine::StackTarget::Effect(effect);
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 2400.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.dock.close(Panel::Layers);
    harness.state_mut().ui.dock.focus(Panel::Properties);
    harness.run_steps(6);

    harness.get_by_role_and_label(Role::Button, "Micro Details").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Micro Details").click();
    harness.run_steps(3);
    slot_reading(&harness, "Empty").scroll_to_me();
    harness.run_steps(3);
    slot_reading(&harness, "Empty").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Tab, "Anchor points").click();
    harness.run_steps(3);

    let offered: Vec<_> = harness.get_all_by_role(Role::ListBoxOption).filter_map(|row| row.accesskit_node().label()).collect();
    assert!(offered.iter().any(|label| label.contains(BELOW)), "the anchor below the source is offered: {offered:?}");
    assert!(!offered.iter().any(|label| label.contains(BETWEEN)), "the anchor above the source is excluded: {offered:?}");
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn generator_panel_snapshots() {
    let directory = PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&directory).unwrap();
    let (mut harness, effect) = harness(true);
    harness.run_steps(12);
    harness.render().unwrap().save(directory.join("generator-panel-default.png")).unwrap();

    for section in ["Attributes", "Resolution"] {
        harness.get_by_role_and_label(Role::Button, section).click();
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

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn full_set_panel_snapshots() {
    let directory = PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&directory).unwrap();
    let kinds = ["edgeWear", "fiberGlassEdgeWear", "dirt", "direction", "curvature", "ambientOcclusion", "position", "worldSpaceNormals"];
    for kind in kinds {
        let (mut harness, _) = harness_with(true, json!({"generator": kind, "seed": 12}));
        if kind == "dirt" {
            harness.get_by_role_and_label(Role::Button, "Micro Details").click();
        }

        // The orbs render on the preview worker
        for _ in 0..40 {
            harness.run_steps(4);
            std::thread::sleep(Duration::from_millis(50));
        }
        for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            harness.run_steps(3);
            harness.render().unwrap().save(directory.join(format!("generator-{kind}-{name}.png"))).unwrap();
            std::fs::write(directory.join(format!("generator-{kind}-{name}.txt")), format!("{:#?}", harness.root())).unwrap();
        }
    }
}

#[test]
fn use_texture_shows_the_texture_rows_and_its_slot() {
    let (harness, _) = harness_with(false, json!({"generator": "position"}));
    assert_eq!(harness.get_all_by_role_and_label(Role::CheckBox, "Use Texture").count(), 1);
    let dropdowns = |harness: &Harness<'_, TextureCraftApp>| harness.get_all_by_role(Role::ComboBox).count();
    let closed = dropdowns(&harness);
    for label in ["Texture Opacity", "Texture"] {
        assert_eq!(count(&harness, label), 0, "{label} waits for Use Texture");
    }

    let (harness, _) = harness_with(false, json!({"generator": "position", "useTexture": true}));
    for label in ["Texture Opacity", "Triplanar", "Scale"] {
        assert!(count(&harness, label) > 0, "{label}");
    }
    assert_eq!(count(&harness, "Triplanar Contrast"), 0, "it waits for Triplanar");
    assert!(harness.get_by_role_and_label(Role::ComboBox, "Texture").rect().is_positive(), "the texture's slot in its card");
    assert_eq!(dropdowns(&harness), closed + 3, "the texture's slot, Grayscale Conversion and Blending Mode");

    let (harness, _) = harness_with(false, json!({"generator": "dirt"}));
    assert_eq!(count(&harness, "Use Texture"), 0, "wear generators have no texture");
}

#[test]
fn the_texture_slot_takes_a_uniform_value_and_empties_again() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "ambientOcclusion", "useTexture": true}));
    slot_reading(&harness, "Empty").scroll_to_me();
    harness.run_steps(3);
    slot_reading(&harness, "Empty").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(Role::ListBoxOption, "Uniform value").click();
    harness.run_steps(3);
    assert!(matches!(generator(&harness, effect).texture.unwrap().source, Some(Source::Uniform(_))));
    assert!(count(&harness, "Value") > 0, "with its value");

    click_live(&mut harness, Role::Button, "Clear");
    assert_eq!(generator(&harness, effect).texture.unwrap().source, None);
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn texture_panel_snapshots() {
    let directory = PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&directory).unwrap();
    for kind in ["position", "ambientOcclusion"] {
        let (mut harness, _) = harness_with(true, json!({"generator": kind, "useTexture": true, "textureOpacity": 0.6, "textureBlend": "normal"}));
        harness.get_by_role_and_label(Role::CheckBox, "Use Texture").scroll_to_me();
        harness.run_steps(12);
        for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            harness.run_steps(3);
            harness.render().unwrap().save(directory.join(format!("texture-panel-{kind}-{name}.png"))).unwrap();
        }
    }
}

/// Whether control `label` with `role` shows on
fn is_on(harness: &Harness<'_, TextureCraftApp>, role: Role, label: &str) -> bool {
    harness.get_by_role_and_label(role, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn restore_defaults_puts_a_kind_with_its_own_controls_back() {
    let added = json!({"generator": "uvRandomColor", "colorSourceMode": "customGradient", "gradientColors": [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]], "gradientPositions": [0.0, 0.3, 1.0], "seed": 4});
    let (mut harness, effect) = harness_with(false, added);
    execute(&mut harness.state_mut().session, "effect.set", &json!({"effect": effect.0, "gradientQuantity": 2})).unwrap();
    harness.run_steps(3);

    click_live(&mut harness, Role::Button, "Restore defaults");

    assert_eq!(generator(&harness, effect), Generator::new(GeneratorKind::UvRandomColor), "colors, places and count back to the kind's");
    assert!(harness.get_by_role_and_label(Role::Button, "Restore defaults").accesskit_node().is_disabled(), "nothing left to restore");
}

#[test]
fn a_point_pick_waits_for_the_view_until_escape() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "3dLinearGradient"}));
    let (start, end) = ("Pick 3D Position Start on the mesh", "Pick 3D Position End on the mesh");
    assert!(!is_on(&harness, Role::Button, start) && !is_on(&harness, Role::Button, end));

    click_live(&mut harness, Role::Button, end);
    assert!(is_on(&harness, Role::Button, end), "armed: the next click in the view sets the end");
    assert!(!is_on(&harness, Role::Button, start), "only the point picked waits");

    click_live(&mut harness, Role::Button, start);
    assert!(is_on(&harness, Role::Button, start) && !is_on(&harness, Role::Button, end), "arming the start hands the wait over");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(!is_on(&harness, Role::Button, start), "Esc cancels the pick");

    // Selecting something else drops the pick: coming back finds it disarmed
    click_live(&mut harness, Role::Button, end);
    let layer = harness.state().session.doc.project.effect(effect).unwrap().0;

    execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": layer.0, "target": "content"})).unwrap();
    harness.run_steps(3);

    execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": layer.0, "target": effect.0})).unwrap();
    harness.run_steps(3);

    assert!(!is_on(&harness, Role::Button, end), "a pick left behind is dropped");

    // Swapping to a kind without points cancels the pending pick
    click_live(&mut harness, Role::Button, end);
    for kind in ["position", "3dLinearGradient"] {
        execute(&mut harness.state_mut().session, "effect.set", &json!({"effect": effect.0, "generator": kind})).unwrap();
        harness.run_steps(3);
    }
    assert!(!is_on(&harness, Role::Button, end), "a pick on a swapped kind is dropped");

    let (mut next, same_effect) = harness_with(false, json!({"generator": "3dLinearGradient"}));
    assert_eq!(effect, same_effect);
    click_live(&mut harness, Role::Button, end);
    assert!(is_on(&harness, Role::Button, end));

    std::mem::swap(&mut harness.state_mut().session, &mut next.state_mut().session);
    harness.run_steps(3);

    assert!(!is_on(&harness, Role::Button, end), "a point pick belongs to its document");
}

#[test]
fn a_gradient_color_place_moves_its_stop_in_one_step() {
    let added =
        json!({"generator": "uvRandomColor", "colorSourceMode": "customGradient", "gradientColors": [[0.0, 0.0, 0.0], [0.8, 0.3, 0.1], [1.0, 1.0, 1.0]]});
    let (mut harness, effect) = harness_with(false, added);
    let places = |harness: &Harness<'_, TextureCraftApp>| match generator(harness, effect).kind_controls() {
        Some(KindControls::UvRandomColor(random)) => random.used_positions(),
        other => panic!("not UV Random Color controls: {other:?}"),
    };
    let before = places(&harness);

    // Color 2's own track, after its marker on the ramp
    let rect = harness.get_all_by_role_and_label(Role::Slider, "Color 2").last().expect("Color 2's place").rect();
    let (from, to) = (egui::pos2(rect.left() + 4.0, rect.center().y), egui::pos2(rect.right() - 4.0, rect.center().y));
    harness.drag_at(from);
    harness.run_steps(2);
    harness.hover_at(to);
    harness.run_steps(2);
    harness.drop_at(to);
    harness.run_steps(3);

    let after = places(&harness);
    assert!(after[1] > before[1], "Color 2 moved right along the gradient");
    assert_eq!((after[0], after[2]), (before[0], before[2]), "the other colors keep their places");
    harness.state_mut().session.undo().unwrap();
    assert_eq!(places(&harness), before, "one undo step puts it back");
}

#[test]
fn the_color_rows_follow_the_color_mode() {
    let (mut harness, _) = harness_with(false, json!({"generator": "uvChecker"}));
    assert_eq!(count(&harness, "Checker Color"), 1, "uniform: its color shows");
    click_live(&mut harness, Role::RadioButton, "Random");
    assert_eq!(count(&harness, "Checker Color"), 0, "random colors: no color to pick");
    assert_eq!(count(&harness, "UV Borders Color"), 1, "the borders keep theirs");

    let (mut harness, _) = harness_with(false, json!({"generator": "uvRandomColor"}));
    assert_eq!(count(&harness, "Color 1"), 0, "random: no gradient");
    harness.get_by_role_and_label(Role::ComboBox, "Random").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::ListBoxOption, "Custom Gradient").click();
    harness.run_steps(3);
    assert!(count(&harness, "Color 1") > 0 && count(&harness, "Color 2") > 0, "a custom gradient shows its colors");
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn new_kind_panel_snapshots() {
    let directory = PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&directory).unwrap();
    let gradient = json!([[0.02, 0.02, 0.02], [0.6, 0.12, 0.03], [0.8, 0.8, 0.8]]);
    let kinds = [
        ("3dDistance", json!({})),
        ("3dLinearGradient", json!({})),
        ("uvBorderDistance", json!({})),
        ("grayscaleConversion", json!({})),
        ("drippingRust", json!({})),
        ("uvRandomColor", json!({"colorSourceMode": "customGradient", "gradientColors": gradient, "gradientPositions": [0.0, 0.45, 1.0]})),
        ("uvChecker", json!({})),
    ];
    for (kind, extra) in kinds {
        let mut added = json!({"generator": kind});
        added.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let (mut harness, _) = harness_with(true, added);
        if kind == "grayscaleConversion" {
            harness.get_by_role_and_label(Role::Button, "Image inputs").click();
        }

        // The orbs render on the preview worker
        for _ in 0..40 {
            harness.run_steps(4);
            std::thread::sleep(Duration::from_millis(50));
        }
        for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
            harness.state_mut().ui.prefs.brightness = brightness;
            harness.run_steps(3);
            harness.render().unwrap().save(directory.join(format!("generator-{kind}-{name}.png"))).unwrap();
            std::fs::write(directory.join(format!("generator-{kind}-{name}.txt")), format!("{:#?}", harness.root())).unwrap();
        }
    }
}

#[test]
fn the_preview_opens_enlarged_in_its_own_window() {
    let (mut harness, effect) = harness_with(false, json!({"generator": "edgeWear"}));
    let enlarged = |harness: &Harness<'_, TextureCraftApp>| harness.query_by_role_and_label(Role::Image, "Preview").is_some();
    assert!(!enlarged(&harness));

    click_live(&mut harness, Role::Button, "Enlarge preview");
    assert!(enlarged(&harness), "a click on the preview opens it enlarged");
    assert!(is_on(&harness, Role::Button, "Enlarge preview"), "the preview shows it is open");

    click_live(&mut harness, Role::Button, "Enlarge preview");
    assert!(!enlarged(&harness), "a second click closes it");

    click_live(&mut harness, Role::Button, "Enlarge preview");
    execute(&mut harness.state_mut().session, "effect.remove", &json!({"effect": effect.0})).unwrap();
    harness.run_steps(3);
    assert!(!enlarged(&harness), "it closes with its generator");
}

#[test]
#[ignore = "requires a GPU adapter and TC_UI_SNAPSHOTS"]
fn enlarged_preview_snapshots() {
    let directory = PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&directory).unwrap();
    let (mut harness, _) = harness_with(true, json!({"generator": "edgeWear", "seed": 12}));
    click_live(&mut harness, Role::Button, "Enlarge preview");

    // The pictures render on the preview worker: up to three seconds of frames
    const WAITS: usize = 60;
    const STEPS_PER_WAIT: usize = 4;
    const WAIT: Duration = Duration::from_millis(50);
    for _ in 0..WAITS {
        harness.run_steps(STEPS_PER_WAIT);
        std::thread::sleep(WAIT);
    }
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        harness.state_mut().ui.prefs.brightness = brightness;
        harness.run_steps(3);
        harness.render().unwrap().save(directory.join(format!("generator-enlarged-preview-{name}.png"))).unwrap();
        std::fs::write(directory.join(format!("generator-enlarged-preview-{name}.txt")), format!("{:#?}", harness.root())).unwrap();
    }
}
