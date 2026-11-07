//! Polygon fill in the views: a click fills one target and a box drag fills what the box selects, each as one undo step

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use texturecraft_engine::Tool;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;

/// The sample project in the default split layout at 1600 x 960 with the polygon fill tool
fn polygon_fill_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().session.tool.tool = Tool::PolygonFill;
    harness.run_steps(4);
    harness
}

/// Drag the primary button from `from` to `to` in a few moves
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);

    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run_steps(1);
    }

    // Only the release frame runs, so the release itself must finish the drag
    harness.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(1);
}

/// Press and release the primary button at `at`
fn click(harness: &mut Harness<'_, TextureCraftApp>, at: Pos2) {
    harness.hover_at(at);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);
}

/// Allocated tiles in the selected layer's base color
fn painted_tiles(harness: &Harness<'_, TextureCraftApp>) -> usize {
    let session = &harness.state().session;
    let layer = session.selected_layer().expect("a selected layer");
    session.doc.pixels.layer(layer, Channel::BaseColor).map_or(0, |image| image.allocated_count())
}

/// The undo steps recorded so far
fn history(harness: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    harness.state().session.history_labels()
}

#[test]
fn a_box_drag_in_the_3d_view_is_one_fill_over_more_than_a_click() {
    let mut clicked = polygon_fill_app();
    let center = clicked.state().viewport_rect.center();
    let before = history(&clicked).len();
    click(&mut clicked, center);
    assert_eq!(history(&clicked).len(), before + 1, "a click fills one triangle");

    let mut boxed = polygon_fill_app();
    drag(&mut boxed, center - vec2(120.0, 120.0), center + vec2(120.0, 120.0));
    assert_eq!(history(&boxed).len(), before + 1);
    assert_eq!(history(&boxed).last().map(String::as_str), Some("Polygon fill"));
    assert!(
        painted_tiles(&boxed) > painted_tiles(&clicked),
        "the box fills every triangle in it: {} tiles vs {}",
        painted_tiles(&boxed),
        painted_tiles(&clicked)
    );
}

#[test]
fn a_box_drag_in_the_2d_view_is_one_fill() {
    let mut harness = polygon_fill_app();
    let view_3d = harness.state().viewport_rect;
    let center = pos2(view_3d.right() + view_3d.width() * 0.5, view_3d.center().y);
    let before = history(&harness).len();

    drag(&mut harness, center - vec2(80.0, 20.0), center + vec2(80.0, 20.0));
    assert_eq!(history(&harness).len(), before + 1);
    assert_eq!(history(&harness).last().map(String::as_str), Some("Polygon fill"));
}
