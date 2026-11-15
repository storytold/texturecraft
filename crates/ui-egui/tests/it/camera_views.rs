//! F5 and F6 switch the projection, the Viewport menu goes to fixed views, and Alt+Shift+drag orbits
//! in 90° steps.

use std::f32::consts::FRAC_PI_2;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'_, TextureCraftApp>, key: Key) {
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// Drag with the left button and `modifiers` from `from` by `by` in a few moves.
fn drag(h: &mut Harness<'_, TextureCraftApp>, from: Pos2, by: egui::Vec2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.hover_at(from);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(2);
    for k in 1..=4 {
        h.hover_at(from + by * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: from + by, button: PointerButton::Primary, pressed: false, modifiers });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

#[test]
fn f6_switches_to_orthographic_and_f5_back_to_perspective_keeping_the_view() {
    let mut h = harness();
    h.state_mut().run("camera.set", json!({"yaw": 30, "pitch": 15}));
    h.run_steps(2);
    let before = h.state().session.camera;
    assert!(!before.ortho);
    press(&mut h, Key::F6);
    let camera = h.state().session.camera;
    assert!(camera.ortho, "F6 is orthographic");
    assert_eq!((camera.yaw, camera.pitch, camera.distance, camera.target), (before.yaw, before.pitch, before.distance, before.target));
    press(&mut h, Key::F6);
    assert!(h.state().session.camera.ortho, "F6 again stays orthographic: not a toggle");
    press(&mut h, Key::F5);
    assert_eq!(h.state().session.camera, before, "F5 is perspective");
}

#[test]
fn the_viewport_menu_goes_to_the_front_view_and_orbit_works_afterwards() {
    let mut h = harness();
    h.state_mut().run("camera.set", json!({"yaw": 70, "pitch": 30, "ortho": true}));
    h.run_steps(2);
    // The menu bar's "Viewport", above the dock's tab of the same name
    h.get_all_by_label("Viewport").min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    h.run_steps(2);
    h.get_by_label_contains("Front view").click();
    // The UI glides there over about a third of a second
    h.run_steps(6);
    let camera = h.state().session.camera;
    let looking_from = (camera.eye() - camera.target).normalize();
    assert!(looking_from.z > 0.999, "the eye is in front of the mesh: {looking_from}");
    assert!(camera.ortho, "the projection stays");

    let center = h.state().viewport_rect.center();
    drag(&mut h, center, vec2(60.0, 30.0), Modifiers::ALT);
    let orbited = h.state().session.camera;
    assert!(orbited.yaw != camera.yaw && orbited.pitch != camera.pitch, "Alt+drag still orbits");
}

#[test]
fn alt_shift_drag_orbits_in_quarter_turns() {
    let mut h = harness();
    h.state_mut().run("camera.set", json!({"yaw": 17, "pitch": 11}));
    h.run_steps(2);
    let center = h.state().viewport_rect.center();
    let at_step = |angle: f32, steps: f32| (angle - steps * FRAC_PI_2).abs() < 1e-5;

    // A short drag stays at the nearest quarter turn: straight on, level
    drag(&mut h, center, vec2(10.0, 5.0), Modifiers::ALT | Modifiers::SHIFT);
    let camera = h.state().session.camera;
    assert!(at_step(camera.yaw, 0.0) && at_step(camera.pitch, 0.0), "yaw {} pitch {}", camera.yaw.to_degrees(), camera.pitch.to_degrees());

    // A long drag to the left turns a quarter: the eye moves to the right side
    drag(&mut h, center, vec2(-200.0, 0.0), Modifiers::ALT | Modifiers::SHIFT);
    let camera = h.state().session.camera;
    assert!(at_step(camera.yaw, 1.0) && at_step(camera.pitch, 0.0), "yaw {} pitch {}", camera.yaw.to_degrees(), camera.pitch.to_degrees());

    // Dragging down far enough looks from above, at the orbit's 89° limit
    drag(&mut h, center, vec2(0.0, 200.0), Modifiers::ALT | Modifiers::SHIFT);
    let camera = h.state().session.camera;
    assert!(at_step(camera.yaw, 1.0) && (camera.pitch - 89f32.to_radians()).abs() < 1e-5, "pitch {}", camera.pitch.to_degrees());
}
