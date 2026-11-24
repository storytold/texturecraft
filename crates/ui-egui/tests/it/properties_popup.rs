//! A plain right click in a view opens the Properties panel's contents at the cursor; right drags keep their gestures

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::LayerKind;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::properties_popup::{self, PropertiesPopup};
use texturecraft_ui_egui::state::{Mode, ViewLayout};

use crate::{BAKE_RESOLUTION, DRAG_STEP_FRAMES, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// A common laptop window
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// Tall enough that the docked list ends before a fill's MATERIAL caption reaches its top
const JUMP_TAB_WINDOW: egui::Vec2 = egui::vec2(1600.0, 1300.0);

/// Width of the docked jump tab strip, from its Fill tab across its other two
const DOCKED_STRIP_WIDTH: f32 = 300.0;

/// Frames for a jump tab's scroll to land
const JUMP_FRAMES: usize = 4;

/// Frames for a selection made through the engine to show
const SELECT_FRAMES: usize = 3;

/// Pointer moves a drag makes between its press and its release
const DRAG_MOVES: usize = 4;

/// Frames for a scroll to a popup row to finish its animation
const SCROLL_FRAMES: usize = 25;

/// How far left of the popup a click lands on the model
const MODEL_LEFT_OF_POPUP: f32 = 60.0;

fn harness() -> Harness<'static, TextureCraftApp> {
    harness_sized(WINDOW_SIZE)
}

/// The sample project's app in a window of `size`, speaking English
fn harness_sized(size: egui::Vec2) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder().with_size(size).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

fn view_center(harness: &Harness<'_, TextureCraftApp>) -> Pos2 {
    harness.state().viewport_rect.center()
}

fn is_open(harness: &Harness<'_, TextureCraftApp>) -> bool {
    matches!(harness.state().properties_popup, Some(PropertiesPopup::Open { .. }))
}

/// Press and release `button` at `at` without moving
fn click(harness: &mut Harness<'_, TextureCraftApp>, button: PointerButton, at: Pos2) {
    harness.hover_at(at);
    harness.event(Event::PointerButton { pos: at, button, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: at, button, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(SETTLE_FRAMES);
}

/// Drag with `button` from `from` by `by` in a few moves while `modifiers` are held
fn drag(harness: &mut Harness<'_, TextureCraftApp>, button: PointerButton, from: Pos2, by: egui::Vec2, modifiers: Modifiers) {
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
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

#[test]
fn a_right_click_opens_the_popup_at_the_cursor_in_both_views() {
    let mut harness = harness();
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    assert!(matches!(harness.state().properties_popup, Some(PropertiesPopup::Open { at, .. }) if at == center));

    harness.key_press(Key::Escape);
    harness.run_steps(2);
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(2);
    click(&mut harness, PointerButton::Secondary, center);
    assert!(is_open(&harness), "the 2D view opens it too");
}

#[test]
fn right_drags_keep_their_gestures_and_never_open_it() {
    let mut harness = harness();
    let center = view_center(&harness);
    let yaw = harness.state().session.camera.yaw;
    drag(&mut harness, PointerButton::Secondary, center, vec2(80.0, -40.0), Modifiers::NONE);
    assert_ne!(harness.state().session.camera.yaw, yaw, "a right drag orbits");
    assert!(!is_open(&harness), "an orbit doesn't open it");

    let size = harness.state().session.tool.brush.size;
    drag(&mut harness, PointerButton::Secondary, center, vec2(80.0, -40.0), Modifiers::COMMAND);
    assert!(harness.state().session.tool.brush.size > size, "Ctrl+right drag sets the brush");
    assert!(!is_open(&harness));

    for modifiers in [Modifiers::ALT, Modifiers::SHIFT] {
        drag(&mut harness, PointerButton::Secondary, center, vec2(80.0, -40.0), modifiers);
        assert!(!is_open(&harness), "{modifiers:?}");
    }
}

#[test]
fn a_click_outside_closes_it_without_painting() {
    let mut harness = harness();
    let center = view_center(&harness);
    let outside = harness.state().viewport_rect.left_center() + vec2(20.0, 0.0);
    let history = harness.state().session.history_labels().len();
    click(&mut harness, PointerButton::Secondary, center);
    assert!(is_open(&harness));

    click(&mut harness, PointerButton::Primary, outside);
    assert_eq!(harness.state().properties_popup, None);
    assert_eq!(harness.state().session.history_labels().len(), history, "the closing click doesn't paint");

    click(&mut harness, PointerButton::Secondary, center);
    click(&mut harness, PointerButton::Secondary, outside);
    assert_eq!(harness.state().properties_popup, None, "a right click outside closes it and doesn't reopen it");
}

#[test]
fn escape_closes_it() {
    let mut harness = harness();
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    assert!(is_open(&harness));
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().properties_popup, None);
}

#[test]
fn dragging_its_size_slider_changes_the_brush() {
    let mut harness = harness();
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();

    // The Size section's header sits above its slider of the same name
    let slider_label = harness
        .query_all_by_label("Size")
        .map(|node| node.rect())
        .filter(|rect| popup.contains_rect(*rect))
        .max_by(|first, second| first.top().total_cmp(&second.top()))
        .unwrap();
    let track_y = slider_label.bottom() + 10.0;
    let size = harness.state().session.tool.brush.size;
    harness.drag_at(egui::pos2(slider_label.left() + 20.0, track_y));
    harness.run_steps(2);
    harness.hover_at(egui::pos2(popup.right() - 30.0, track_y));
    harness.run_steps(2);
    harness.drop_at(egui::pos2(popup.right() - 30.0, track_y));
    harness.run_steps(3);
    assert!(harness.state().session.tool.brush.size > size, "{size} -> {}", harness.state().session.tool.brush.size);
    assert!(is_open(&harness), "editing it keeps it open");
}

#[test]
fn it_shows_what_the_properties_panel_shows() {
    let mut harness = harness();
    let fill = harness.state().session.doc.project.layers.values().find(|layer| matches!(layer.kind, LayerKind::Fill(_))).map(|layer| layer.id.0).unwrap();
    let selected = texturecraft_engine::execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": fill}));

    assert!(selected.is_ok(), "{selected:?}");

    let projected = texturecraft_engine::execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": fill, "projection": "uv"}));

    assert!(projected.is_ok(), "{projected:?}");

    harness.run_steps(3);

    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();
    let in_popup = |label: &str| harness.query_all_by_label(label).any(|node| popup.contains_rect(node.rect()));
    assert!(in_popup("UV projection"), "a selected fill shows its fill settings");
    assert!(!in_popup("Minimum size (%)"), "not the brush's");
}

/// Whether the jump tab `name` inside `area` is the lit one
fn tab_lit(harness: &Harness<'_, TextureCraftApp>, area: egui::Rect, name: &str) -> bool {
    let tab = harness.get_all_by_role_and_label(Role::Tab, name).find(|tab| area.contains_rect(tab.rect()));
    tab.unwrap_or_else(|| panic!("a {name} tab in {area:?}")).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn its_jump_tabs_scroll_to_a_section_and_light_it_without_moving_the_docked_panel() {
    let mut harness = harness_sized(JUMP_TAB_WINDOW);
    let fill = harness.state().session.doc.project.layers.values().find(|layer| matches!(layer.kind, LayerKind::Fill(_))).map(|layer| layer.id.0).unwrap();
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.select", &json!({"layer": fill})).unwrap();
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.fill.projection", &json!({"layer": fill, "projection": "uv"})).unwrap();
    harness.run_steps(SELECT_FRAMES);

    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();
    let docked_fill = harness.get_all_by_role_and_label(Role::Tab, "Fill").map(|tab| tab.rect()).find(|tab| !popup.contains_rect(*tab)).unwrap();

    let docked = egui::Rect::from_min_max(docked_fill.min, docked_fill.max + vec2(DOCKED_STRIP_WIDTH, 0.0));
    let in_popup = |harness: &Harness<'_, TextureCraftApp>, label: &str| harness.query_all_by_label(label).any(|node| popup.contains_rect(node.rect()));

    assert!(tab_lit(&harness, popup, "Fill"), "it opens on Fill");
    assert!(in_popup(&harness, "UV projection"));

    let material = harness.get_all_by_role_and_label(Role::Tab, "Material").find(|tab| popup.contains_rect(tab.rect())).unwrap().rect().center();
    click(&mut harness, PointerButton::Primary, material);
    harness.run_steps(JUMP_FRAMES);

    assert!(is_open(&harness), "a tab click keeps it open");
    assert!(!in_popup(&harness, "UV projection"), "the list jumped down to MATERIAL");
    assert!(tab_lit(&harness, popup, "Material"), "the section in view is lit");
    assert!(!tab_lit(&harness, popup, "Fill"));
    assert!(tab_lit(&harness, docked, "Fill"), "the docked panel keeps its own place");
}

#[test]
fn bake_and_render_modes_never_open_it() {
    let mut harness = harness();
    let center = view_center(&harness);
    for mode in [Mode::Baking, Mode::Rendering] {
        harness.state_mut().ui.mode = mode;
        harness.run_steps(3);
        click(&mut harness, PointerButton::Secondary, center);

        assert_eq!(harness.state().properties_popup, None, "{mode:?}");
    }
}

#[test]
fn the_colour_picker_opened_in_the_popup_takes_its_presses_and_the_first_escape() {
    let mut harness = harness();
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.add.fill", &json!({"color": [0.2, 0.2, 0.2]})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);

    assert!(is_open(&harness));

    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();
    let in_popup = |rect: egui::Rect| popup.x_range().contains(rect.center().x);
    harness.get_all_by_role_and_label(Role::Button, "Base color").find(|node| in_popup(node.rect())).unwrap().scroll_to_me();

    harness.run_steps(SCROLL_FRAMES);
    let button = harness.get_all_by_role_and_label(Role::Button, "Base color").find(|node| in_popup(node.rect())).unwrap().rect();
    click(&mut harness, PointerButton::Primary, button.center());
    let title = harness.get_by_role_and_label(Role::TitleBar, "Base color").rect();

    click(&mut harness, PointerButton::Primary, title.center());

    assert!(is_open(&harness), "a press in the picker window stays in the popup");
    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_some(), "and keeps the picker open");

    harness.key_press(Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_none(), "the first Escape closes the picker");
    assert!(is_open(&harness), "and leaves the popup open");

    harness.key_press(Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(!is_open(&harness), "the next Escape closes the popup");
}

#[test]
fn an_eyedropper_pick_from_the_popup_keeps_the_popup_open_for_its_colour() {
    let mut harness = harness();
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.add.fill", &json!({"color": [0.2, 0.2, 0.2]})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    let center = view_center(&harness);
    click(&mut harness, PointerButton::Secondary, center);
    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();
    let in_popup = |rect: egui::Rect| popup.x_range().contains(rect.center().x);
    harness.get_all_by_role_and_label(Role::Button, "Pick color").find(|node| in_popup(node.rect())).unwrap().scroll_to_me();

    harness.run_steps(SCROLL_FRAMES);
    let button = harness.get_all_by_role_and_label(Role::Button, "Pick color").find(|node| in_popup(node.rect())).unwrap().rect();
    click(&mut harness, PointerButton::Primary, button.center());

    assert!(harness.state().picking_color.is_some(), "the eyedropper waits for a click on the model");

    // The model's middle, left of the popup
    let model = egui::pos2(popup.left() - MODEL_LEFT_OF_POPUP, center.y);
    click(&mut harness, PointerButton::Primary, model);

    assert!(harness.state().picking_color.is_none(), "the click on the model picks");
    assert!(is_open(&harness), "and leaves the popup open for the picked colour");
}
