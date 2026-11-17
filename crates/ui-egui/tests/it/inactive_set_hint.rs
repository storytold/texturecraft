//! A brush stroke that starts on a texture set that is not the active one paints nothing and says why, once per stroke

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use texturecraft_engine::Tool;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_ui_egui::TextureCraftApp;

/// Distance between the view points searched for a set
const SEARCH_STEP: f32 = 8.0;

/// The sample project (the ball is set 0, the base set 1) with the Ball active and the paint tool, at 1600 x 960
fn sample_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    assert_eq!(harness.state().session.ts(), 0);
    assert_eq!(harness.state().session.tool.tool, Tool::Paint);
    harness
}

/// How far around a point `set` must still show, so a drag that starts there stays on the set
const MARGIN: f32 = 32.0;

/// The first point of the 3D view with texture set `set` all around it
fn point_over(harness: &Harness<'_, TextureCraftApp>, set: usize) -> Pos2 {
    let app = harness.state();
    let rect = app.viewport_rect;
    let steps = |length: f32| (1..(length / SEARCH_STEP) as usize).map(|step| step as f32 * SEARCH_STEP);

    steps(rect.height())
        .flat_map(|y| steps(rect.width()).map(move |x| (x, y)))
        .find(|&(x, y)| {
            [(0.0, 0.0), (MARGIN, 0.0), (-MARGIN, 0.0), (0.0, MARGIN), (0.0, -MARGIN)]
                .iter()
                .all(|(dx, dy)| app.session.texture_set_at(Vec2::new(x + dx, y + dy), app.session.viewport) == Some(set))
        })
        .map(|(x, y)| rect.min + vec2(x, y))
        .unwrap()
}

/// Press at `from`, move by `by` in a few steps and release
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, by: egui::Vec2) {
    harness.hover_at(from);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    for step in 1..=6 {
        harness.hover_at(from + by * (step as f32 / 6.0));
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: from + by, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);
}

/// The text of the toast that says the sample project's second set is not active
fn hint_text(harness: &Harness<'_, TextureCraftApp>) -> String {
    let name = &harness.state().session.doc.project.texture_sets[1].name;
    format!("Texture set '{name}' is not active for painting. Select it in the Texture Sets list.")
}

/// How many toasts say that the sample project's second set is not active
fn hint_count(harness: &Harness<'_, TextureCraftApp>) -> usize {
    let hint = hint_text(harness);
    harness.state().toasts.iter().filter(|toast| toast.text == hint).count()
}

#[test]
fn a_stroke_starting_on_another_set_shows_the_hint_once_and_paints_nothing() {
    let mut harness = sample_app();
    let history = harness.state().session.history_labels().len();

    let base = point_over(&harness, 1);
    drag(&mut harness, base, vec2(24.0, 0.0));

    assert_eq!(hint_count(&harness), 1, "one toast for the whole drag, not one per frame");
    assert_eq!(harness.state().session.history_labels().len(), history, "nothing was painted");
    assert_eq!(harness.state().session.ts(), 0, "the stroke does not switch the active set");

    // A second stroke on that set says it again (the first toast may have expired by then)
    let hint = hint_text(&harness);
    let first_until = harness.state().toasts.iter().filter(|toast| toast.text == hint).map(|toast| toast.until).fold(f64::MIN, f64::max);
    drag(&mut harness, base + vec2(0.0, 12.0), vec2(24.0, 0.0));
    assert!(
        harness.state().toasts.iter().any(|toast| toast.text == hint && toast.until > first_until),
        "the second stroke shows a new inactive-set hint, not just any toast"
    );
}

#[test]
fn a_stroke_on_the_active_set_shows_no_hint_and_paints() {
    let mut harness = sample_app();
    let history = harness.state().session.history_labels().len();

    let ball = point_over(&harness, 0);
    drag(&mut harness, ball, vec2(24.0, 0.0));

    assert!(harness.state().toasts.iter().all(|toast| !toast.text.contains("is not active for painting")));
    assert_eq!(harness.state().session.history_labels().len(), history + 1, "the stroke painted");
}

#[test]
fn the_color_picker_shows_no_hint_on_another_set() {
    let mut harness = sample_app();
    harness.state_mut().session.tool.tool = Tool::ColorPicker;
    harness.run_steps(2);

    let base = point_over(&harness, 1);
    drag(&mut harness, base, vec2(24.0, 0.0));

    assert_eq!(hint_count(&harness), 0, "the color picker reads any set, so it needs no hint");
}
