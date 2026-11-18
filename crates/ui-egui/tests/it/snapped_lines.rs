//! Ctrl+Shift+click paints a straight line from the last stroke with its angle snapped to 15 degree steps

use egui::{Event, Modifiers, PointerButton, Pos2, Vec2, pos2};
use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

/// Where the first, plain click lands in the 2D view
const FIRST_2D: Pos2 = pos2(700.0, 420.0);

/// Length of the test lines in points
const LINE_LENGTH: f32 = 160.0;

/// An angle 7 degrees past a step: it snaps back to 15 degrees, not on to 30
const CLICK_DEGREES: f32 = 22.0;

/// The step it snaps to
const SNAPPED_DEGREES: f32 = 15.0;

/// A line must differ at least this much from the unsnapped one (share of the snapped line's alpha)
const MIN_DIFFERENCE: f32 = 0.05;

/// The snapped line may differ at most this much from the reference (share of its alpha)
const MAX_DIFFERENCE: f32 = 0.01;

fn app(layout: ViewLayout) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.layout = layout;
    harness.run_steps(4);
    harness
}

fn ctrl_shift() -> Modifiers {
    Modifiers::COMMAND.plus(Modifiers::SHIFT)
}

/// A primary click at `pos` with `modifiers` held
fn click(harness: &mut Harness<'_, TextureCraftApp>, pos: Pos2, modifiers: Modifiers) {
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(pos);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(2);
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(1);
}

/// The point `length` away from `from` at `degrees` on screen
fn away(from: Pos2, degrees: f32, length: f32) -> Pos2 {
    from + Vec2::angled(degrees.to_radians()) * length
}

/// The selected layer's base color alpha in every texture set, set after set
fn alpha(harness: &mut Harness<'_, TextureCraftApp>) -> Vec<f32> {
    let sets = harness.state().session.doc.project.texture_sets.len();
    let mut all = Vec::new();
    for set in 0..sets {
        harness.state_mut().run("textureSet.select", json!({"ts": set}));
        let session = &harness.state().session;
        let (width, height) = session.doc.project.texture_sets[set].size();
        let layer = session.selected_layer().expect("a selected layer");
        let image = session.doc.pixels.layer(layer, Channel::BaseColor);
        all.extend(
            (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| image.and_then(|image| image.get(x, y).get(3).copied()).unwrap_or(0) as f32 / 65535.0),
        );
    }
    harness.state_mut().run("textureSet.select", json!({"ts": 0}));
    all
}

/// Total alpha painted between two snapshots
fn painted(before: &[f32], after: &[f32]) -> Vec<f32> {
    after.iter().zip(before).map(|(after, before)| (after - before).max(0.0)).collect()
}

/// Sum of the absolute differences of two paints
fn difference(first: &[f32], second: &[f32]) -> f32 {
    first.iter().zip(second).map(|(first, second)| (first - second).abs()).sum()
}

/// A plain click at `first`, then a click at `second` with `modifiers`; the paint the second click added
fn line_paint(layout: ViewLayout, first: Pos2, second: Pos2, modifiers: Modifiers) -> Vec<f32> {
    let mut harness = app(layout);
    click(&mut harness, first, Modifiers::NONE);
    let before = alpha(&mut harness);
    click(&mut harness, second, modifiers);
    painted(&before, &alpha(&mut harness))
}

/// Ctrl+Shift+click paints the line Shift+click paints to the end snapped to 15 degrees, and not the straight one to the click
fn assert_snaps(layout: ViewLayout, first: Pos2) {
    let clicked = away(first, CLICK_DEGREES, LINE_LENGTH);
    let snapped_end = away(first, SNAPPED_DEGREES, LINE_LENGTH);

    let snapped = line_paint(layout, first, clicked, ctrl_shift());
    let reference = line_paint(layout, first, snapped_end, Modifiers::SHIFT);
    let unsnapped = line_paint(layout, first, clicked, Modifiers::SHIFT);

    let total: f32 = reference.iter().sum();
    assert!(total > 1.0, "the reference line painted: {total}");
    assert!(difference(&snapped, &reference) < total * MAX_DIFFERENCE, "Ctrl+Shift+click ends on the 15 degree line: {}", difference(&snapped, &reference));
    assert!(
        difference(&unsnapped, &reference) > total * MIN_DIFFERENCE,
        "Shift+click without Ctrl keeps the clicked angle: {}",
        difference(&unsnapped, &reference)
    );
}

#[test]
fn ctrl_shift_click_snaps_the_line_to_15_degrees_in_the_2d_view() {
    assert_snaps(ViewLayout::Only2D, FIRST_2D);
}

#[test]
fn ctrl_shift_click_snaps_the_line_to_15_degrees_in_the_3d_view() {
    let center = app(ViewLayout::Only3D).state().viewport_rect.center() - Vec2::new(80.0, 40.0);
    assert_snaps(ViewLayout::Only3D, center);
}

#[test]
fn ctrl_shift_click_snaps_to_negative_angles_in_the_2d_view() {
    let first = pos2(700.0, 560.0);
    let clicked = away(first, -CLICK_DEGREES, LINE_LENGTH);
    let snapped = line_paint(ViewLayout::Only2D, first, clicked, ctrl_shift());
    let reference = line_paint(ViewLayout::Only2D, first, away(first, -SNAPPED_DEGREES, LINE_LENGTH), Modifiers::SHIFT);
    let total: f32 = reference.iter().sum();
    assert!(total > 1.0, "the reference line painted: {total}");
    assert!(difference(&snapped, &reference) < total * MAX_DIFFERENCE);
}

#[test]
fn ctrl_shift_click_without_a_previous_stroke_paints_a_dot_like_a_click() {
    let mut harness = app(ViewLayout::Only2D);
    let blank = alpha(&mut harness);
    click(&mut harness, FIRST_2D, ctrl_shift());
    let dot = painted(&blank, &alpha(&mut harness));

    let mut reference = app(ViewLayout::Only2D);
    click(&mut reference, FIRST_2D, Modifiers::NONE);
    let plain = painted(&blank, &alpha(&mut reference));
    assert!(plain.iter().sum::<f32>() > 1.0);
    assert!(difference(&dot, &plain) < plain.iter().sum::<f32>() * MAX_DIFFERENCE);
}

#[test]
fn ctrl_shift_click_paints_instead_of_changing_the_brush() {
    let mut harness = app(ViewLayout::Only2D);
    let (brush, steps) = (harness.state().session.tool.brush.clone(), harness.state().session.history_labels().len());
    click(&mut harness, FIRST_2D, ctrl_shift());
    assert_eq!(harness.state().session.history_labels().len(), steps + 1, "the click painted");
    assert_eq!(harness.state().session.tool.brush.size, brush.size);
    assert_eq!(harness.state().session.tool.brush.flow, brush.flow);
}

#[test]
fn ctrl_shift_right_drag_still_changes_the_brush() {
    let mut harness = app(ViewLayout::Only3D);
    let size = harness.state().session.tool.brush.size;
    let from = harness.state().viewport_rect.center();
    let modifiers = ctrl_shift();
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Secondary, pressed: true, modifiers });
    harness.run_steps(2);
    for step in 1..=4 {
        harness.hover_at(from + Vec2::new(20.0 * step as f32, 0.0));
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: from + Vec2::new(80.0, 0.0), button: PointerButton::Secondary, pressed: false, modifiers });
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(3);
    assert!(harness.state().session.tool.brush.size > size, "a Ctrl+Shift right drag to the right enlarges the brush");
}
