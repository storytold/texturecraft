//! The Clone tool marks its source in the 2D view, forgets it when another texture set becomes
//! active, and has an Aligned option in its panel.

use egui::{Pos2, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

/// Radius of the crosshair's circle, and the stroke width of its coloured pass
const MARKER_RADIUS: f32 = 7.0;
const MARKER_STROKE: f32 = 1.2;

fn clone_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    execute(&mut harness.state_mut().session, "tool.clone", &serde_json::Value::Null).unwrap();
    harness.run_steps(4);
    harness
}

fn set_source(harness: &mut Harness<'_, TextureCraftApp>, uv: [f32; 2]) {
    execute(&mut harness.state_mut().session, "tool.cloneSource", &json!({ "uv": uv })).unwrap();
    harness.run_steps(3);
}

/// Centres of the crosshair circles in the last frame
fn markers(harness: &Harness<'_, TextureCraftApp>) -> Vec<Pos2> {
    fn walk(shape: &Shape, out: &mut Vec<Pos2>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            Shape::Circle(circle) if circle.radius == MARKER_RADIUS && (circle.stroke.width - MARKER_STROKE).abs() < 1e-3 => out.push(circle.center),
            _ => {}
        }
    }
    let mut out = Vec::new();
    harness.output().shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
    out
}

fn marker_at(harness: &Harness<'_, TextureCraftApp>) -> Pos2 {
    let found = markers(harness);
    assert_eq!(found.len(), 1, "one crosshair: {found:?}");
    found[0]
}

#[test]
fn the_2d_view_draws_a_crosshair_at_the_clone_source() {
    let mut harness = clone_app();
    assert!(markers(&harness).is_empty(), "no source, no crosshair");
    set_source(&mut harness, [0.25, 0.5]);
    let left = marker_at(&harness);
    set_source(&mut harness, [0.5, 0.5]);
    let middle = marker_at(&harness);
    set_source(&mut harness, [0.75, 0.5]);
    let right = marker_at(&harness);
    assert!((left.y - middle.y).abs() < 0.5 && (right.y - middle.y).abs() < 0.5, "same texture row: {left:?} {middle:?} {right:?}");
    assert!(middle.x - left.x > 50.0, "the crosshair follows the source in u: {left:?} {middle:?}");
    assert!(((middle.x - left.x) - (right.x - middle.x)).abs() < 0.5, "evenly spaced in u: {left:?} {middle:?} {right:?}");
    set_source(&mut harness, [0.5, 0.25]);
    let top = marker_at(&harness);
    assert!((top.x - middle.x).abs() < 0.5 && top.y < middle.y - 50.0, "v runs upwards on screen: {top:?} {middle:?}");
}

#[test]
fn the_2d_view_draws_the_crosshair_for_a_source_on_the_far_edge() {
    let mut harness = clone_app();
    set_source(&mut harness, [1.0, 0.5]);
    let edge = marker_at(&harness);
    set_source(&mut harness, [0.5, 0.5]);
    let middle = marker_at(&harness);
    assert!(edge.x > middle.x + 50.0, "the edge source sits right of the middle: {edge:?} {middle:?}");
    set_source(&mut harness, [0.5, 1.0]);
    let bottom = marker_at(&harness);
    assert!(bottom.y > middle.y + 50.0, "v = 1.0 shows too: {bottom:?} {middle:?}");
}

#[test]
fn the_2d_crosshair_only_shows_for_the_clone_tool() {
    let mut harness = clone_app();
    set_source(&mut harness, [0.5, 0.5]);
    assert_eq!(markers(&harness).len(), 1);
    execute(&mut harness.state_mut().session, "tool.paint", &serde_json::Value::Null).unwrap();
    harness.run_steps(3);
    assert!(markers(&harness).is_empty(), "the paint tool shows no source");
}

#[test]
fn switching_the_texture_set_removes_the_clone_source_and_its_crosshair() {
    let mut harness = clone_app();
    set_source(&mut harness, [0.5, 0.5]);
    assert_eq!(markers(&harness).len(), 1);
    let other = if harness.state().session.ts() == 0 { 1 } else { 0 };
    execute(&mut harness.state_mut().session, "textureSet.select", &json!({ "ts": other })).unwrap();
    harness.run_steps(3);
    assert!(harness.state().session.tool.clone_source.is_none());
    assert!(markers(&harness).is_empty(), "no crosshair for a source of the other set");
}

#[test]
fn the_aligned_checkbox_sets_the_clone_mode() {
    let mut harness = clone_app();
    assert!(harness.state().session.tool.clone_aligned, "aligned by default");
    harness.get_by_label("Aligned").click();
    harness.run_steps(3);
    assert!(!harness.state().session.tool.clone_aligned);
    harness.get_by_label("Aligned").click();
    harness.run_steps(3);
    assert!(harness.state().session.tool.clone_aligned);
}
