//! Helpers the filter panel tests share

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, Filter, LayerId};
use texturecraft_engine::{StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

/// The test window's width: the dock at its usual size beside the views
pub(crate) const WINDOW_WIDTH: f32 = 1600.0;

/// A window tall enough for a filter's rows and its Attributes in the docked Properties panel
pub(crate) const WINDOW_HEIGHT: f32 = 1400.0;

/// A window tall enough for the filters with many cards (Baked Lighting Stylized, Color Match) to show every row unscrolled
pub(crate) const TALL_WINDOW_HEIGHT: f32 = 2400.0;

/// The app `height` tall with library filter `filter` on a fill layer's `target` ("content" or "mask", a white mask added), selected so Properties shows it
pub(crate) fn app_with(filter: &str, target: &str, height: f32) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    if target == "mask" {
        execute(&mut session, "mask.add.white", &json!({"layer": layer["layer"]})).unwrap();
    }
    let effect = execute(&mut session, "effect.add.filter", &json!({"target": target, "filter": filter})).unwrap();
    let effect = LayerId(effect["effect"].as_u64().unwrap());
    execute(&mut session, "layer.select", &json!({"layer": layer["layer"]})).unwrap();
    session.target = StackTarget::Effect(effect);

    let mut harness = Harness::builder().with_size(egui::vec2(WINDOW_WIDTH, height)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    (harness, effect)
}

/// The filter effect `effect` holds now
pub(crate) fn filter_of(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> Filter {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Filter(filter) => filter.clone(),
        other => panic!("not a filter: {other:?}"),
    }
}

/// A real pointer click at `pos`: move there, press, one frame, release, then frames for the edit to show
pub(crate) fn click_at(harness: &mut Harness<'_, TextureCraftApp>, pos: egui::Pos2) {
    let modifiers = egui::Modifiers::NONE;
    harness.event(egui::Event::PointerMoved(pos));
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(3);
}

/// Pick `option` of a setting shown as a boxed switch where its words fit, or as a dropdown showing `current` where they don't
pub(crate) fn pick(harness: &mut Harness<'_, TextureCraftApp>, current: &str, option: &str) {
    if let Some(segment) = harness.query_by_role_and_label(Role::RadioButton, option).map(|node| node.rect()) {
        click_at(harness, segment.center());
    } else {
        let list = harness.get_by_role_and_label(Role::ComboBox, current).rect();
        click_at(harness, list.center());
        let choice = harness.get_by_label(option).rect();
        click_at(harness, choice.center());
    }
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

/// The rect of the topmost of the widgets with `role` and `label` (a name a later row repeats)
pub(crate) fn topmost(harness: &Harness<'_, TextureCraftApp>, role: Role, label: &str) -> egui::Rect {
    harness.get_all_by_role_and_label(role, label).map(|node| node.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap()
}

/// The rect of the lowest of the widgets with `role` and `label`
pub(crate) fn lowest(harness: &Harness<'_, TextureCraftApp>, role: Role, label: &str) -> egui::Rect {
    harness.get_all_by_role_and_label(role, label).map(|node| node.rect()).max_by(|a, b| a.top().total_cmp(&b.top())).unwrap()
}

/// Moves a drag makes on its way, one frame each, so a press makes several changes
const DRAG_STEPS: usize = 4;

/// The undo steps the session holds now
pub(crate) fn undo_depth(harness: &Harness<'_, TextureCraftApp>) -> usize {
    harness.state().session.history_labels().len()
}

/// Undoes the last step, then frames for the panel to show it
pub(crate) fn undo(harness: &mut Harness<'_, TextureCraftApp>) {
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(2);
}
