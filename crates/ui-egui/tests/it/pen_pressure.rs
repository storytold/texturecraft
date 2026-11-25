//! Pen pressure (touch force) thins strokes in the 2D view as it does in the 3D view.

use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase, pos2};
use egui_kittest::Harness;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

/// One frame of input: the windowing layer sends a pen sample as a touch (`pen`: its phase and
/// force) plus the pointer events it drives, in the same frame. A mouse sends no touch.
fn frame(harness: &mut Harness<'_, TextureCraftApp>, pen: Option<(TouchPhase, Option<f32>)>, position: Pos2, button: Option<bool>) {
    let events = &mut harness.input_mut().events;
    if let Some((phase, force)) = pen {
        events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase, pos: position, force });
    }
    events.push(Event::PointerMoved(position));
    if let Some(pressed) = button {
        events.push(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }

    harness.step();
}

/// Paint in the 2D view with `input` and return the summed base color alpha it added.
fn paint_2d(input: impl FnOnce(&mut Harness<'_, TextureCraftApp>)) -> f64 {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.state_mut().session.tool.brush.min_size = 0.0;
    harness.run_steps(4);
    let before = painted(&harness);

    input(&mut harness);
    harness.run_steps(3);

    painted(&harness) - before
}

const STROKE_START: Pos2 = pos2(600.0, 500.0);

/// One stroke across the view, with pen `force` on every sample (None: a mouse).
fn stroke(force: Option<f32>) -> f64 {
    let pen = |phase| force.map(|pressure| (phase, Some(pressure)));
    let stroke_end = pos2(800.0, 500.0);

    paint_2d(|harness| {
        frame(harness, pen(TouchPhase::Start), STROKE_START, Some(true));
        for sample_index in 1..=8 {
            frame(harness, pen(TouchPhase::Move), STROKE_START + (stroke_end - STROKE_START) * (sample_index as f32 / 8.0), None);
        }
        frame(harness, pen(TouchPhase::End), stroke_end, Some(false));
    })
}

/// A tap; a pen lifts reporting no pressure (as Windows Ink does), and the dab paints then.
fn tap(force: Option<f32>) -> f64 {
    paint_2d(|harness| {
        frame(harness, force.map(|pressure| (TouchPhase::Start, Some(pressure))), STROKE_START, Some(true));
        frame(harness, force.map(|_| (TouchPhase::End, None)), STROKE_START, Some(false));
    })
}

/// The selected layer's summed base color alpha.
fn painted(harness: &Harness<'_, TextureCraftApp>) -> f64 {
    let session = &harness.state().session;
    let layer_id = session.selected_layer().expect("a selected layer");
    let Some(image) = session.doc.pixels.layer(layer_id, Channel::BaseColor) else { return 0.0 };

    let (width, height) = session.doc.project.texture_sets[session.ts()].size();
    (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| image.get(x, y)[3] as f64 / 65535.0).sum()
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
    let stroke_end = pos2(800.0, 500.0);
    let intermittent = paint_2d(|harness| {
        frame(harness, Some((TouchPhase::Start, Some(0.2))), STROKE_START, Some(true));
        for sample_index in 1..=8 {
            let touch = if sample_index % 2 == 0 { Some((TouchPhase::Move, Some(0.2))) } else { None };
            frame(harness, touch, STROKE_START + (stroke_end - STROKE_START) * (sample_index as f32 / 8.0), None);
        }
        frame(harness, Some((TouchPhase::End, Some(0.2))), stroke_end, Some(false));
    });
    let mouse = stroke(None);

    assert!(intermittent > 0.0 && intermittent < mouse * 0.5, "intermittent frames without touch keep light pressure: {intermittent} vs {mouse}");
}

#[test]
fn mouse_stroke_immediately_after_pen_release_uses_full_pressure() {
    let stroke_end = pos2(800.0, 500.0);
    let baseline = stroke(None);
    let immediate_mouse = paint_2d(|harness| {
        let tap_position = pos2(600.0, 680.0);
        frame(harness, Some((TouchPhase::Start, Some(0.2))), tap_position, Some(true));
        frame(harness, Some((TouchPhase::End, None)), tap_position, Some(false));

        frame(harness, None, STROKE_START, Some(true));
        for sample_index in 1..=8 {
            frame(harness, None, STROKE_START + (stroke_end - STROKE_START) * (sample_index as f32 / 8.0), None);
        }
        frame(harness, None, stroke_end, Some(false));
    });

    assert!(immediate_mouse > baseline * 0.9, "mouse stroke after pen release keeps full pressure: {immediate_mouse} vs {baseline}");
}

#[test]
fn secondary_pressed_during_active_primary_pen_stroke_retains_light_pressure() {
    let context = egui::Context::default();
    let mut start_input = egui::RawInput::default();
    start_input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Start, pos: pos2(100.0, 100.0), force: Some(0.2) });
    start_input.events.push(Event::PointerButton { pos: pos2(100.0, 100.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });

    context
        .run_ui(start_input, |ui| {
            let pressure = texturecraft_ui_egui::viewport::pen_pressure(ui);
            assert!((pressure - 0.2).abs() < 1e-4, "initial touch records 0.2: {pressure}");
        })
        .drop_without_applying_deltas();

    let mut secondary_input = egui::RawInput::default();
    secondary_input.events.push(Event::PointerButton { pos: pos2(110.0, 100.0), button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });

    context
        .run_ui(secondary_input, |ui| {
            let pressure = texturecraft_ui_egui::viewport::pen_pressure(ui);
            assert!((pressure - 0.2).abs() < 1e-4, "secondary press preserves contact pressure: {pressure}");
        })
        .drop_without_applying_deltas();
}

#[test]
fn touch_start_and_unforced_end_in_one_frame_keep_contact_pressure() {
    let context = egui::Context::default();
    let position = pos2(100.0, 100.0);
    let mut input = egui::RawInput::default();
    input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Start, pos: position, force: Some(0.2) });
    input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::End, pos: position, force: None });

    context
        .run_ui(input, |ui| {
            let pressure = texturecraft_ui_egui::viewport::pen_pressure(ui);
            assert!((pressure - 0.2).abs() < 1e-4, "the tap keeps its contact pressure: {pressure}");
        })
        .drop_without_applying_deltas();
}

#[test]
fn unforced_move_on_press_frame_keeps_same_contact_pressure() {
    let context = egui::Context::default();
    let position = pos2(100.0, 100.0);
    let mut press_input = egui::RawInput::default();
    press_input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Start, pos: position, force: Some(0.2) });
    press_input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Move, pos: position, force: None });
    press_input.events.push(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });

    context
        .run_ui(press_input, |ui| {
            let pressure = texturecraft_ui_egui::viewport::pen_pressure(ui);
            assert!((pressure - 0.2).abs() < 1e-4, "press frame keeps the same contact's pressure: {pressure}");
        })
        .drop_without_applying_deltas();

    context
        .run_ui(egui::RawInput::default(), |ui| {
            let pressure = texturecraft_ui_egui::viewport::pen_pressure(ui);
            assert!((pressure - 0.2).abs() < 1e-4, "touch-free frame keeps the recovered pressure: {pressure}");
        })
        .drop_without_applying_deltas();
}

#[test]
fn unforced_move_does_not_recover_a_completed_contact() {
    let context = egui::Context::default();
    let position = pos2(100.0, 100.0);
    let mut input = egui::RawInput::default();
    input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::End, pos: position, force: Some(0.2) });
    input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Move, pos: position, force: None });
    input.events.push(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });

    context
        .run_ui(input, |ui| {
            let pressure = texturecraft_ui_egui::viewport::pen_pressure(ui);
            assert!((pressure - 1.0).abs() < 1e-4, "a completed contact cannot supply pressure to the later move: {pressure}");
        })
        .drop_without_applying_deltas();
}

#[test]
fn unforced_touch_start_and_end_in_one_frame_clear_old_pen_pressure() {
    let context = egui::Context::default();
    let position = pos2(100.0, 100.0);
    let mut pen_input = egui::RawInput::default();
    pen_input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Start, pos: position, force: Some(0.2) });
    pen_input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::End, pos: position, force: None });

    context
        .run_ui(pen_input, |ui| {
            assert!((texturecraft_ui_egui::viewport::pen_pressure(ui) - 0.2).abs() < 1e-4);
        })
        .drop_without_applying_deltas();

    let mut finger_input = egui::RawInput::default();
    finger_input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Start, pos: position, force: None });
    finger_input.events.push(Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::End, pos: position, force: None });

    context
        .run_ui(finger_input, |ui| {
            let pressure = texturecraft_ui_egui::viewport::pen_pressure(ui);
            assert!((pressure - 1.0).abs() < 1e-4, "a pressure-free tap clears the old pen pressure: {pressure}");
        })
        .drop_without_applying_deltas();
}
