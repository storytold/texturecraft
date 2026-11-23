//! Ctrl+drag in the 3D view changes the brush instead of painting or orbiting.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use texturecraft_ui_egui::TextureCraftApp;

/// Drag with `button` from `from` by `by` in a few moves.
fn drag(h: &mut Harness<'_, TextureCraftApp>, button: PointerButton, from: Pos2, by: egui::Vec2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.hover_at(from);
    h.event(Event::PointerButton { pos: from, button, pressed: true, modifiers });
    h.run_steps(2);
    for k in 1..=4 {
        h.hover_at(from + by * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: from + by, button, pressed: false, modifiers });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

#[test]
fn ctrl_drag_changes_the_brush_instead_of_painting_or_orbiting() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    let c = h.state().viewport_rect.center();
    let (brush, yaw, history) = {
        let s = &h.state().session;
        (s.tool.brush.clone(), s.camera.yaw, s.history_labels().len())
    };

    // Right drag to the right and up: a bigger, harder brush; the camera stays
    drag(&mut h, PointerButton::Secondary, c, vec2(80.0, -40.0), Modifiers::COMMAND);
    let b = h.state().session.tool.brush.clone();
    assert!(b.size > brush.size && b.hardness > brush.hardness, "{} {} → {} {}", brush.size, brush.hardness, b.size, b.hardness);
    assert_eq!(h.state().session.camera.yaw, yaw, "no orbit");

    // Left drag to the left and up: less flow, a turned brush, and no stroke
    drag(&mut h, PointerButton::Primary, c, vec2(-80.0, -40.0), Modifiers::COMMAND);
    let b2 = h.state().session.tool.brush.clone();
    assert!(b2.flow < b.flow && b2.angle != b.angle, "{} {} → {} {}", b.flow, b.angle, b2.flow, b2.angle);
    assert_eq!(h.state().session.history_labels().len(), history, "nothing painted");

    // Without Ctrl, a right drag orbits again and the brush keeps its values
    drag(&mut h, PointerButton::Secondary, c, vec2(80.0, -40.0), Modifiers::NONE);
    assert_ne!(h.state().session.camera.yaw, yaw);
    assert_eq!(h.state().session.tool.brush, b2);
}

fn app(layout: texturecraft_ui_egui::state::ViewLayout) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.layout = layout;
    h.run_steps(4);
    h
}

/// One wheel notch of `delta_y` points with `modifiers` held, as egui gets it from the system.
fn wheel(h: &mut Harness<'_, TextureCraftApp>, at: Pos2, delta_y: f32, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.hover_at(at);
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, delta_y),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers,
    });
    h.run_steps(12);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn shift_wheel_changes_the_brush_size_in_the_3d_view_and_does_not_zoom() {
    let mut h = app(texturecraft_ui_egui::state::ViewLayout::Only3D);
    let c = h.state().viewport_rect.center();
    let (size, distance) = (h.state().session.tool.brush.size, h.state().session.camera.distance);
    wheel(&mut h, c, 20.0, Modifiers::SHIFT);
    let bigger = h.state().session.tool.brush.size;
    assert!(bigger > size, "{size} -> {bigger}");
    assert_eq!(h.state().session.camera.distance, distance, "no zoom");
    wheel(&mut h, c, -20.0, Modifiers::SHIFT);
    assert!(h.state().session.tool.brush.size < bigger);

    // Without Shift the wheel zooms and keeps the size
    let size = h.state().session.tool.brush.size;
    wheel(&mut h, c, 20.0, Modifiers::NONE);
    assert_eq!(h.state().session.tool.brush.size, size);
    assert_ne!(h.state().session.camera.distance, distance, "plain wheel zooms");
}

#[test]
fn shift_wheel_changes_the_brush_size_in_the_2d_view_and_does_not_zoom() {
    let mut h = app(texturecraft_ui_egui::state::ViewLayout::Only2D);
    let c = egui::pos2(800.0, 480.0);
    let (size, zoom) = (h.state().session.tool.brush.size, h.state().ui.uv_zoom);
    wheel(&mut h, c, 20.0, Modifiers::SHIFT);
    assert!(h.state().session.tool.brush.size > size);
    assert_eq!(h.state().ui.uv_zoom, zoom, "no zoom");
}

#[test]
fn shift_during_a_ctrl_right_drag_changes_size_and_hardness_in_fine_steps() {
    let mut h = app(texturecraft_ui_egui::state::ViewLayout::Only3D);
    let c = h.state().viewport_rect.center();
    let before = h.state().session.tool.brush.clone();
    let history = h.state().session.history_labels().len();

    // Compare Ctrl and Ctrl+Shift on the same right drag
    drag(&mut h, PointerButton::Secondary, c, vec2(10.0, -40.0), Modifiers::COMMAND);
    let coarse = h.state().session.tool.brush.clone();
    h.state_mut().session.tool.brush = before.clone();
    h.run_steps(2);
    drag(&mut h, PointerButton::Secondary, c, vec2(10.0, -40.0), Modifiers::COMMAND | Modifiers::SHIFT);
    let fine = h.state().session.tool.brush.clone();
    assert_eq!(h.state().session.history_labels().len(), history, "nothing painted");
    let (coarse_hardness, fine_hardness) = (coarse.hardness - before.hardness, fine.hardness - before.hardness);
    assert!(coarse_hardness > 0.05 && fine_hardness > 0.0, "{coarse_hardness} {fine_hardness}");
    assert!((fine_hardness * 10.0 - coarse_hardness).abs() < 0.01, "hardness {coarse_hardness} vs {fine_hardness}");
    let (coarse_size, fine_size) = (coarse.size / before.size - 1.0, fine.size / before.size - 1.0);
    assert!(coarse_size > 0.01 && fine_size > 0.0 && fine_size < coarse_size / 5.0, "size {coarse_size} vs {fine_size}");
}

#[test]
fn ctrl_shift_left_drag_does_not_change_flow_or_angle() {
    let mut h = app(texturecraft_ui_egui::state::ViewLayout::Only3D);
    let c = h.state().viewport_rect.center();
    let before = h.state().session.tool.brush.clone();
    drag(&mut h, PointerButton::Primary, c, vec2(-80.0, -40.0), Modifiers::COMMAND | Modifiers::SHIFT);
    let after = &h.state().session.tool.brush;
    assert_eq!((after.flow, after.angle), (before.flow, before.angle));
}

#[test]
fn ctrl_wheel_sizes_the_brush_in_both_views_up_smaller_and_does_not_zoom() {
    for layout in [texturecraft_ui_egui::state::ViewLayout::Only3D, texturecraft_ui_egui::state::ViewLayout::Only2D] {
        let mut h = app(layout);
        let c = if layout == texturecraft_ui_egui::state::ViewLayout::Only3D { h.state().viewport_rect.center() } else { egui::pos2(800.0, 480.0) };
        let (size, distance, zoom) = (h.state().session.tool.brush.size, h.state().session.camera.distance, h.state().ui.uv_zoom);
        wheel(&mut h, c, 20.0, Modifiers::COMMAND);
        let smaller = h.state().session.tool.brush.size;
        assert!(smaller < size, "{layout:?}: wheel up makes it smaller: {size} -> {smaller}");
        wheel(&mut h, c, -20.0, Modifiers::COMMAND);
        assert!(h.state().session.tool.brush.size > smaller, "{layout:?}: wheel down makes it larger");
        assert_eq!(h.state().session.camera.distance, distance, "{layout:?}: no camera zoom");
        assert_eq!(h.state().ui.uv_zoom, zoom, "{layout:?}: no 2D zoom");
    }
}

#[test]
fn a_trackpad_pinch_zooms_each_view_and_keeps_the_brush_size() {
    use texturecraft_ui_egui::state::ViewLayout;
    for layout in [ViewLayout::Only3D, ViewLayout::Only2D] {
        let mut h = app(layout);
        let at = if layout == ViewLayout::Only3D { h.state().viewport_rect.center() } else { egui::pos2(800.0, 480.0) };
        let size = h.state().session.tool.brush.size;
        let distance = h.state().session.camera.distance;
        let zoom = h.state().ui.uv_zoom;

        h.hover_at(at);
        h.event(Event::Zoom(1.25));
        h.run_steps(2);

        assert_eq!(h.state().session.tool.brush.size, size);
        if layout == ViewLayout::Only3D {
            assert!(h.state().session.camera.distance < distance);
        } else {
            assert!(h.state().ui.uv_zoom > zoom);
        }
    }
}
