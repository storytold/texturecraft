//! The gradient filters in Properties: Gradient Map's rows (the ramp's markers, Color Quantity, Input Transformation, the color rows), Gradient Curve's
//! Preset list, Invert and Balance, and Gradient Dynamic's orientation and Gradient Input Position

use crate::filter_support::{WINDOW_HEIGHT, along, app_with, click_at, drag, drag_through, filter_of, lowest, topmost, undo, undo_depth};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{CurvePreset, Filter, GrayscaleMode};
use texturecraft_engine::filter_lib::GRADIENT_QUANTITY_MIN;

/// Dragging Color 2's marker along the ramp moves its place, and one undo puts it back
#[test]
fn a_gradient_marker_drags_its_color_along_the_ramp() {
    let (mut harness, effect) = app_with("gradientMap", "content", WINDOW_HEIGHT);

    // Color 2's row has a place track named Color 2 too: the ramp's marker is the topmost
    let marker = topmost(&harness, Role::Slider, "Color 2");
    drag(&mut harness, marker.center(), marker.center() + egui::vec2(60.0, 0.0));

    let Filter::GradientMap { positions, .. } = filter_of(&harness, effect) else { panic!() };
    let moved = positions.get(1).copied().unwrap();

    assert!(moved > 0.6, "dragged right: {positions:?}");

    undo(&mut harness);

    let Filter::GradientMap { positions, .. } = filter_of(&harness, effect) else { panic!() };
    assert_eq!(positions.get(1), Some(&0.5), "one undo step for the drag");
}

/// Color Quantity at its right end shows ten color rows
#[test]
fn color_quantity_adds_color_rows() {
    let (mut harness, effect) = app_with("gradientMap", "content", WINDOW_HEIGHT);
    assert!(harness.query_by_label("Color 4").is_none());

    let track = harness.get_by_role_and_label(Role::Slider, "Color Quantity").rect();
    click_at(&mut harness, along(track, 1.0));

    assert_eq!(filter_of(&harness, effect).ramp_colors().len(), 10);
    assert!(harness.query_all_by_label("Color 10").next().is_some(), "the tenth color has its row");
}

/// egui's click distance (`InputOptions::max_click_dist`): a press whose pointer travels further is a drag
const DRAG_THRESHOLD: f32 = 6.0;

/// A Color Quantity drag whose first change rounds to the count it had starts its own undo step: undo leaves the edit before it alone
#[test]
fn a_color_quantity_drag_is_its_own_undo_step() {
    let (mut harness, effect) = app_with("gradientMap", "content", WINDOW_HEIGHT);
    let linear = harness.get_by_role_and_label(Role::CheckBox, "Blend Color in Linear").rect();
    click_at(&mut harness, linear.center());

    assert!(matches!(filter_of(&harness, effect), Filter::GradientMap { linear: false, .. }));

    let depth = undo_depth(&harness);

    // From 2.6 colors to 3.2 (both round to the 3 it has), then to 4.2
    let track = harness.get_by_role_and_label(Role::Slider, "Color Quantity").rect();
    let (least, most) = (f32::from(GRADIENT_QUANTITY_MIN), f32::from(Filter::MAX_GRADIENT_COLORS));
    let at = |colors: f32| along(track, (colors - least) / (most - least));
    assert!(at(3.2).x - at(2.6).x > DRAG_THRESHOLD, "the first move already drags: {track:?}");

    drag_through(&mut harness, at(2.6), &[at(3.2), at(4.2)]);

    assert_eq!(filter_of(&harness, effect).ramp_colors().len(), 4);
    assert_eq!(undo_depth(&harness), depth + 1, "one undo step for the drag");

    undo(&mut harness);
    let filter = filter_of(&harness, effect);
    assert_eq!(filter.ramp_colors().len(), 3, "the drag undone");
    assert!(matches!(filter, Filter::GradientMap { linear: false, .. }), "Blend Color in Linear stays off: {filter:?}");
}

/// Input Transformation's Grayscale Mode list, Blur Intensity and Invert, Display Gradient (Debug), Blend Color in Linear and a color row's place track each set their own setting
#[test]
fn gradient_map_rows_set_their_own_setting() {
    let (mut harness, effect) = app_with("gradientMap", "content", WINDOW_HEIGHT);

    let list = harness.get_by_role_and_label(Role::ComboBox, "Average").rect();
    click_at(&mut harness, list.center());
    let max = harness.get_by_label("Max").rect();
    click_at(&mut harness, max.center());
    let blur = harness.get_by_role_and_label(Role::Slider, "Blur Intensity").rect();
    click_at(&mut harness, along(blur, 0.5));
    for toggle in ["Invert", "Display Gradient (Debug)", "Blend Color in Linear"] {
        let rect = harness.get_by_role_and_label(Role::CheckBox, toggle).rect();
        click_at(&mut harness, rect.center());
    }

    // Color 2's row is the lower of the two Color 2 tracks (the ramp's marker is the upper)
    let row = lowest(&harness, Role::Slider, "Color 2");
    click_at(&mut harness, along(row, 1.0));

    let Filter::GradientMap { input, debug, linear, positions, .. } = filter_of(&harness, effect) else { panic!("{:?}", filter_of(&harness, effect)) };
    assert_eq!((input.mode, input.invert, debug, linear), (GrayscaleMode::Max, true, true, false));
    assert!((input.blur - 8.0).abs() < 0.05, "the middle of 0..16: {}", input.blur);
    assert_eq!(positions.get(1), Some(&1.0), "Color 2 moved to the end: {positions:?}");
    assert_eq!((input.balance, input.contrast), (0.5, 0.0), "the others stay");
}

/// Gradient Curve's Preset list picks a curve
#[test]
fn gradient_curve_preset_list_picks_a_curve() {
    let (mut harness, effect) = app_with("gradientCurve", "content", WINDOW_HEIGHT);
    let list = harness.get_by_role_and_label(Role::ComboBox, "Round").rect();
    click_at(&mut harness, list.center());
    let bump = harness.get_by_label("Bump").rect();

    click_at(&mut harness, bump.center());

    assert!(matches!(filter_of(&harness, effect), Filter::GradientCurve { preset: CurvePreset::Bump, .. }), "{:?}", filter_of(&harness, effect));
}

/// Gradient Curve's Invert and Balance set their own setting
#[test]
fn gradient_curve_invert_and_balance() {
    let (mut harness, effect) = app_with("gradientCurve", "content", WINDOW_HEIGHT);

    let invert = harness.get_by_role_and_label(Role::CheckBox, "Invert").rect();
    click_at(&mut harness, invert.center());
    let balance = harness.get_by_role_and_label(Role::Slider, "Balance").rect();
    click_at(&mut harness, along(balance, 1.0));

    let Filter::GradientCurve { invert, balance, contrast, .. } = filter_of(&harness, effect) else { panic!("{:?}", filter_of(&harness, effect)) };
    assert!(invert && balance > 0.99 && contrast == 0.0, "{invert} {balance} {contrast}");
}

/// Gradient Dynamic's Gradient Input Position follows its track
#[test]
fn gradient_dynamic_input_position_follows_its_track() {
    let (mut harness, effect) = app_with("gradientDynamic", "content", WINDOW_HEIGHT);
    let position = harness.get_by_role_and_label(Role::Slider, "Gradient Input Position").rect();

    click_at(&mut harness, along(position, 0.5));

    assert!(
        matches!(filter_of(&harness, effect), Filter::GradientDynamic { position, .. } if (position - 0.5).abs() < 0.01),
        "{:?}",
        filter_of(&harness, effect)
    );
}

/// Gradient Dynamic's orientation turns vertical: a boxed switch at the panel's width, Vertical one click away
#[test]
fn gradient_dynamic_orientation_turns_vertical() {
    let (mut harness, effect) = app_with("gradientDynamic", "content", WINDOW_HEIGHT);
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Horizontal").is_none(), "both choices fit side by side");

    let vertical = harness.get_by_label("Vertical").rect();
    click_at(&mut harness, vertical.center());

    assert!(matches!(filter_of(&harness, effect), Filter::GradientDynamic { vertical: true, .. }), "{:?}", filter_of(&harness, effect));
}

#[test]
fn gradient_dynamic_source_card_opens_the_color_picker() {
    let (mut harness, _) = app_with("gradientDynamic", "content", WINDOW_HEIGHT);
    assert!(harness.query_by_role_and_label(Role::TitleBar, "Gradient Source").is_none(), "no picker yet");

    // Through the pointer, on the card's name: the card's whole row picks its source
    let card = harness.get_by_role_and_label(Role::ComboBox, "Gradient Source").rect();
    click_at(&mut harness, card.center());
    harness.run_steps(4);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Gradient Source").is_some(), "the card opens the colour picker of its bar");
}
