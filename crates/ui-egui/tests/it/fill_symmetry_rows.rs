//! A fill's Symmetry section: its switch, Type, Axis, Flip copy and the radial rows each change the fill's symmetry

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{FillSymmetry, LayerId, SymmetryKind};
use texturecraft_ui_egui::TextureCraftApp;

use crate::fill_support::{click_label, click_node, fill_app, fill_of};
use crate::filter_support::{along, click_at, drag, undo_depth};
use crate::{PROPERTIES_WINDOW, SETTLE_FRAMES};

/// The app with a new fill layer (projected as `projection`) selected, its Symmetry tab clicked, and its layer id
fn app_with_fill(projection: &str) -> (Harness<'static, TextureCraftApp>, u64) {
    let (harness, LayerId(layer)) = fill_app(json!({}), Some(projection), Some("Symmetry"), PROPERTIES_WINDOW);
    (harness, layer)
}

/// The fill layer's symmetry as the document holds it
fn symmetry(harness: &Harness<'_, TextureCraftApp>, layer: u64) -> FillSymmetry {
    fill_of(harness, LayerId(layer)).sampling.symmetry
}

/// The app with a planar fill's radial symmetry on (six copies over a full turn) and its radial rows showing
fn app_with_radial() -> (Harness<'static, TextureCraftApp>, u64) {
    let (mut harness, layer) = app_with_fill("planar");
    harness.state_mut().run("layer.fill.projection", json!({"layer": layer, "symmetry": true, "symmetryType": "radial"}));
    harness.run_steps(SETTLE_FRAMES);
    (harness, layer)
}

/// The Count track's start: the fewest copies
const COUNT_TRACK_FEWEST: f32 = *FillSymmetry::COUNTS.start() as f32;

/// The Count track's end; a typed count reaches further
const COUNT_TRACK_MOST: f32 = 16.0;

/// Where the Count track shows `count` copies
fn count_at(harness: &Harness<'_, TextureCraftApp>, count: f32) -> egui::Pos2 {
    let track = harness.get_by_role_and_label(Role::Slider, "Count").rect();
    along(track, (count - COUNT_TRACK_FEWEST) / (COUNT_TRACK_MOST - COUNT_TRACK_FEWEST))
}

#[test]
fn the_symmetry_switch_shows_its_rows_and_each_row_changes_the_symmetry() {
    let (mut harness, layer) = app_with_fill("planar");

    assert!(harness.query_by_label("Mirror symmetry").is_none(), "off, the section is the caption alone");

    click_node(&mut harness, Role::CheckBox, "Symmetry");

    assert!(symmetry(&harness, layer).on);
    assert!(harness.query_by_label("Axis position X").is_some(), "a mirror across X moves along X");
    assert!(harness.query_by_label("Count").is_none(), "a mirror has no radial rows");

    click_node(&mut harness, Role::RadioButton, "Y");

    assert_eq!(symmetry(&harness, layer).axis, 1);

    click_node(&mut harness, Role::CheckBox, "Flip copy U");

    assert_eq!(symmetry(&harness, layer).flip, [true, false]);

    click_node(&mut harness, Role::ComboBox, "Mirror symmetry");
    click_node(&mut harness, Role::ListBoxOption, "Radial symmetry");

    assert_eq!(symmetry(&harness, layer).kind, SymmetryKind::Radial);

    for label in ["Count", "Angle span", "Axis position X", "Axis position Z"] {
        assert!(harness.query_by_label(label).is_some(), "radial round Y: {label}");
    }

    assert!(harness.query_by_label("Axis position Y").is_none(), "the turning axis does not move along itself");

    click_node(&mut harness, Role::CheckBox, "Symmetry");

    assert!(!symmetry(&harness, layer).on);
    assert_eq!(symmetry(&harness, layer).kind, SymmetryKind::Radial, "switched off, the settings stay");
}

#[test]
fn a_uv_fill_says_why_in_place_of_the_symmetry_switch() {
    let (mut harness, layer) = app_with_fill("uv");

    assert!(harness.query_by_role_and_label(Role::CheckBox, "Symmetry").is_none(), "no switch to click");

    click_label(&mut harness, "Not available with UV projection");

    assert!(!symmetry(&harness, layer).on, "a UV projection has no symmetry to switch on");
    assert!(harness.query_by_label("Type").is_none(), "its settings stay hidden, not greyed");
}

/// A click on the Count track that rounds to the count it has changes nothing and adds no undo step; a drag to the next whole count is one step
#[test]
fn a_count_click_that_keeps_the_count_adds_no_undo_step_and_a_drag_to_the_next_adds_one() {
    let (mut harness, layer) = app_with_radial();
    let steps = undo_depth(&harness);

    let inside = count_at(&harness, 6.2);
    click_at(&mut harness, inside);

    assert_eq!(symmetry(&harness, layer).count, 6);
    assert_eq!(undo_depth(&harness), steps, "a count that stays six is no edit");

    let (from, to) = (count_at(&harness, 6.0), count_at(&harness, 7.3));
    drag(&mut harness, from, to);

    assert_eq!(symmetry(&harness, layer).count, 7);
    assert_eq!(undo_depth(&harness), steps + 1, "the drag's frames merge into one step");
}

/// A typed count reaches past the track's end, up to the engine's most copies, as the symmetry popup's Count does
#[test]
fn a_typed_count_reaches_past_the_track_to_the_engines_most() {
    let (mut harness, layer) = app_with_radial();

    click_node(&mut harness, Role::Label, "Count value");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("40".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(symmetry(&harness, layer).count, 40);
}

fn commit_value_by_clicking_track(harness: &mut Harness<'_, TextureCraftApp>, value_label: &str, value: &str, track_label: &str) {
    click_node(harness, Role::Label, value_label);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(value.into()));
    harness.run_steps(SETTLE_FRAMES);

    let track = harness.get_by_role_and_label(Role::Slider, track_label).rect();
    let pos = along(track, 0.75);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    harness.input_mut().events.extend([egui::Event::PointerMoved(pos), button(true), button(false)]);
    harness.step();
}

#[test]
fn typing_one_radial_offset_and_clicking_the_other_keeps_both_edits() {
    let (mut harness, layer) = app_with_radial();
    harness.state_mut().run("layer.fill.projection", json!({"layer": layer, "symmetryAxis": 1}));
    harness.run_steps(SETTLE_FRAMES);
    let undo_before = undo_depth(&harness);

    commit_value_by_clicking_track(&mut harness, "Axis position X value", "0.25", "Axis position Z");

    let [x, y, z] = symmetry(&harness, layer).position;

    assert_eq!(x, 0.25, "losing focus commits the typed X offset");
    assert_eq!(y, 0.0, "the radial axis does not move along itself");
    assert!(z > 0.0, "the same click edits the Z offset");
    assert_eq!(undo_depth(&harness), undo_before + 1);
}

#[test]
fn typing_count_and_clicking_angle_span_keeps_both_edits() {
    let (mut harness, layer) = app_with_radial();
    let undo_before = undo_depth(&harness);

    commit_value_by_clicking_track(&mut harness, "Count value", "40", "Angle span");

    let symmetry = symmetry(&harness, layer);

    assert_eq!(symmetry.count, 40, "losing focus commits the typed count");
    assert!(symmetry.span < 360.0, "the same click edits the angle span");
    assert_eq!(undo_depth(&harness), undo_before + 1);
}
