//! Helpers the filter panel tests share

use egui_kittest::Harness;
use texturecraft_ui_egui::TextureCraftApp;

/// A real pointer click at `pos`: move there, press, one frame, release, then frames for the edit to show
pub(crate) fn click_at(harness: &mut Harness<'_, TextureCraftApp>, pos: egui::Pos2) {
    let modifiers = egui::Modifiers::NONE;
    harness.event(egui::Event::PointerMoved(pos));
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(3);
}

/// A press at `from`, a drag to `to` in steps and a release, as the pointer does it
pub(crate) fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: egui::Pos2, to: egui::Pos2) {
    let steps: Vec<egui::Pos2> = (1..=DRAG_STEPS).map(|step| from + (to - from) * (step as f32 / DRAG_STEPS as f32)).collect();
    drag_through(harness, from, &steps);
}

/// A press at `from`, a move to each of `steps` (one frame each) and a release at the last, as the pointer does it
pub(crate) fn drag_through(harness: &mut Harness<'_, TextureCraftApp>, from: egui::Pos2, steps: &[egui::Pos2]) {
    let modifiers = egui::Modifiers::NONE;
    harness.event(egui::Event::PointerMoved(from));
    harness.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    for step in steps {
        harness.event(egui::Event::PointerMoved(*step));
        harness.run_steps(1);
    }
    let to = steps.last().copied().unwrap_or(from);
    harness.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(2);
}

/// How far a slider's track runs inside its rect at each end (the knob's radius)
const SLIDER_TRACK_INSET: f32 = 5.0;

/// The point at `share` (0..1) along slider `track`'s track: 0 is the range's start, 1 its end
pub(crate) fn along(track: egui::Rect, share: f32) -> egui::Pos2 {
    egui::pos2(track.left() + SLIDER_TRACK_INSET + (track.width() - 2.0 * SLIDER_TRACK_INSET) * share, track.center().y)
}

/// Moves a drag makes on its way, one frame each, so a press makes several changes
const DRAG_STEPS: usize = 4;

/// The undo steps the session holds now
pub(crate) fn undo_depth(harness: &Harness<'_, TextureCraftApp>) -> usize {
    harness.state().session.history_labels().len()
}
