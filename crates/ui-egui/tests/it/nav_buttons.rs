//! The on-screen orbit, pan and zoom buttons move the views with a primary-button drag, without painting.
//! A pen tip reaches egui as the same pointer events a mouse sends, so these drags stand for a pen drag too.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::nav_buttons::{NavButton, NavView, button_rect};
use texturecraft_ui_egui::state::{UiState, ViewLayout};
use texturecraft_ui_egui::ui_state_store::UiStateStore;

mod memory_storage;

use memory_storage::MemoryStorage;

const WINDOW: egui::Vec2 = vec2(1280.0, 800.0);

/// The app showing only one view, so that the view fills the viewport rect the 3D view reports
fn app(layout: ViewLayout) -> (Harness<'static, TextureCraftApp>, Rect) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW).with_pixels_per_point(1.0).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().session.tool.tool = texturecraft_engine::Tool::Paint;
    harness.state_mut().ui.layout = ViewLayout::Only3D;
    harness.run_steps(4);
    let view = harness.state().viewport_rect;
    harness.state_mut().ui.layout = layout;
    harness.run_steps(4);
    (harness, view)
}

/// Zoom the camera in and shift the 2D view so that the model and the texture lie under the buttons: a press there paints
fn fill_views(harness: &mut Harness<'_, TextureCraftApp>) {
    harness.state_mut().session.camera.distance = 1.2 * harness.state().session.camera.scene_radius;
    harness.state_mut().ui.uv_pan = [0.9, 0.0];
    harness.run_steps(4);
}

/// Press at `from` with the primary button, move to `to` in steps and release
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.run_steps(2);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);
}

fn drag_button(harness: &mut Harness<'_, TextureCraftApp>, view: Rect, kind: NavView, button: NavButton, by: egui::Vec2) {
    let from = button_rect(view, kind, button).expect("the button is shown").center();
    drag(harness, from, from + by);
}

/// What painting changes: the undo steps and the tiles the selected layer's base color has allocated
fn history(harness: &Harness<'_, TextureCraftApp>) -> (Vec<String>, usize) {
    let session = &harness.state().session;
    let layer = session.selected_layer().expect("a selected layer");
    let tiles = session.doc.pixels.layer(layer, texturecraft_engine::core::Channel::BaseColor).map_or(0, |image| image.allocated_count());
    (session.history_labels(), tiles)
}

#[test]
fn dragging_the_orbit_button_turns_the_camera_and_paints_nothing() {
    let (mut harness, view) = app(ViewLayout::Only3D);
    fill_views(&mut harness);
    let (camera, labels) = (harness.state().session.camera, history(&harness));

    drag_button(&mut harness, view, NavView::Scene, NavButton::Orbit, vec2(40.0, 20.0));

    let after = harness.state().session.camera;
    assert!(after.yaw != camera.yaw && after.pitch != camera.pitch, "orbit: {camera:?} -> {after:?}");
    assert_eq!((after.target, after.distance), (camera.target, camera.distance), "only the angles change");
    assert_eq!(history(&harness), labels, "nothing painted");
}

#[test]
fn orbit_button_turns_the_camera_as_far_as_the_mouse_drag_does() {
    let (mut harness, view) = app(ViewLayout::Only3D);
    let start = harness.state().session.camera;
    drag_button(&mut harness, view, NavView::Scene, NavButton::Orbit, vec2(40.0, 0.0));
    let by_button = harness.state().session.camera.yaw - start.yaw;

    // The same drag with the right mouse button over the view
    let (mut reference, view) = app(ViewLayout::Only3D);
    let from = view.center();
    reference.hover_at(from);
    reference.run_steps(2);
    reference.event(Event::PointerButton { pos: from, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    reference.run_steps(2);
    for step in 1..=4 {
        reference.hover_at(from + vec2(40.0, 0.0) * (step as f32 / 4.0));
        reference.run_steps(1);
    }
    reference.event(Event::PointerButton { pos: from + vec2(40.0, 0.0), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    reference.run_steps(3);
    let by_mouse = reference.state().session.camera.yaw - start.yaw;

    assert!(by_button != 0.0 && (by_button - by_mouse).abs() < 1e-4, "button {by_button}, mouse {by_mouse}");
}

#[test]
fn dragging_the_pan_button_moves_the_camera_target_and_paints_nothing() {
    let (mut harness, view) = app(ViewLayout::Only3D);
    let (camera, labels) = (harness.state().session.camera, history(&harness));

    drag_button(&mut harness, view, NavView::Scene, NavButton::Pan, vec2(-30.0, 20.0));

    let after = harness.state().session.camera;
    assert!((after.target - camera.target).length() > 1e-3, "pan: {camera:?} -> {after:?}");
    assert_eq!((after.yaw, after.pitch, after.distance), (camera.yaw, camera.pitch, camera.distance));
    assert_eq!(history(&harness), labels, "nothing painted");
}

#[test]
fn dragging_the_zoom_button_up_moves_closer_and_down_moves_away() {
    let (mut harness, view) = app(ViewLayout::Only3D);
    let (camera, labels) = (harness.state().session.camera, history(&harness));

    drag_button(&mut harness, view, NavView::Scene, NavButton::Zoom, vec2(0.0, -20.0));
    let closer = harness.state().session.camera;
    assert!(closer.distance < camera.distance, "{} -> {}", camera.distance, closer.distance);
    assert_eq!((closer.yaw, closer.pitch, closer.target), (camera.yaw, camera.pitch, camera.target));

    drag_button(&mut harness, view, NavView::Scene, NavButton::Zoom, vec2(0.0, 40.0));
    assert!(harness.state().session.camera.distance > closer.distance);
    assert_eq!(history(&harness), labels, "nothing painted");
}

#[test]
fn the_2d_view_pans_and_zooms_with_its_buttons_and_paints_nothing() {
    let (mut harness, view) = app(ViewLayout::Only2D);
    let (zoom, pan, labels, camera) = {
        let state = harness.state();
        (state.ui.uv_zoom, state.ui.uv_pan, history(&harness), state.session.camera)
    };

    drag_button(&mut harness, view, NavView::Uv, NavButton::Pan, vec2(40.0, 20.0));
    let panned = harness.state().ui.uv_pan;
    assert!(panned[0] > pan[0] && panned[1] < pan[1], "dragging right and down moves the texture right and down: {pan:?} -> {panned:?}");
    assert_eq!(harness.state().ui.uv_zoom, zoom);

    drag_button(&mut harness, view, NavView::Uv, NavButton::Zoom, vec2(0.0, -30.0));
    let zoomed_in = harness.state().ui.uv_zoom;
    assert!(zoomed_in > zoom, "dragging up zooms in");
    drag_button(&mut harness, view, NavView::Uv, NavButton::Zoom, vec2(0.0, 60.0));
    assert!(harness.state().ui.uv_zoom < zoomed_in, "dragging down zooms out");
    assert_eq!(history(&harness), labels, "nothing painted");
    assert_eq!(harness.state().session.camera, camera, "the 3D camera stays");
}

#[test]
fn the_same_drags_paint_where_there_is_no_button() {
    let (mut harness, view) = app(ViewLayout::Only3D);
    fill_views(&mut harness);
    harness.state_mut().ui.nav_buttons = false;
    harness.run_steps(2);
    let labels = history(&harness);
    drag_button(&mut harness, view, NavView::Scene, NavButton::Orbit, vec2(40.0, 20.0));
    assert!(history(&harness) != labels, "with the buttons off the press paints, so the other tests prove the buttons stop it");

    let (mut harness, view) = app(ViewLayout::Only2D);
    fill_views(&mut harness);
    harness.state_mut().ui.nav_buttons = false;
    harness.run_steps(2);
    let labels = history(&harness);
    drag_button(&mut harness, view, NavView::Uv, NavButton::Pan, vec2(40.0, 20.0));
    assert!(history(&harness) != labels, "the 2D view paints there too");
}

#[test]
fn a_press_in_the_gap_between_buttons_or_in_the_margin_paints_nothing() {
    for (layout, kind) in [(ViewLayout::Only3D, NavView::Scene), (ViewLayout::Only2D, NavView::Uv)] {
        let (mut harness, view) = app(layout);
        fill_views(&mut harness);
        let first = button_rect(view, kind, kind.buttons()[0]).unwrap();
        let labels = history(&harness);
        let gap = first.left_bottom() + vec2(15.0, 2.0);
        drag(&mut harness, gap, gap + vec2(30.0, 10.0));
        let margin = first.right_center() + vec2(3.0, 0.0);
        drag(&mut harness, margin, margin + vec2(-30.0, 10.0));
        assert_eq!(history(&harness), labels, "{kind:?}: nothing painted");
    }
}

#[test]
fn the_2d_view_has_no_orbit_button() {
    let view = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0));
    assert!(button_rect(view, NavView::Uv, NavButton::Orbit).is_none());
    assert!(button_rect(view, NavView::Scene, NavButton::Orbit).is_some());
}

#[test]
fn buttons_stay_clear_of_the_pickers_the_gizmo_and_the_split_divider_at_1280_by_800() {
    let (harness, _) = app(ViewLayout::Split);
    let view = harness.state().viewport_rect;
    assert!(view.width() >= 300.0, "the pickers fit the 3D side: {view:?}");

    // Keep navigation buttons clear of the pickers, axis gizmo, set name, and divider
    let pickers = Rect::from_min_size(egui::pos2(view.center().x - 150.0, view.top() + 8.0), vec2(300.0, 22.0));
    let gizmo = Rect::from_min_size(egui::pos2(view.left(), view.bottom() - 56.0), vec2(80.0, 56.0));
    let set_name = Rect::from_min_size(egui::pos2(view.right() - 160.0, view.bottom() - 24.0), vec2(160.0, 24.0));
    for button in NavView::Scene.buttons() {
        let at = button_rect(view, NavView::Scene, *button).expect("shown at 1280 x 800");
        assert!(view.contains_rect(at), "{button:?} inside the view");
        assert!(!at.intersects(pickers) && !at.intersects(gizmo) && !at.intersects(set_name), "{button:?} overlaps an overlay");
        assert!(view.right() - at.right() >= 4.0, "{button:?} stays off the divider's grab zone");
    }

    // The 2D side is as wide as the 3D side or wider at the default ratio
    let uv_view = Rect::from_min_size(egui::pos2(view.right() + 2.0, view.top()), vec2(view.width(), view.height()));
    for button in NavView::Uv.buttons() {
        let at = button_rect(uv_view, NavView::Uv, *button).expect("shown at 1280 x 800");
        assert!(!at.intersects(Rect::from_min_size(egui::pos2(uv_view.center().x - 150.0, uv_view.top() + 8.0), vec2(300.0, 22.0))));
        assert!(!at.intersects(Rect::from_min_size(egui::pos2(uv_view.right() - 72.0, uv_view.bottom() - 72.0), vec2(72.0, 72.0))), "the axis gizmo");
    }
}

#[test]
fn views_too_small_for_the_column_show_no_buttons() {
    assert!(button_rect(Rect::from_min_size(Pos2::ZERO, vec2(30.0, 600.0)), NavView::Scene, NavButton::Pan).is_none());
    assert!(button_rect(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 120.0)), NavView::Scene, NavButton::Pan).is_none());
}

#[test]
fn the_view_menu_item_hides_the_buttons_and_the_choice_is_saved() {
    let (mut harness, view) = app(ViewLayout::Only3D);
    assert!(harness.state().ui.nav_buttons, "shown by default");
    let context = harness.ctx.clone();
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "ui.navButtons");
    harness.run_steps(3);
    assert!(!harness.state().ui.nav_buttons);

    // A drag where the orbit button was no longer turns the camera
    let camera = harness.state().session.camera;
    drag_button(&mut harness, view, NavView::Scene, NavButton::Orbit, vec2(40.0, 20.0));
    assert_eq!(harness.state().session.camera, camera, "the button is gone");

    // Saved with the other UI settings and read back; a state saved before the setting existed shows the buttons
    let mut storage = MemoryStorage::default();
    UiStateStore::default().save(&harness.state().ui, &mut storage);
    let restored = UiStateStore::default().load(Some(&storage)).expect("saved state loads");
    assert!(!restored.nav_buttons);

    let mut saved: serde_json::Value = serde_json::from_str(&storage.0["ui.json"]).unwrap();
    assert!(saved.as_object_mut().unwrap().remove("nav_buttons").is_some(), "the saved state names the setting");
    storage.0.insert("ui.json".to_owned(), saved.to_string());
    assert!(UiStateStore::default().load(Some(&storage)).expect("an older saved state loads").nav_buttons);
    assert!(UiState::default().nav_buttons);

    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "ui.navButtons");
    harness.run_steps(3);
    drag_button(&mut harness, view, NavView::Scene, NavButton::Orbit, vec2(40.0, 20.0));
    assert_ne!(harness.state().session.camera, camera, "shown again");
}
