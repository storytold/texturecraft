//! The Smudge panel shows which channels a smudge drags and shares their on/off flags with the Material tab

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, Tool, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::PropsTab;

const RES: f32 = 256.0;

fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    session.viewport = Vec2::new(400.0, 400.0);
    session.camera.yaw = 0.0;
    session.camera.pitch = 0.0;
    session.update();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(4);
    run(&mut harness, "tool.smudge", Value::Null);
    harness
}

fn run(harness: &mut Harness<'_, TextureCraftApp>, command: &str, params: Value) {
    execute(&mut harness.state_mut().session, command, &params).unwrap();
    harness.run_steps(4);
}

/// The channel chips drawn in the open Properties panel
fn chip_is_on(harness: &Harness<'_, TextureCraftApp>, channel: Channel) -> bool {
    let chips: Vec<_> = harness.get_all_by_role_and_label(Role::Button, channel.short()).collect();
    assert_eq!(chips.len(), 1, "one {} chip is drawn", channel.short());
    chips[0].accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// Channels whose chip is on, in the order of `Channel::ALL`
fn chips_on(harness: &Harness<'_, TextureCraftApp>) -> Vec<Channel> {
    Channel::ALL
        .iter()
        .copied()
        .filter(|channel| harness.get_all_by_role_and_label(Role::Button, channel.short()).next().is_some() && chip_is_on(harness, *channel))
        .collect()
}

fn enabled(harness: &Harness<'_, TextureCraftApp>) -> Vec<Channel> {
    let tool = &harness.state().session.tool;
    Channel::ALL.iter().copied().filter(|channel| tool.enabled.contains(channel)).collect()
}

fn base_color_at(session: &Session, x: f32, y: f32) -> [f32; 3] {
    let hit = session.pick(Vec2::new(x, y), session.viewport).unwrap();
    session.caches[0].composite.value(Channel::BaseColor, (hit.uv.x * RES) as u32, (hit.uv.y * RES) as u32)
}

/// Paints a red dab, then drags it right with a full-strength smudge
fn smudge_red_dab(harness: &mut Harness<'_, TextureCraftApp>) -> ([f32; 3], [f32; 3]) {
    run(harness, "tool.paint", Value::Null);
    run(harness, "tool.material", json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]}));
    run(harness, "tool.brush", json!({"size": 12, "hardness": 90}));
    run(harness, "tool.stroke", json!({"points": [[170, 200]]}));
    run(harness, "tool.smudge", Value::Null);
    run(harness, "tool.brush", json!({"size": 10, "smudgeStrength": 100, "spacing": 5}));
    let before = base_color_at(&harness.state().session, 205.0, 200.0);
    let points: Vec<[f32; 2]> = (0..=40).map(|i| [170.0 + i as f32, 200.0]).collect();
    run(harness, "tool.stroke", json!({"points": points}));
    (before, base_color_at(&harness.state().session, 205.0, 200.0))
}

#[test]
fn smudge_panel_shows_the_channels_the_material_has_on() {
    let mut harness = harness();
    assert_eq!(harness.state().session.tool.tool, Tool::Smudge);
    assert!(harness.query_by_label("Channels dragged").is_some());
    assert_eq!(chips_on(&harness), enabled(&harness));
    for channel in [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Normal, Channel::Height] {
        assert!(chip_is_on(&harness, channel), "{channel:?} is on by default");
    }

    run(&mut harness, "tool.material", json!({"channel": "roughness", "enabled": false}));

    assert_eq!(chips_on(&harness), enabled(&harness));
    assert!(!chip_is_on(&harness, Channel::Roughness));
}

#[test]
fn clicking_a_channel_in_the_smudge_panel_turns_it_off_and_on_for_the_material() {
    let mut harness = harness();

    harness.get_by_role_and_label(Role::Button, Channel::Metallic.short()).click();
    harness.run_steps(4);
    assert!(!harness.state().session.tool.enabled.contains(&Channel::Metallic));
    assert!(!chip_is_on(&harness, Channel::Metallic));

    harness.get_by_role_and_label(Role::Button, Channel::Metallic.short()).click();
    harness.run_steps(4);
    assert!(harness.state().session.tool.enabled.contains(&Channel::Metallic));
    assert!(chip_is_on(&harness, Channel::Metallic));
}

#[test]
fn a_channel_switched_off_in_the_smudge_panel_is_not_dragged() {
    let mut dragged = harness();
    let (before, after) = smudge_red_dab(&mut dragged);
    assert!(after[0] - before[0] > 0.2, "base color is dragged while its chip is on: {before:?} -> {after:?}");

    let mut kept = harness();
    run(&mut kept, "tool.paint", Value::Null);
    run(&mut kept, "tool.material", json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]}));
    run(&mut kept, "tool.brush", json!({"size": 12, "hardness": 90}));
    run(&mut kept, "tool.stroke", json!({"points": [[170, 200]]}));
    run(&mut kept, "tool.smudge", Value::Null);
    run(&mut kept, "tool.brush", json!({"size": 10, "smudgeStrength": 100, "spacing": 5}));
    kept.get_by_role_and_label(Role::Button, Channel::BaseColor.short()).click();
    kept.run_steps(4);
    assert!(!kept.state().session.tool.enabled.contains(&Channel::BaseColor));
    let before = base_color_at(&kept.state().session, 205.0, 200.0);
    let points: Vec<[f32; 2]> = (0..=40).map(|i| [170.0 + i as f32, 200.0]).collect();
    run(&mut kept, "tool.stroke", json!({"points": points}));
    let after = base_color_at(&kept.state().session, 205.0, 200.0);
    assert!((after[0] - before[0]).abs() < 0.02, "base color stays put with its chip off: {before:?} -> {after:?}");
}

#[test]
fn material_tab_and_smudge_panel_stay_in_sync() {
    let mut harness = harness();
    harness.state_mut().ui.props_tab = PropsTab::Material;
    harness.run_steps(4);

    // Both the Smudge panel and the Material tab draw a chip per channel
    let chips = harness.get_all_by_role_and_label(Role::Button, Channel::Height.short()).count();
    assert_eq!(chips, 2, "Smudge panel and Material tab");

    let first = harness.get_all_by_role_and_label(Role::Button, Channel::Height.short()).next().unwrap();
    first.click();
    harness.run_steps(4);

    assert!(!harness.state().session.tool.enabled.contains(&Channel::Height));
    for chip in harness.get_all_by_role_and_label(Role::Button, Channel::Height.short()) {
        assert_ne!(chip.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "both chips show height off");
    }
}
