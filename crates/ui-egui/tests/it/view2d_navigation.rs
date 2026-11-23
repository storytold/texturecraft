//! The 2D view navigates with the camera mouse bindings: Alt+left turns it (Alt+Shift snaps), Alt+right drags the zoom,
//! Alt+middle and Ctrl+Alt+left pan 1:1, F resets pan, zoom and rotation, and the zoom reaches the reference's limits.

use std::f32::consts::{FRAC_PI_2, TAU};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use texturecraft_engine::Session;
use texturecraft_engine::geom::primitives;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

const WINDOW: egui::Vec2 = vec2(1280.0, 800.0);

/// The app showing only the 2D view, and the rect that view fills
fn app_with(session: Session) -> (Harness<'static, TextureCraftApp>, Rect) {
    let mut harness =
        Harness::builder().with_size(WINDOW).with_pixels_per_point(1.0).with_step_dt(1.0 / 60.0).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().session.tool.tool = texturecraft_engine::Tool::Paint;
    harness.state_mut().ui.layout = ViewLayout::Only3D;
    harness.run_steps(4);
    let view = harness.state().viewport_rect;
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(4);
    (harness, view)
}

fn app() -> (Harness<'static, TextureCraftApp>, Rect) {
    app_with(texturecraft_engine::sample_session(256, 64).unwrap())
}

/// A project with two UV tiles, 1001 and 1002
fn tiled_app() -> (Harness<'static, TextureCraftApp>, Rect) {
    app_with(Session::new_project("Tiles", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap())
}

/// Drag with `button` and `modifiers` held from `from` by `by` in a few moves
fn drag(harness: &mut Harness<'_, TextureCraftApp>, button: PointerButton, modifiers: Modifiers, from: Pos2, by: egui::Vec2) {
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
    harness.run_steps(2);
    harness.event(Event::PointerButton { pos: from, button, pressed: true, modifiers });
    harness.run_steps(2);
    for step in 1..=4 {
        harness.hover_at(from + by * (step as f32 / 4.0));
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: from + by, button, pressed: false, modifiers });
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(3);
}

fn wheel(delta: egui::Vec2) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    }
}

fn press(harness: &mut Harness<'_, TextureCraftApp>, key: Key) {
    harness.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    harness.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
}

/// The base color tiles the selected layer has painted, as a check that a drag painted nothing
fn painted(harness: &Harness<'_, TextureCraftApp>) -> (Vec<String>, usize) {
    let session = &harness.state().session;
    let layer = session.selected_layer().expect("a selected layer");
    (session.history_labels(), session.doc.pixels.layer(layer, texturecraft_engine::core::Channel::BaseColor).map_or(0, |image| image.allocated_count()))
}

/// A point on the view clear of the corner buttons and the header
fn start(view: Rect) -> Pos2 {
    view.center() + vec2(40.0, 60.0)
}

#[test]
fn polygon_fill_draws_the_rotated_uv_bounds_between_the_drag_corners() {
    let (mut harness, view) = tiled_app();
    harness.state_mut().session.tool.tool = texturecraft_engine::Tool::PolygonFill;
    harness.state_mut().ui.uv_rotation = 0.6;
    let origin = view.center() - vec2(50.0, 40.0);
    let opposite = view.center() + vec2(70.0, 20.0);
    let tint = texturecraft_ui_egui::theme::Tokens::get(&harness.ctx).viewport_accent.gamma_multiply(0.15);
    harness.hover_at(origin);
    harness.run_steps(2);
    harness.event(Event::PointerButton { pos: origin, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.step();

    harness.hover_at(opposite);
    harness.run_steps(2);

    let points = harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.fill == tint && path.closed && path.points.len() == 4 => Some(&path.points),
            _ => None,
        })
        .expect("the fill preview draws a rotated quadrilateral");
    assert!([origin, opposite].iter().all(|endpoint| points.iter().any(|point| point.distance(*endpoint) < 0.01)));
    let edge = points[1] - points[0];
    assert!(edge.x.abs() > 1.0 && edge.y.abs() > 1.0);
}

#[test]
fn alt_left_drag_turns_the_view_and_does_not_pan_or_paint() {
    let (mut harness, view) = app();
    let before = (harness.state().ui.uv_pan, harness.state().ui.uv_zoom, painted(&harness));
    assert_eq!(harness.state().ui.uv_rotation, 0.0);

    drag(&mut harness, PointerButton::Primary, Modifiers::ALT, start(view), vec2(80.0, 10.0));

    let ui = &harness.state().ui;
    assert!(ui.uv_rotation != 0.0, "the view turned");
    assert_eq!((ui.uv_pan, ui.uv_zoom, painted(&harness)), before, "no pan, no zoom, no paint");
}

#[test]
fn alt_left_drag_to_the_right_and_back_returns_to_the_start() {
    let (mut harness, view) = app();
    drag(&mut harness, PointerButton::Primary, Modifiers::ALT, start(view), vec2(80.0, 0.0));
    let turned = harness.state().ui.uv_rotation;
    assert!(turned != 0.0);
    drag(&mut harness, PointerButton::Primary, Modifiers::ALT, start(view), vec2(-80.0, 0.0));
    let ui = &harness.state().ui;
    assert!(ui.uv_rotation.min(TAU - ui.uv_rotation) < 1e-3, "back at 0: {}", ui.uv_rotation);
}

#[test]
fn alt_shift_left_drag_turns_in_quarter_steps() {
    let (mut harness, view) = app();

    // The reference: a 300 x 200 point drag gave exactly 180 degrees
    drag(&mut harness, PointerButton::Primary, Modifiers::ALT | Modifiers::SHIFT, start(view), vec2(300.0, 200.0));
    assert!((harness.state().ui.uv_rotation - std::f32::consts::PI).abs() < 1e-3, "{}", harness.state().ui.uv_rotation);
    harness.state_mut().ui.uv_rotation = 0.0;
    for by in [vec2(300.0, 200.0), vec2(-90.0, 0.0), vec2(40.0, 0.0)] {
        drag(&mut harness, PointerButton::Primary, Modifiers::ALT | Modifiers::SHIFT, start(view), by);
        let steps = harness.state().ui.uv_rotation / FRAC_PI_2;
        assert!((steps - steps.round()).abs() < 1e-3, "{by:?}: {} quarter turns", steps);
    }
}

#[test]
fn a_long_alt_shift_drag_reaches_a_half_turn() {
    let (mut harness, view) = app();
    drag(&mut harness, PointerButton::Primary, Modifiers::ALT | Modifiers::SHIFT, start(view), vec2(-400.0, 0.0));
    let steps = harness.state().ui.uv_rotation / FRAC_PI_2;
    assert!(steps.round() >= 1.0 && (steps - steps.round()).abs() < 1e-3, "turned by whole steps: {steps}");
}

#[test]
fn alt_right_drag_zooms_in_to_the_right_and_out_upwards_as_measured() {
    let (mut harness, view) = app();
    drag(&mut harness, PointerButton::Secondary, Modifiers::ALT, start(view), vec2(100.0, 0.0));
    let zoomed_in = harness.state().ui.uv_zoom;

    // The reference: 100 points right 760 -> 860 px
    assert!((zoomed_in - 860.0 / 760.0).abs() < 0.01, "right 100: {zoomed_in}");
    assert!(harness.state().properties_popup.is_none(), "no context menu");

    harness.state_mut().ui.uv_zoom = 1.0;
    drag(&mut harness, PointerButton::Secondary, Modifiers::ALT, start(view), vec2(0.0, -100.0));

    // The reference: 100 points up 760 -> 704 px
    let zoomed_out = harness.state().ui.uv_zoom;
    assert!((zoomed_out - 704.0 / 760.0).abs() < 0.01, "up 100: {zoomed_out}");
}

#[test]
fn alt_middle_and_ctrl_alt_left_pan_one_to_one_and_plain_middle_still_pans() {
    // A real Ctrl press sets both `ctrl` and `command`
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let cases = [(PointerButton::Middle, Modifiers::ALT), (PointerButton::Primary, Modifiers::ALT | ctrl), (PointerButton::Middle, Modifiers::NONE)];
    for (button, modifiers) in cases {
        let (mut harness, view) = app();
        let before = painted(&harness);
        let brush = harness.state().session.tool.brush.clone();
        drag(&mut harness, button, modifiers, start(view), vec2(100.0, -40.0));
        assert_eq!(harness.state().session.tool.brush, brush, "{button:?} {modifiers:?}: the brush is left alone");
        let ui = &harness.state().ui;
        let size = harness.state().viewport_rect.size();
        let moved = vec2(ui.uv_pan[0] * size.x * 0.5, -ui.uv_pan[1] * size.y * 0.5);
        assert!((moved - vec2(100.0, -40.0)).length() < 1.5, "{button:?} {modifiers:?}: moved {moved:?}");
        assert_eq!((ui.uv_rotation, ui.uv_zoom, painted(&harness)), (0.0, 1.0, before), "{button:?} {modifiers:?}: only a pan");
    }
}

#[test]
fn the_wheel_zooms_toward_the_cursor_and_stops_at_the_reference_limits() {
    let (mut harness, view) = app();
    let at = view.center() + vec2(-200.0, -100.0);
    harness.hover_at(at);
    harness.run_steps(2);
    harness.event(wheel(vec2(0.0, 100.0)));
    harness.run_steps(3);
    let ui = &harness.state().ui;
    assert!(ui.uv_zoom > 1.0 && ui.uv_pan[0] > 0.0 && ui.uv_pan[1] < 0.0, "zoomed about the cursor: {} {:?}", ui.uv_zoom, ui.uv_pan);

    for (start_zoom, scroll, limit) in [(200.0, 400.0, 220.0), (0.01, -400.0, 0.0026)] {
        harness.state_mut().ui.uv_zoom = start_zoom;
        for _ in 0..10 {
            harness.event(wheel(vec2(0.0, scroll)));
            harness.run_steps(3);
        }
        harness.run_steps(10);
        let zoom = harness.state().ui.uv_zoom;
        assert!((zoom - limit).abs() < limit * 0.01, "stops at {limit}: {zoom}");
    }
}

#[test]
fn f_resets_pan_zoom_and_rotation() {
    let (mut harness, _) = app();
    {
        let ui = &mut harness.state_mut().ui;
        (ui.uv_zoom, ui.uv_pan, ui.uv_rotation) = (7.0, [0.4, -0.3], 1.1);
    }
    harness.run_steps(2);
    press(&mut harness, Key::F);
    let ui = &harness.state().ui;
    assert_eq!((ui.uv_zoom, ui.uv_pan, ui.uv_rotation), (1.0, [0.0, 0.0], 0.0));
}

#[test]
fn f_frames_tile_1001_in_a_uv_tile_project() {
    let (mut harness, view) = tiled_app();
    {
        let ui = &mut harness.state_mut().ui;
        (ui.uv_zoom, ui.uv_pan, ui.uv_rotation) = (0.5, [0.4, -0.3], 1.1);
    }
    harness.run_steps(2);
    press(&mut harness, Key::F);
    let ui = &harness.state().ui;
    assert_eq!(ui.uv_rotation, 0.0);

    // Two tiles side by side: tile 1001 is the left half of the atlas, so the zoom doubles and the pan moves its centre to the view's centre
    assert!((ui.uv_zoom - 2.0).abs() < 1e-3, "zoom {}", ui.uv_zoom);
    let width = view.width();
    let atlas_center_offset = ui.uv_pan[0] * width * 0.5;
    assert!(atlas_center_offset > 0.0, "the atlas moves right so that its left tile is centred: {atlas_center_offset}");
    assert!(ui.uv_pan[1].abs() < 1e-4, "no vertical move: {:?}", ui.uv_pan);
}

#[test]
fn double_click_still_resets_the_view_and_the_rotation() {
    let (mut harness, view) = app();
    {
        let ui = &mut harness.state_mut().ui;
        (ui.uv_zoom, ui.uv_pan, ui.uv_rotation) = (3.0, [0.2, 0.1], 0.8);
    }
    harness.run_steps(2);
    let at = start(view);
    harness.hover_at(at);
    harness.run_steps(2);

    // Both clicks in one frame, inside egui's double-click time
    for _ in 0..2 {
        harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    }
    harness.run_steps(2);
    let ui = &harness.state().ui;
    assert_eq!((ui.uv_zoom, ui.uv_pan, ui.uv_rotation), (1.0, [0.0, 0.0], 0.0));
}

#[test]
fn painting_follows_the_turned_view() {
    let (mut harness, view) = app();
    harness.state_mut().ui.uv_rotation = FRAC_PI_2;
    harness.run_steps(2);
    let before = painted(&harness).1;
    drag(&mut harness, PointerButton::Primary, Modifiers::NONE, view.center(), vec2(30.0, 0.0));
    assert!(painted(&harness).1 > before, "a stroke on the turned texture still paints");
}

#[test]
fn ctrl_alt_left_pans_the_3d_camera_and_neither_orbits_it_nor_adjusts_the_brush() {
    let (mut harness, _) = app();
    harness.state_mut().ui.layout = ViewLayout::Only3D;
    harness.run_steps(4);
    let view = harness.state().viewport_rect;
    let (camera, brush) = (harness.state().session.camera, harness.state().session.tool.brush.clone());

    // A real Ctrl press sets both `ctrl` and `command`
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    drag(&mut harness, PointerButton::Primary, Modifiers::ALT | ctrl, start(view), vec2(80.0, 30.0));

    let after = harness.state().session.camera;
    assert!(after.target != camera.target, "the camera panned");
    assert_eq!((after.yaw, after.pitch, after.distance), (camera.yaw, camera.pitch, camera.distance), "no orbit, no zoom");
    assert_eq!(harness.state().session.tool.brush, brush, "the brush is left alone");
}

#[test]
fn f_in_baking_mode_resets_the_2d_view_that_baking_draws() {
    let (mut harness, _) = app();
    {
        let ui = &mut harness.state_mut().ui;
        (ui.uv_zoom, ui.uv_pan, ui.uv_rotation) = (7.0, [0.4, -0.3], 1.1);
        ui.mode = texturecraft_ui_egui::state::Mode::Baking;
    }
    harness.run_steps(3);
    press(&mut harness, Key::F);
    harness.state_mut().ui.mode = texturecraft_ui_egui::state::Mode::Painting;
    harness.run_steps(4);
    let ui = &harness.state().ui;
    assert_eq!((ui.uv_zoom, ui.uv_pan, ui.uv_rotation), (1.0, [0.0, 0.0], 0.0));
}
