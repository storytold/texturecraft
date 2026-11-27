//! The colour filters in Properties: HSL Perceptive's Mode switch and centred sliders, the tone cards of Color Balance and
//! Color Correct, and Blur's Wrap toggle

use crate::filter_support::{WINDOW_HEIGHT, along, app_with, click_at, filter_of, topmost};
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::Filter;

#[test]
fn hsl_mode_switches_and_its_sliders_sit_at_the_middle() {
    let (mut harness, effect) = app_with("hsl", "content", WINDOW_HEIGHT);
    for label in ["Hue", "Saturation", "Lightness"] {
        let value = harness.get_by_role_and_label(Role::Slider, label).accesskit_node().numeric_value();
        assert_eq!(value, Some(0.5), "{label} unchanged reads 0.50");
    }

    let regular = harness.get_by_role_and_label(Role::RadioButton, "Regular").rect();
    click_at(&mut harness, regular.center());

    assert!(matches!(filter_of(&harness, effect), Filter::Hsl { perceptive: false, .. }), "{:?}", filter_of(&harness, effect));

    // A click at the right end of Saturation doubles it (stored 1)
    let track = harness.get_by_role_and_label(Role::Slider, "Saturation").rect();
    click_at(&mut harness, along(track, 1.0));

    assert!(matches!(filter_of(&harness, effect), Filter::Hsl { saturation, .. } if saturation > 0.95), "{:?}", filter_of(&harness, effect));
}

/// A stored hue past half a turn (0.75, the same hue as -0.25) shows wrapped round on the centred Hue slider, not pinned at its right end
#[test]
fn hsl_hue_past_half_a_turn_shows_wrapped_round() {
    let (mut harness, effect) = app_with("hsl", "content", WINDOW_HEIGHT);
    harness.state_mut().run("effect.set", serde_json::json!({"effect": effect.0, "hue": 0.75}));
    harness.run_steps(2);

    let shown = harness.get_by_role_and_label(Role::Slider, "Hue").accesskit_node().numeric_value().unwrap();

    assert!((shown - 0.25).abs() < 1e-3, "a quarter turn back shows a quarter left of the middle: {shown}");

    harness.state_mut().run("effect.set", serde_json::json!({"effect": effect.0, "hue": 0.5}));
    harness.run_steps(2);

    assert_eq!(harness.get_by_role_and_label(Role::Slider, "Hue").accesskit_node().numeric_value(), Some(1.0));
}

/// Color Balance's tone cards fold one at a time, and a row edits only its own card's tone
#[test]
fn color_balance_tone_cards_fold_and_edit_their_own_tone() {
    let (mut harness, effect) = app_with("colorBalance", "content", WINDOW_HEIGHT);
    assert_eq!(harness.query_all_by_role_and_label(Role::Slider, "Cyan").count(), 3, "a Cyan row per tone");

    let midtones = harness.get_by_role_and_label(Role::Button, "Midtones").rect();
    click_at(&mut harness, midtones.center());

    assert_eq!(harness.query_all_by_role_and_label(Role::Slider, "Cyan").count(), 2, "Midtones folds");

    // Highlights comes first: its Yellow row's left end pushes the highlights to yellow
    let track = topmost(&harness, Role::Slider, "Yellow");
    click_at(&mut harness, along(track, 0.0));

    let Filter::ColorBalance { highlights, shadows, midtones } = filter_of(&harness, effect) else { panic!() };
    assert!(highlights[2] < -0.9 && shadows == [0.0; 3] && midtones == [0.0; 3], "{highlights:?} {midtones:?} {shadows:?}");
}

/// Color Correct's Midtones card sits between its header and the Highlights card's, and its Saturation row edits the midtones only
#[test]
fn color_correct_midtones_card_edits_its_own_tone() {
    let (mut harness, effect) = app_with("colorCorrect", "content", WINDOW_HEIGHT);
    let midtones = harness.get_by_role_and_label(Role::Button, "Midtones").rect();
    let highlights = harness.get_by_role_and_label(Role::Button, "Highlights").rect();
    let track = harness
        .get_all_by_role_and_label(Role::Slider, "Saturation")
        .map(|node| node.rect())
        .find(|rect| rect.top() > midtones.bottom() && rect.bottom() < highlights.top())
        .unwrap();
    click_at(&mut harness, along(track, 0.0));

    let Filter::ColorCorrect { shadows, midtones, .. } = filter_of(&harness, effect) else { panic!() };
    assert!(midtones[2] < 0.05 && shadows[2] == 0.5, "the Midtones card's Saturation: {midtones:?} {shadows:?}");
}

#[test]
fn blur_wrap_switches_the_edge_wrap() {
    let (mut harness, effect) = app_with("blur", "content", WINDOW_HEIGHT);
    let wrap = harness.get_by_role_and_label(Role::CheckBox, "Blur Wrap").rect();

    click_at(&mut harness, wrap.center());

    assert!(matches!(filter_of(&harness, effect), Filter::Blur { wrap: true, .. }));
}
