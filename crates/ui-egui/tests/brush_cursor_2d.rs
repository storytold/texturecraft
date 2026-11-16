//! The 2D view hides the system pointer and draws the brush outline under it while painting.

use egui::{CursorIcon, Event, Pos2, Shape, TouchDeviceId, TouchId, TouchPhase};
use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{Mode, ViewLayout};

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(4);
    harness
}

fn cursor(harness: &Harness<'_, TextureCraftApp>) -> CursorIcon {
    harness.output().platform_output.cursor_icon
}

#[test]
fn painting_in_the_2d_view_replaces_the_pointer_with_the_brush_outline() {
    let mut harness = harness();
    let center = egui::pos2(800.0, 480.0);
    harness.hover_at(center);
    harness.run_steps(3);
    assert_eq!(cursor(&harness), CursorIcon::None, "the brush outline replaces the pointer");
    harness.hover_at(egui::pos2(2.0, 2.0));
    harness.run_steps(3);
    assert_ne!(cursor(&harness), CursorIcon::None, "outside the view the pointer is back");
}

#[test]
fn outside_painting_the_2d_view_keeps_the_normal_pointer() {
    let mut harness = harness();
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(3);
    let center = egui::pos2(800.0, 480.0);
    harness.hover_at(center);
    harness.run_steps(3);
    assert_ne!(cursor(&harness), CursorIcon::None);
}

#[test]
fn unavailable_paint_tool_keeps_the_pointer_and_hides_the_2d_brush_outline() {
    let mut harness = harness();
    let fill = execute(&mut harness.state_mut().session, "layer.add.fill", &json!({})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": fill, "target": "content"})).unwrap();

    let pointer = egui::pos2(800.0, 480.0);
    harness.hover_at(pointer);
    harness.run_steps(3);

    assert_ne!(cursor(&harness), CursorIcon::None, "a fill layer cannot take the paint tool");
    assert!(rings_at(&harness, pointer).is_empty(), "an unavailable brush has no outline");
}

/// Radii of the white brush-outline rings (stroke 1.2) around `center` in the last frame.
fn rings_at(harness: &Harness<'_, TextureCraftApp>, center: Pos2) -> Vec<f32> {
    fn walk(shape: &Shape, center: Pos2, out: &mut Vec<f32>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, center, out)),
            Shape::Circle(circle) if (circle.stroke.width - 1.2).abs() < 1e-3 && circle.center.distance(center) < 0.5 => out.push(circle.radius),
            _ => {}
        }
    }
    let mut out = Vec::new();
    harness.output().shapes.iter().for_each(|clipped| walk(&clipped.shape, center, &mut out));
    out
}

#[test]
fn the_view_header_keeps_the_normal_pointer_and_gets_no_brush_outline() {
    let mut harness = harness();
    // The header starts 8 points below the view edge and is 22 points high
    let view_top = (0..200).map(|y| y as f32).find(|&y| {
        harness.hover_at(egui::pos2(300.0, y));
        harness.run_steps(2);
        cursor(&harness) == CursorIcon::None
    });
    let top = view_top.expect("the view has a top edge");
    let header = egui::pos2(800.0, top + 8.0 + 11.0);
    harness.hover_at(header);
    harness.run_steps(3);
    assert_ne!(cursor(&harness), CursorIcon::None, "over the pickers the pointer stays");
    assert!(rings_at(&harness, header).is_empty(), "no brush outline over the pickers");
    let below = egui::pos2(800.0, top + 120.0);
    harness.hover_at(below);
    harness.run_steps(3);
    assert_eq!(cursor(&harness), CursorIcon::None, "below the header the brush outline is back");
    assert!(!rings_at(&harness, below).is_empty());
}

#[test]
fn a_light_pen_shows_the_smaller_footprint_it_paints() {
    let mut harness = harness();
    harness.state_mut().session.tool.brush.size = 0.05;
    harness.state_mut().session.tool.brush.min_size = 0.5;
    let center = egui::pos2(800.0, 600.0);
    harness.hover_at(center);
    harness.run_steps(3);
    let full = rings_at(&harness, center);
    assert_eq!(full.len(), 1, "one outline at full pressure");
    // Half pressure gives 0.75 of the full radius with this minimum size
    let events = &mut harness.input_mut().events;
    events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Move, pos: center, force: Some(0.5) });
    events.push(Event::PointerMoved(center));
    harness.step();
    let light = rings_at(&harness, center);
    assert_eq!(light.len(), 1);
    assert!((light[0] / full[0] - 0.75).abs() < 1e-3, "half pressure shows 0.75 of the radius: {} vs {}", light[0], full[0]);
}

#[test]
fn dragging_out_of_the_2d_view_brings_the_system_pointer_back() {
    let mut harness = harness();
    let center = egui::pos2(800.0, 480.0);
    harness.hover_at(center);
    harness.run_steps(3);
    assert_eq!(cursor(&harness), CursorIcon::None, "inside the view the brush outline replaces the pointer");
    // Drag from the view to its upper-left outside corner
    harness.event(Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    harness.run_steps(2);
    assert_eq!(cursor(&harness), CursorIcon::None, "still inside while dragging");
    let outside = egui::pos2(2.0, 2.0);
    harness.hover_at(outside);
    harness.run_steps(3);
    assert_ne!(cursor(&harness), CursorIcon::None, "outside the view the pointer is visible even during a drag");
    assert!(rings_at(&harness, outside).is_empty(), "no brush outline outside the view");
    harness.event(Event::PointerButton { pos: outside, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    harness.run_steps(2);
}

#[test]
fn a_ctrl_right_drag_out_of_the_2d_view_keeps_its_outline_at_the_anchor() {
    let mut harness = harness();
    let center = egui::pos2(800.0, 480.0);
    let ctrl = egui::Modifiers::COMMAND;
    harness.hover_at(center);
    harness.run_steps(2);
    harness.event(Event::ModifiersChanged(ctrl));
    harness.event(Event::PointerButton { pos: center, button: egui::PointerButton::Secondary, pressed: true, modifiers: ctrl });
    harness.run_steps(2);
    // The outline stays anchored while the pointer is hidden inside the view
    harness.hover_at(center + egui::vec2(10.0, 0.0));
    harness.run_steps(3);
    assert_eq!(rings_at(&harness, center).len(), 1, "anchored outline while the pointer is in the view");
    assert_eq!(cursor(&harness), CursorIcon::None);
    // Outside the view, the outline stays anchored and the system pointer returns
    harness.hover_at(egui::pos2(2.0, 2.0));
    harness.run_steps(3);
    assert_eq!(rings_at(&harness, center).len(), 1, "the outline stays at the press origin outside the view");
    assert_ne!(cursor(&harness), CursorIcon::None, "outside the view the pointer is visible");
    harness.event(Event::PointerButton { pos: egui::pos2(2.0, 2.0), button: egui::PointerButton::Secondary, pressed: false, modifiers: ctrl });
    harness.event(Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run_steps(2);
}
