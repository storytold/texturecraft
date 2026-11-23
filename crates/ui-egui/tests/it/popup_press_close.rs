//! A press outside a popup closes it when it turns into a drag (a paint stroke on the model), as a click does: the tool's colour, the layer menus and the symmetry settings, in the default layout and in a rearranged one restored after a restart

use eframe::App as _;
use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Dock;

use crate::{BAKE_RESOLUTION, DRAG_STEP_FRAMES, PROPERTIES_WINDOW, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

use crate::memory_storage::MemoryStorage;

/// Moves of a drag on the model, each a few points, so it is a drag well before its release
const DRAG_MOVES: usize = 6;

/// A short stroke on the model
const STROKE: Vec2 = vec2(-60.0, 40.0);

/// From the tool's Base color label to inside its swatch
const LABEL_TO_SWATCH: f32 = 60.0;

/// The app started on `storage` (its saved arrangement, if any) with the paint tool's Properties showing
fn app(storage: &MemoryStorage) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();

    // The view has no accessible name of its own; these tests drive it by position
    let builder = Harness::builder().with_size(PROPERTIES_WINDOW).with_accessibility_check(false);
    let mut harness = builder.build_eframe(|context| {
        let mut creation = eframe::CreationContext::_new_kittest(context.egui_ctx.clone());
        creation.storage = Some(storage);
        TextureCraftApp::new(&creation, session)
    });

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// Properties swapped with Assets (now left of the view), saved and the app started again from that save
fn restarted_rearranged() -> Harness<'static, TextureCraftApp> {
    let mut storage = MemoryStorage::default();
    let mut first = app(&storage);
    let saved = serde_json::to_string(&first.state().ui.dock).unwrap();
    let swapped = saved.replace("\"Assets\"", "\"Swap\"").replace("\"Properties\"", "\"Assets\"").replace("\"Swap\"", "\"Properties\"");
    first.state_mut().ui.dock = Dock::from_value(serde_json::from_str(&swapped).unwrap());
    first.run_steps(STARTUP_FRAMES);
    first.state_mut().save(&mut storage);
    let expected_layout = first.state().ui.dock.clone();
    drop(first);

    let restarted = app(&storage);
    assert_eq!(restarted.state().ui.dock, expected_layout, "the rearranged layout comes back");
    restarted
}

fn press(harness: &mut Harness<'_, TextureCraftApp>, at: Pos2, pressed: bool, modifiers: Modifiers) {
    harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers });
    harness.run_steps(DRAG_STEP_FRAMES);
}

/// A primary press at `from` dragged by `by` in small moves, then released
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, by: Vec2, modifiers: Modifiers) {
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
    harness.run_steps(1);
    press(harness, from, true, modifiers);
    for step in 1..=DRAG_MOVES {
        harness.hover_at(from + by * (step as f32 / DRAG_MOVES as f32));
        harness.run_steps(1);
    }
    press(harness, from + by, false, modifiers);
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(SETTLE_FRAMES);
}

/// A plain press and release at `at`
fn click(harness: &mut Harness<'_, TextureCraftApp>, at: Pos2) {
    harness.hover_at(at);
    harness.run_steps(1);
    press(harness, at, true, Modifiers::NONE);
    press(harness, at, false, Modifiers::NONE);
    harness.run_steps(SETTLE_FRAMES);
}

/// A stroke on the model, clear of the panels and the popups that open beside them
fn stroke_on_model(harness: &mut Harness<'_, TextureCraftApp>, modifiers: Modifiers) {
    let view = harness.state().viewport_rect;
    drag(harness, view.center(), STROKE, modifiers);
}

/// Opens the paint tool's Base color swatch from its Properties' Material tab
fn open_tool_color(harness: &mut Harness<'_, TextureCraftApp>) {
    let header = harness.get_by_label("Properties - Paint").rect();
    let in_properties = |rect: egui::Rect| header.x_range().contains(rect.center().x) && rect.top() > header.top();
    let material = harness.get_all_by_label("Material").find(|node| in_properties(node.rect())).unwrap();
    material.click();
    harness.run_steps(SETTLE_FRAMES);

    let label = harness.get_all_by_value("Base color").find(|node| in_properties(node.rect())).unwrap().rect();
    click(harness, label.right_center() + vec2(LABEL_TO_SWATCH, 0.0));
    assert!(egui::Popup::is_any_open(&harness.ctx), "the swatch opens its colour popup");
}

#[test]
fn a_stroke_on_the_model_closes_the_tool_colour_popup_in_the_default_and_a_restored_rearranged_layout() {
    let check_stroke = |mut harness: Harness<'_, TextureCraftApp>, layout: &str| {
        open_tool_color(&mut harness);
        let history = harness.state().session.history_labels().len();

        stroke_on_model(&mut harness, Modifiers::NONE);

        assert!(!egui::Popup::is_any_open(&harness.ctx), "{layout}: the stroke closes the popup");
        assert!(harness.state().session.history_labels().len() > history, "{layout}: the stroke still paints");
    };

    check_stroke(app(&MemoryStorage::default()), "default");
    check_stroke(restarted_rearranged(), "restored rearranged");
}

#[test]
fn an_alt_orbit_keeps_the_tool_colour_popup_open() {
    let mut harness = app(&MemoryStorage::default());
    open_tool_color(&mut harness);

    stroke_on_model(&mut harness, Modifiers::ALT);

    assert!(egui::Popup::is_any_open(&harness.ctx), "an Alt+drag orbits the camera and keeps the popup, like the colour picker");
}

#[test]
fn a_stroke_on_the_model_closes_the_add_effect_menu() {
    let mut harness = app(&MemoryStorage::default());
    harness.get_by_role_and_label(Role::Button, "Add effect").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(egui::Popup::is_any_open(&harness.ctx), "the menu opens");

    stroke_on_model(&mut harness, Modifiers::NONE);

    assert!(!egui::Popup::is_any_open(&harness.ctx), "the stroke closes the menu");
}

#[test]
fn outside_drag_dismissal_handles_motion_batched_with_release() {
    for (modifiers, stays_open) in [(Modifiers::ALT, true), (Modifiers::NONE, false)] {
        let mut harness = app(&MemoryStorage::default());
        open_tool_color(&mut harness);
        let origin = harness.state().viewport_rect.center();
        harness.event(Event::ModifiersChanged(modifiers));
        harness.hover_at(origin);
        press(&mut harness, origin, true, modifiers);
        assert!(egui::Popup::is_any_open(&harness.ctx), "the press is still a potential click");

        let end = origin + STROKE;
        harness
            .input_mut()
            .events
            .extend([Event::PointerMoved(end), Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers }]);
        harness.run_steps(1);

        assert!(harness.ctx.input(|input| input.pointer.button_released(PointerButton::Primary) && !input.pointer.any_click()));
        assert_eq!(egui::Popup::is_any_open(&harness.ctx), stays_open);
    }
}

#[test]
fn a_stroke_on_the_model_closes_the_symmetry_settings() {
    let mut harness = app(&MemoryStorage::default());

    // The paint controls overflow the bar at this width; its arrow brings the Symmetry buttons into view
    harness.get_by_label("Scroll right").click();
    harness.run_steps(SETTLE_FRAMES);
    harness.get_by_label("Symmetry settings").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_label("Radial symmetry").is_some(), "the settings open");

    stroke_on_model(&mut harness, Modifiers::NONE);

    assert!(harness.query_by_label("Radial symmetry").is_none(), "the stroke closes the settings");
}
