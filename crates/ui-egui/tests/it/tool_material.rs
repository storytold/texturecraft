//! A brush tool's Material section: the fill layer's channel pills, then a card per channel that is on with the value the brush paints

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

use crate::filter_support::drag;
use crate::{BAKE_RESOLUTION, FRAME_SECONDS, PROPERTIES_WINDOW, SETTLE_FRAMES, SLIDER_END_INSET, STARTUP_FRAMES, TEXTURE_RESOLUTION, click_at, click_widget};

/// A second at 60 frames: the Material tab's jump scrolls with egui's scroll animation
const SCROLL_FRAMES: usize = 60;

/// `tool`'s Properties scrolled to its Material section
fn material_section(tool: &str) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    execute(&mut session, "tool.select", &json!({"tool": tool})).unwrap();
    let mut harness = Harness::builder()
        .with_size(PROPERTIES_WINDOW)
        .with_step_dt(FRAME_SECONDS)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    click_widget(&mut harness, Role::Tab, "Material");
    harness.run_steps(SCROLL_FRAMES);
    harness
}

fn tool_value(harness: &Harness<'_, TextureCraftApp>, channel: Channel) -> [f32; 3] {
    harness.state().session.tool.values.get(&channel).copied().unwrap_or(channel.default_value())
}

/// The top row of the card named `card` ("Metallic: Uniform"), if it shows
fn card_row(harness: &Harness<'_, TextureCraftApp>, card: &str) -> Option<egui::Rect> {
    let named = |node: &egui_kittest::Node<'_>| node.accesskit_node().value().is_some_and(|value| value.starts_with(&format!("{card}: ")));
    harness.query_all_by_role(Role::Label).find(named).map(|node| node.rect())
}

/// Clicks the widget with `role` and `label` on the top row of the card named `card`
fn click_in_card(harness: &mut Harness<'_, TextureCraftApp>, card: &str, role: Role, label: &str) {
    let row = card_row(harness, card).unwrap();
    let at = harness.get_all_by_role_and_label(role, label).map(|node| node.rect().center()).find(|at| row.contains(*at)).unwrap();
    click_at(harness, at);
}

#[test]
fn each_channel_that_is_on_gets_a_card_whose_reset_returns_the_channels_default() {
    let mut harness = material_section("paint");
    assert!(harness.query_by_role_and_label(Role::Button, "Base color").is_some(), "Base color is a colour bar");
    for gray in ["Metallic", "Roughness", "Height"] {
        assert!(harness.query_by_role_and_label(Role::Slider, gray).is_some(), "{gray} is a gray ramp");
    }
    assert!(card_row(&harness, "Normal").is_some(), "Normal has a card");
    assert!(harness.query_by_role_and_label(Role::Slider, "Normal").is_none(), "the normal paints flat: no value to set");

    execute(&mut harness.state_mut().session, "tool.material", &json!({"channel": "metallic", "value": 0.8})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    click_in_card(&mut harness, "Metallic", Role::Button, "Restore defaults");

    assert_eq!(tool_value(&harness, Channel::Metallic), Channel::Metallic.default_value(), "⟲ goes back to the channel's default");

    click_widget(&mut harness, Role::CheckBox, "Metal");

    assert!(!harness.state().session.tool.enabled.contains(&Channel::Metallic), "off in the strip, the brush stops painting Metallic");
    assert!(card_row(&harness, "Metallic").is_none(), "a channel that is off has no card");
}

#[test]
fn a_click_on_a_ramp_sets_the_value_the_brush_paints() {
    let mut harness = material_section("paint");
    let before = tool_value(&harness, Channel::Roughness);
    harness.get_by_role_and_label(Role::Slider, "Roughness").scroll_to_me();
    harness.run_steps(SCROLL_FRAMES);
    let track = harness.get_by_role_and_label(Role::Slider, "Roughness").rect();

    drag(&mut harness, track.center(), egui::pos2(track.right() - SLIDER_END_INSET, track.center().y));

    assert!(tool_value(&harness, Channel::Roughness)[0] > before[0], "dragged toward the ramp's white end the brush paints rougher");
}

#[test]
fn the_eraser_picks_its_channels_without_value_cards() {
    let harness = material_section("eraser");
    assert!(harness.query_by_role_and_label(Role::CheckBox, "Color").is_some(), "the channel strip");
    assert!(harness.query_by_role_and_label(Role::Button, "Base color").is_none(), "the eraser paints no colour");
    assert!(harness.query_by_role_and_label(Role::Slider, "Roughness").is_none());
}

#[test]
fn a_channel_cards_name_is_not_read_as_unavailable() {
    let harness = material_section("paint");
    let named = |node: &egui_kittest::Node<'_>| node.accesskit_node().value().is_some_and(|value| value.starts_with("Metallic: "));
    let card = harness.query_all_by_role(Role::Label).find(named).unwrap();

    assert!(!card.accesskit_node().is_disabled(), "the card's reset and value row answer, so its name must not read as unavailable");
}
