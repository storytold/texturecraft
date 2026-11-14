//! Pen pressure (touch force) thins strokes in the 2D view as it does in the 3D view.

use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase, pos2};
use egui_kittest::Harness;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

/// One frame of input: the windowing layer sends a pen sample as a touch (`pen`: its phase and
/// force) plus the pointer events it drives, in the same frame. A mouse sends no touch.
fn frame(h: &mut Harness<'_, TextureCraftApp>, pen: Option<(TouchPhase, Option<f32>)>, pos: Pos2, button: Option<bool>) {
    let events = &mut h.input_mut().events;
    if let Some((phase, force)) = pen {
        events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase, pos, force });
    }
    events.push(Event::PointerMoved(pos));
    if let Some(pressed) = button {
        events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
}

/// Paint in the 2D view with `input` and return the summed base color alpha it added.
fn paint_2d(input: impl FnOnce(&mut Harness<'_, TextureCraftApp>)) -> f64 {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.layout = ViewLayout::Only2D;
    h.state_mut().session.tool.brush.min_size = 0.0;
    h.run_steps(4);
    let before = painted(&h);
    input(&mut h);
    h.run_steps(3);
    painted(&h) - before
}

const FROM: Pos2 = pos2(600.0, 500.0);

/// One stroke across the view, with pen `force` on every sample (None: a mouse).
fn stroke(force: Option<f32>) -> f64 {
    let pen = |phase| force.map(|f| (phase, Some(f)));
    let to = pos2(800.0, 500.0);
    paint_2d(|h| {
        frame(h, pen(TouchPhase::Start), FROM, Some(true));
        for k in 1..=8 {
            frame(h, pen(TouchPhase::Move), FROM + (to - FROM) * (k as f32 / 8.0), None);
        }
        frame(h, pen(TouchPhase::End), to, Some(false));
    })
}

/// A tap; a pen lifts reporting no pressure (as Windows Ink does), and the dab paints then.
fn tap(force: Option<f32>) -> f64 {
    paint_2d(|h| {
        frame(h, force.map(|f| (TouchPhase::Start, Some(f))), FROM, Some(true));
        frame(h, force.map(|_| (TouchPhase::End, None)), FROM, Some(false));
    })
}

/// The selected layer's summed base color alpha.
fn painted(h: &Harness<'_, TextureCraftApp>) -> f64 {
    let s = &h.state().session;
    let id = s.selected_layer().expect("a selected layer");
    let Some(img) = s.doc.pixels.layer(id, Channel::BaseColor) else { return 0.0 };
    let (w, hgt) = s.doc.project.texture_sets[s.ts()].size();
    (0..hgt).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| img.get(x, y)[3] as f64 / 65535.0).sum()
}

#[test]
fn pen_pressure_thins_strokes_in_the_2d_view() {
    let mouse = stroke(None);
    let light = stroke(Some(0.2));
    assert!(mouse > 10.0, "the mouse stroke painted: {mouse}");
    assert!(light > 0.0 && light < mouse * 0.5, "a light pen paints a thinner stroke: {light} vs {mouse}");
}

#[test]
fn a_light_pen_tap_paints_a_small_dab() {
    let mouse = tap(None);
    let light = tap(Some(0.2));
    assert!(mouse > 1.0, "the mouse click painted: {mouse}");
    assert!(light > 0.0 && light < mouse * 0.25, "the tap keeps its contact pressure: {light} vs {mouse}");
}

#[test]
fn frames_omitting_touch_events_during_stroke_do_not_spike_pressure() {
    let to = pos2(800.0, 500.0);
    let intermittent = paint_2d(|h| {
        frame(h, Some((TouchPhase::Start, Some(0.2))), FROM, Some(true));
        for k in 1..=8 {
            let pen = if k % 2 == 0 { Some((TouchPhase::Move, Some(0.2))) } else { None };
            frame(h, pen, FROM + (to - FROM) * (k as f32 / 8.0), None);
        }
        frame(h, Some((TouchPhase::End, Some(0.2))), to, Some(false));
    });
    let mouse = stroke(None);
    assert!(intermittent > 0.0 && intermittent < mouse * 0.5, "intermittent frames without touch keep light pressure: {intermittent} vs {mouse}");
}

#[test]
fn mouse_stroke_immediately_after_pen_release_uses_full_pressure() {
    let to = pos2(800.0, 500.0);
    let baseline = stroke(None);
    let immediate_mouse = paint_2d(|h| {
        let tap_pos = pos2(600.0, 680.0);
        frame(h, Some((TouchPhase::Start, Some(0.2))), tap_pos, Some(true));
        frame(h, Some((TouchPhase::End, None)), tap_pos, Some(false));
        frame(h, None, FROM, Some(true));
        for k in 1..=8 {
            frame(h, None, FROM + (to - FROM) * (k as f32 / 8.0), None);
        }
        frame(h, None, to, Some(false));
    });
    assert!(immediate_mouse > baseline * 0.9, "mouse stroke after pen release keeps full pressure: {immediate_mouse} vs {baseline}");
}
