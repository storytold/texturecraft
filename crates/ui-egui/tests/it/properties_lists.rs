//! Every list in each Properties screen, through real pointer input: a click opens it; with it open the view still zooms on the wheel, pans on a middle drag and orbits on an Alt drag; Escape, a click outside and a stroke on the model close it

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::{Session, StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

use crate::fill_support::{click_node, fill_app};
use crate::filter_support::click_at;
use crate::{BAKE_RESOLUTION, DRAG_STEP_FRAMES, PROPERTIES_WINDOW, REDRAW_FRAMES, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION, WHEEL_NOTCH};

/// Moves of a drag in the view, each a few points, so it is a drag well before its release
const DRAG_MOVES: usize = 6;

/// A drag in the view: long enough to move the camera, short enough to stay in the view
const GESTURE: Vec2 = vec2(-60.0, 40.0);

/// Where in the view the gestures start, as a share of its width from its left edge
const GESTURE_SHARE: f32 = 0.3;

/// Escapes between lists, so one that stays open can't leave its popup over the next
const ESCAPES_TO_CLEAR: usize = 2;

/// The Properties screen a `setup` leaves selected in the sample session
fn screen(setup: impl FnOnce(&mut Session)) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    setup(&mut session);
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).with_accessibility_check(false).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// A new fill layer with a white mask, `command` adding an effect with `params` to its `target` stack, and that effect selected
fn with_effect(session: &mut Session, target: &str, command: &str, params: Value) {
    let layer = execute(session, "layer.add.fill", &json!({})).unwrap()["layer"].as_u64().unwrap();
    execute(session, "mask.add.white", &json!({"layer": layer})).unwrap();
    let mut params = params;
    params["layer"] = json!(layer);
    params["target"] = json!(target);
    let effect = execute(session, command, &params).unwrap()["effect"].as_u64().unwrap();
    execute(session, "layer.select", &json!({"layer": layer})).unwrap();
    session.target = StackTarget::Effect(LayerId(effect));
}

/// The screens with lists in Properties, each named for the failure messages
fn screens() -> Vec<(&'static str, Harness<'static, TextureCraftApp>)> {
    vec![
        ("paint tool", screen(|_| {})),
        (
            "fill layer",
            screen(|session| {
                execute(session, "layer.add.fill", &json!({})).unwrap();
            }),
        ),
        (
            "folder",
            screen(|session| {
                let folder = execute(session, "layer.add.group", &json!({})).unwrap()["layer"].clone();
                execute(session, "layer.select", &json!({"layer": folder, "target": "content"})).unwrap();
            }),
        ),
        ("mask fill", screen(|session| with_effect(session, "mask", "effect.add.fill", json!({})))),
        ("mask paint", screen(|session| with_effect(session, "mask", "effect.add.paint", json!({})))),
        ("colour selection", screen(|session| with_effect(session, "mask", "effect.add.colorSelection", json!({})))),
        ("compare mask", screen(|session| with_effect(session, "mask", "effect.add.compare", json!({})))),
        ("levels", screen(|session| with_effect(session, "content", "effect.add.levels", json!({})))),
        ("generator", screen(|session| with_effect(session, "mask", "effect.add.generator", json!({"generator": "curvature"})))),
        ("blur filter", screen(|session| with_effect(session, "content", "effect.add.filter", json!({"filter": "blur"})))),
        ("gradient filter", screen(|session| with_effect(session, "content", "effect.add.filter", json!({"filter": "gradient"})))),
        ("warp filter", screen(|session| with_effect(session, "mask", "effect.add.filter", json!({"filter": "warp"})))),
    ]
}

/// The lists right of the view (the docked panels), top to bottom, each as its rect and what it shows
fn lists(harness: &Harness<'_, TextureCraftApp>) -> Vec<(Rect, String)> {
    let view = harness.state().viewport_rect;
    let window = Rect::from_min_size(Pos2::ZERO, PROPERTIES_WINDOW);
    let mut lists: Vec<(Rect, String)> = harness
        .query_all_by_role(Role::ComboBox)
        .filter(|node| node.rect().left() >= view.right() && window.contains_rect(node.rect()))
        .map(|node| (node.rect(), format!("{:?} = {:?}", node.accesskit_node().label(), node.value())))
        .collect();
    lists.sort_by(|(a, _), (b, _)| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
    lists
}

/// A `button` press at `from` dragged by `by` in small moves, then released, with `modifiers` held
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, by: Vec2, button: PointerButton, modifiers: Modifiers) {
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button, pressed: true, modifiers });
    harness.run_steps(DRAG_STEP_FRAMES);
    for step in 1..=DRAG_MOVES {
        harness.hover_at(from + by * (step as f32 / DRAG_MOVES as f32));
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: from + by, button, pressed: false, modifiers });
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(SETTLE_FRAMES);
}

/// One wheel notch at `at`
fn wheel(harness: &mut Harness<'_, TextureCraftApp>, at: Pos2) {
    harness.hover_at(at);
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, WHEEL_NOTCH),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(REDRAW_FRAMES);
}

/// Whether one wheel notch at `at` zooms the camera
fn zooms(harness: &mut Harness<'_, TextureCraftApp>, at: Pos2) -> bool {
    let distance = harness.state().session.camera.distance;
    wheel(harness, at);
    harness.state().session.camera.distance != distance
}

/// Whether a middle drag from `from` pans the camera
fn pans(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2) -> bool {
    let target = harness.state().session.camera.target;
    drag(harness, from, GESTURE, PointerButton::Middle, Modifiers::NONE);
    harness.state().session.camera.target != target
}

/// Whether an Alt drag from `from` orbits the camera
fn orbits(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2) -> bool {
    let yaw = harness.state().session.camera.yaw;
    drag(harness, from, GESTURE, PointerButton::Primary, Modifiers::ALT);
    harness.state().session.camera.yaw != yaw
}

/// Where the gestures start in the view: clear of the lists, which open beside Properties
fn gesture_start(harness: &Harness<'_, TextureCraftApp>) -> Pos2 {
    let view = harness.state().viewport_rect;
    view.left_center() + vec2(view.width() * GESTURE_SHARE, 0.0)
}

fn is_open(harness: &Harness<'_, TextureCraftApp>) -> bool {
    egui::Popup::is_any_open(&harness.ctx)
}

/// Opens the list at `button` with a click; false when it did not open
fn open(harness: &mut Harness<'_, TextureCraftApp>, button: Rect) -> bool {
    click_at(harness, button.center());
    harness.run_steps(SETTLE_FRAMES);
    is_open(harness)
}

/// What goes wrong with the list at `button`, each a line naming it
fn check_list(harness: &mut Harness<'_, TextureCraftApp>, button: Rect, name: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let gesture_start = gesture_start(harness);
    if !open(harness, button) {
        return vec![format!("{name}: a click does not open it")];
    }

    if !zooms(harness, gesture_start) {
        problems.push(format!("{name}: the wheel over the view does not zoom while it is open"));
    }

    // Each gesture starts with the list open, whatever the one before did to it
    if !is_open(harness) && !open(harness, button) {
        return [problems, vec![format!("{name}: it does not open again after the wheel")]].concat();
    }

    if !pans(harness, gesture_start) {
        problems.push(format!("{name}: a middle drag in the view does not pan while it is open"));
    }

    // A middle drag may close it, as a click outside does; the orbit is checked with it open again
    if !is_open(harness) && !open(harness, button) {
        return [problems, vec![format!("{name}: a second click does not open it")]].concat();
    }

    if !orbits(harness, gesture_start) {
        problems.push(format!("{name}: an Alt drag in the view does not orbit while it is open"));
    }

    if !is_open(harness) && !open(harness, button) {
        return [problems, vec![format!("{name}: it does not open again")]].concat();
    }

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);
    if is_open(harness) {
        problems.push(format!("{name}: Escape does not close it"));
        harness.key_press(egui::Key::Escape);
        harness.run_steps(SETTLE_FRAMES);
    }

    if !open(harness, button) {
        problems.push(format!("{name}: it does not reopen before the outside click"));
        return problems;
    }

    click_at(harness, gesture_start);
    if is_open(harness) {
        problems.push(format!("{name}: a click in the view does not close it"));
        return problems;
    }

    if !open(harness, button) {
        problems.push(format!("{name}: it does not reopen before the stroke"));
        return problems;
    }

    drag(harness, gesture_start, GESTURE, PointerButton::Primary, Modifiers::NONE);
    if is_open(harness) {
        problems.push(format!("{name}: a stroke on the model does not close it"));
    }

    problems
}

#[test]
fn every_list_opens_leaves_the_view_its_camera_and_closes_on_escape_a_click_and_a_stroke_outside() {
    let mut problems = Vec::new();
    for (screen, mut harness) in screens() {
        let count = lists(&harness).len();
        assert!(count > 0, "{screen}: lists show");

        for index in 0..count {
            // Looked up again each time: a closed list or a stroke may move the rows below
            let Some((button, shown)) = lists(&harness).get(index).cloned() else {
                problems.push(format!("{screen}: list {index} of {count} is gone"));
                continue;
            };

            problems.extend(check_list(&mut harness, button, &format!("{screen}: {shown}")));
            for _ in 0..ESCAPES_TO_CLEAR {
                harness.key_press(egui::Key::Escape);
                harness.run_steps(SETTLE_FRAMES);
            }
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_colour_picker_window_and_the_symmetry_settings_leave_the_view_its_wheel_middle_drag_and_alt_orbit() {
    let (mut harness, _) = fill_app(json!({"color": [0.2, 0.2, 0.2]}), None, Some("Material"), PROPERTIES_WINDOW);
    click_node(&mut harness, Role::Button, "Base color");
    let start = gesture_start(&harness);
    let picker_open = |harness: &Harness<'_, TextureCraftApp>| harness.query_by_role_and_label(Role::TitleBar, "Base color").is_some();
    assert!(picker_open(&harness), "the colour bar opens its picker");
    assert!(zooms(&mut harness, start), "the wheel over the view zooms with the picker open");
    assert!(pans(&mut harness, start), "a middle drag pans with the picker open");
    assert!(orbits(&mut harness, start), "an Alt drag orbits with the picker open");
    assert!(picker_open(&harness), "the picker stays open through the camera moves, as a window beside the panel");

    let mut harness = screen(|_| {});

    // The paint controls overflow the bar at this width; its arrow brings the Symmetry buttons into view
    harness.get_by_label("Scroll right").click();
    harness.run_steps(SETTLE_FRAMES);
    harness.get_by_label("Symmetry settings").click();
    harness.run_steps(SETTLE_FRAMES);
    let settings_open = |harness: &Harness<'_, TextureCraftApp>| harness.query_by_label("Radial symmetry").is_some();
    assert!(settings_open(&harness), "the settings open");

    let start = gesture_start(&harness);
    assert!(zooms(&mut harness, start), "the wheel over the view zooms with the symmetry settings open");
    assert!(settings_open(&harness), "the wheel leaves the settings open");
    assert!(pans(&mut harness, start), "a middle drag pans with the symmetry settings open");
    assert!(orbits(&mut harness, start), "an Alt drag orbits with the symmetry settings open");
}
