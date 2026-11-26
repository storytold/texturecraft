//! The warp controls of the top tool bar while the selected fill projects through a warp, in place of the tool's own: the transform tool's settings, the warp mode, the gizmo's mode and space, the mirrors and the resets

mod gizmo_mode;
mod gizmo_space;
mod snap;
mod warp_target;
mod warp_toolbar_settings;

use egui::Ui;
use egui_phosphor::regular as ic;
use serde_json::json;

pub use gizmo_mode::GizmoMode;
pub use gizmo_space::GizmoSpace;
pub(crate) use snap::to_step;
pub use warp_toolbar_settings::WarpToolbarSettings;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::toolbar::icon_dropdown;
use crate::widgets::{icon_button, icon_dropdown_button};
use texturecraft_engine::core::Projection;
pub(crate) use warp_target::WarpTarget;
use warp_toolbar_settings::{ANGLE_STEPS, GRID_STEPS, MANIPULATOR_SIZES};

/// The width of the settings popup (points)
const POPUP_WIDTH: f32 = 220.0;

/// The gizmo mode buttons: mode, icon, label
const MODES: [(GizmoMode, &str, &str); 4] = [
    (GizmoMode::Move, ic::ARROWS_OUT_CARDINAL, "Move"),
    (GizmoMode::Rotate, ic::ARROWS_CLOCKWISE, "Rotate"),
    (GizmoMode::Scale, ic::CORNERS_OUT, "Scale"),
    (GizmoMode::PlaceOnSurface, ic::ARROW_LINE_DOWN, "Place on surface"),
];

/// The warp modes the bar lists; only the first works until the warp has a grid of vertices
const WARP_MODES: [&str; 5] = ["Transform warp", "Edit vertices", "Split both directions", "Split horizontally", "Split vertically"];

/// Why the warp modes past the first and the vertex reset are greyed out
const NEEDS_GRID: &str = "Needs the warp grid, which is not available yet";

/// The warp controls of the top tool bar, left to right
pub(crate) fn controls(app: &mut TextureCraftApp, ui: &mut Ui, target: WarpTarget) {
    let size = ui.spacing().interact_size.y;

    // The only tool there is, so it stays on
    icon_button(ui, ic::BOUNDING_BOX, tr("Transform tool"), true, size);
    manipulator_settings(app, ui, size);
    warp_mode(ui);
    ui.separator();

    let current = app.ui.warp_toolbar.mode;
    for (mode, icon, label) in MODES {
        if icon_button(ui, icon, tr(label), current == mode, size).clicked() {
            app.ui.warp_toolbar.mode = mode;
        }
    }
    gizmo_space(app, ui);
    ui.separator();

    for (axis, icon, label, field) in [(0, ic::FLIP_HORIZONTAL, "Mirror horizontally", "flipU"), (1, ic::FLIP_VERTICAL, "Mirror vertically", "flipV")] {
        if icon_button(ui, icon, tr(label), target.flip[axis], size).clicked() {
            let mut fields = json!({});
            fields[field] = json!(!target.flip[axis]);
            app.run("layer.decal.set", target.params(fields));
        }
    }
    resets(app, ui, &target);
}

/// The settings popup of the transform tool: the gizmo's size and the steps its drags snap to
fn manipulator_settings(app: &mut TextureCraftApp, ui: &mut Ui, size: f32) {
    let popup_id = ui.make_persistent_id("warp_manipulator_settings");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = icon_dropdown_button(ui, ic::GEAR, tr("Manipulator settings"), open, size);
    let response = if open { response } else { response.on_hover_text(tr("Manipulator settings")) };

    // Sliders sit in the popup, so a click inside must not close it
    egui::Popup::menu(&response).id(popup_id).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_width(POPUP_WIDTH);
        ui.label(egui::RichText::new(tr("MANIPULATOR SETTINGS")).strong());

        let settings = &mut app.ui.warp_toolbar;
        ui.add(egui::Slider::new(&mut settings.manipulator_size, MANIPULATOR_SIZES).text(tr("Manipulator size")).fixed_decimals(2));
        ui.add(egui::Slider::new(&mut settings.grid_steps, GRID_STEPS).text(tr("Grid steps")).fixed_decimals(2))
            .on_hover_text(tr("Hold Ctrl while dragging to move in these steps"));
        ui.add(egui::Slider::new(&mut settings.angle_steps, ANGLE_STEPS).text(tr("Angle steps")).fixed_decimals(0))
            .on_hover_text(tr("Hold Ctrl while dragging to turn in these steps"));
    });
}

/// The warp mode list: Transform warp works; the others wait for the warp grid
fn warp_mode(ui: &mut Ui) {
    icon_dropdown(ui, "warp_mode", ic::FRAME_CORNERS, tr(WARP_MODES[0]), |ui| {
        for (index, label) in WARP_MODES.into_iter().enumerate() {
            let unavailable = (index > 0).then(|| tr(NEEDS_GRID));
            menu_row(ui, tr(label), index == 0, unavailable);
        }
    });
}

/// Object space or World space: the directions the move arrows follow
fn gizmo_space(app: &mut TextureCraftApp, ui: &mut Ui) {
    let spaces = [(GizmoSpace::Object, ic::CUBE, "Object space"), (GizmoSpace::World, ic::GLOBE, "World space")];
    let current = app.ui.warp_toolbar.space;
    let (_, icon, label) = spaces.iter().find(|(space, ..)| *space == current).unwrap_or(&spaces[0]);
    let mut chosen = None;
    icon_dropdown(ui, "warp_space", icon, tr(label), |ui| {
        for (space, _, label) in spaces {
            if menu_row(ui, tr(label), current == space, None) {
                chosen = Some(space);
            }
        }
    });

    if let Some(space) = chosen {
        app.ui.warp_toolbar.space = space;
    }
}

/// The reset list: the frame back around the mesh, or the corners back on the frame
fn resets(app: &mut TextureCraftApp, ui: &mut Ui, target: &WarpTarget) {
    let mut command = None;
    icon_dropdown(ui, "warp_reset", ic::ARROW_COUNTER_CLOCKWISE, tr("Reset"), |ui| {
        if menu_row(ui, tr("Reset warp transform"), false, None) {
            command = Some(("layer.fill.resetFrame", json!({"keepCorners": true})));
        }
        if menu_row(ui, tr("Reset all vertices"), false, None) {
            command = Some(("layer.fill.projection", json!({"warpCorners": Projection::FRAME_CORNERS})));
        }

        // Vertices cannot be selected before the warp has a grid
        menu_row(ui, tr("Reset selected vertices"), false, Some(tr(NEEDS_GRID)));
    });

    if let Some((id, fields)) = command {
        app.run(id, target.params(fields));
    }
}

/// One list entry; `unavailable` greys it out and says why on hover. Returns true when clicked
fn menu_row(ui: &mut Ui, label: &str, selected: bool, unavailable: Option<&str>) -> bool {
    let response = ui.add_enabled(unavailable.is_none(), egui::Button::new(label).selected(selected));
    if let Some(reason) = unavailable {
        response.on_disabled_hover_text(reason);
        return false;
    }

    response.clicked()
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui::{Modifiers, Pos2, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use texturecraft_engine::core::Projection;
    use texturecraft_engine::geom::glam::{Vec2, Vec3};

    use super::*;
    use crate::frame_gizmo::FrameGrab;
    use crate::frame_gizmo::tests::{drag, fill_app, gizmo, projection};
    use crate::projection_gizmo::Gizmo;

    fn click_label(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
        harness.get_by_label(label).click();
        harness.run_steps(2);
    }

    /// The warp's frame and corners
    fn warp_parts(harness: &Harness<'_, TextureCraftApp>, layer: texturecraft_engine::core::LayerId) -> (texturecraft_engine::core::Decal, [[f32; 2]; 4]) {
        let Projection::Warp { frame, corners, .. } = projection(harness, layer) else { panic!("a warp") };
        (frame, corners)
    }

    #[test]
    fn the_bar_shows_for_a_warp_fill_only() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");

        assert!(harness.query_by_label("Transform tool").is_some());

        harness.state_mut().run("layer.fill.projection", json!({"layer": layer.0, "projection": "planar"}));
        harness.run_steps(2);

        assert!(harness.query_by_label("Transform tool").is_none(), "a planar frame has no warp tools");

        let away = harness.state().viewport_rect.left_bottom() + vec2(10.0, -10.0);
        assert_eq!(gizmo(&harness).grab(away), None, "a planar frame has no Place on surface");
    }

    #[test]
    fn each_mode_offers_only_its_handles() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, _) = fill_app("warp");
        let start = gizmo(&harness);
        click_label(&mut harness, "Rotate");
        let ring = gizmo(&harness).ring.expect("a frame facing the camera shows its ring");
        let ring_point = *ring.iter().max_by(|first, second| first.y.total_cmp(&second.y)).unwrap();
        let handles: [(FrameGrab, Pos2, GizmoMode); 4] = [
            (FrameGrab::Slide, start.center, GizmoMode::Move),
            (FrameGrab::Axis(0), start.arrows[0].unwrap(), GizmoMode::Move),
            (FrameGrab::Turn, ring_point, GizmoMode::Rotate),
            (FrameGrab::Side(1), start.sides[1], GizmoMode::Scale),
        ];

        for (_, _, label) in MODES {
            click_label(&mut harness, label);
            let shown = gizmo(&harness);
            let mode = shown.tools.unwrap().mode;

            for (grab, at, offered_in) in handles {
                assert_eq!(shown.grab(at) == Some(grab), mode == offered_in, "{grab:?} in {mode:?}");
            }

            assert_eq!(shown.grab(shown.corners[1]), Some(FrameGrab::Corner(1)), "a warp's corners stay in {mode:?}");
        }
    }

    #[test]
    fn place_on_surface_puts_the_frame_where_the_pointer_touches_the_model() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");
        let rect = harness.state().viewport_rect;
        let middle = crate::viewport::project(harness.state(), rect, Vec3::ZERO).unwrap();
        let (from, onto) = (middle + vec2(30.0, 20.0), middle + vec2(-60.0, -40.0));
        let steps = harness.state().session.history_labels().len();
        let (before, corners_before) = warp_parts(&harness, layer);
        click_label(&mut harness, "Place on surface");

        drag(&mut harness, from, onto, Modifiers::NONE);

        let (placed, corners) = warp_parts(&harness, layer);
        let on_view = onto - rect.min;
        let hit = harness.state().session.pick(Vec2::new(on_view.x, on_view.y), harness.state().session.viewport).expect("the model is under the pointer");
        assert!(Vec3::from(placed.center).distance(hit.pos) < 1e-2, "the frame centre sits at the surface hit: {:?} {:?}", placed.center, hit.pos);
        assert!(Vec3::from(placed.normal).dot(hit.nrm) > 0.99, "the frame faces along the surface normal");
        assert_eq!((placed.size, placed.rotation, corners), (before.size, before.rotation, corners_before), "its size, rotation and corners stay");
        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step per drag");

        // Past the model's edge the frame waits where it last touched it
        drag(&mut harness, onto, rect.left_bottom() + vec2(10.0, -10.0), Modifiers::NONE);

        let (waiting, _) = warp_parts(&harness, layer);
        assert!((Vec3::from(waiting.center).length() - 1.0).abs() < 0.05, "still on the ball: {:?}", waiting.center);
    }

    #[test]
    fn place_on_surface_keeps_a_warp_on_its_own_texture_set() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mesh = texturecraft_engine::geom::primitives::material_ball();
        let session = texturecraft_engine::Session::new_project("Warp placement", mesh, "builtin:material_ball", None, 128).unwrap();
        let mut harness = Harness::builder()
            .with_size(vec2(1600.0, 960.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        let layer = harness.state_mut().run("layer.add.fill", json!({})).unwrap()["layer"].as_u64().unwrap();
        harness.state_mut().run("layer.fill.projection", json!({"layer": layer, "projection": "warp"})).unwrap();
        harness.run_steps(2);
        click_label(&mut harness, "Place on surface");

        let app = harness.state();
        let rect = app.viewport_rect;
        let frame = gizmo(&harness);
        let on_other_set = |point: Pos2| {
            let local = point - rect.min;
            app.session.texture_set_at(Vec2::new(local.x, local.y), app.session.viewport).is_some_and(|set| set != app.session.ts())
        };
        let movement = vec2(12.0, 0.0);
        let rows = (0..rect.height() as usize).step_by(8);
        let columns = (0..rect.width() as usize).step_by(8);
        let mut candidates = rows.flat_map(|y| columns.clone().map(move |x| rect.min + vec2(x as f32, y as f32)));
        let from = candidates
            .find(|point| frame.grab(*point) == Some(FrameGrab::Place) && (0..=4).all(|step| on_other_set(*point + movement * (step as f32 / 4.0))))
            .expect("a drag entirely on the material ball's other set");
        let layer = frame.layer;
        let before = projection(&harness, layer);
        let steps = app.session.history_labels().len();

        drag(&mut harness, from, from + movement, Modifiers::NONE);

        assert_eq!(projection(&harness, layer), before, "the foreign surface must not take the warp");
        assert_eq!(harness.state().session.history_labels().len(), steps);
    }

    #[test]
    fn world_space_arrows_ignore_the_frames_turn() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");
        let turn = |harness: &mut Harness<'_, TextureCraftApp>| {
            harness.state_mut().run("layer.decal.set", json!({"layer": layer.0, "rotation": 30}));
            harness.run_steps(2);
        };
        let arrow = |harness: &Harness<'_, TextureCraftApp>| {
            let shown = gizmo(harness);
            (shown.arrows[0].unwrap() - shown.center).normalized()
        };
        let frame_arrow = arrow(&harness);
        turn(&mut harness);

        assert!(arrow(&harness).dot(frame_arrow) < 0.95, "an object space arrow turns with the frame");

        harness.state_mut().run("layer.decal.set", json!({"layer": layer.0, "rotation": 0}));
        click_label(&mut harness, "Object space");
        click_label(&mut harness, "World space");
        let world_arrow = arrow(&harness);
        turn(&mut harness);

        assert!(arrow(&harness).dot(world_arrow) > 0.999, "a world space arrow stays on its world axis");
    }

    #[test]
    fn holding_ctrl_moves_in_whole_grid_steps() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");
        let grid_step = harness.state().ui.warp_toolbar.grid_step();
        let start = gizmo(&harness);
        let tip = start.arrows[0].unwrap();
        let along = (tip - start.center).normalized();

        // On macOS Ctrl is independent of the platform Command modifier
        drag(&mut harness, tip, tip + along * 37.0, Modifiers::CTRL);

        let (frame, _) = warp_parts(&harness, layer);
        let steps = frame.offset[0] / grid_step;
        assert!(frame.offset[0].abs() > 1e-3 && (steps - steps.round()).abs() < 1e-3, "moved by whole steps of {grid_step}: {:?}", frame.offset);
    }

    #[test]
    fn keyboard_changes_the_warp_drag_steps() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, _) = fill_app("warp");
        let before = harness.state().ui.warp_toolbar.grid_step();
        click_label(&mut harness, "Manipulator settings");
        harness.get_by_role_and_label(Role::Slider, "Grid steps").focus();
        harness.run_steps(2);

        harness.key_press(egui::Key::ArrowRight);
        harness.run_steps(2);

        assert!(harness.get_by_role_and_label(Role::Slider, "Grid steps").is_focused());
        assert!(harness.state().ui.warp_toolbar.grid_step() > before);
        assert!(harness.query_by_role_and_label(Role::Slider, "Angle steps").is_some());
    }

    #[test]
    fn the_mirror_and_reset_buttons_edit_the_warp_in_one_undo_step_each() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");

        let (fitted, _) = warp_parts(&harness, layer);
        let pulled = [[-0.5, 0.5], [0.9, 0.9], [0.5, -0.5], [-0.5, -0.5]];
        harness.state_mut().run("layer.fill.projection", json!({"layer": layer.0, "warpCorners": pulled}));
        harness.state_mut().run("layer.decal.set", json!({"layer": layer.0, "rotation": 30}));
        harness.run_steps(2);
        let steps = harness.state().session.history_labels().len();

        click_label(&mut harness, "Mirror horizontally");

        assert_eq!(warp_parts(&harness, layer).0.flip, [true, false]);

        click_label(&mut harness, "Mirror vertically");
        click_label(&mut harness, "Mirror horizontally");

        assert_eq!(warp_parts(&harness, layer).0.flip, [false, true], "each button toggles its own direction");
        assert_eq!(harness.state().session.history_labels().len(), steps + 3);

        click_label(&mut harness, "Reset");
        click_label(&mut harness, "Reset warp transform");

        let (frame, corners) = warp_parts(&harness, layer);
        assert_eq!((frame, corners), (fitted, pulled), "the frame goes back and the corners stay");

        click_label(&mut harness, "Reset");
        click_label(&mut harness, "Reset all vertices");

        assert_eq!(warp_parts(&harness, layer).1, Projection::FRAME_CORNERS);
        assert_eq!(harness.state().session.history_labels().len(), steps + 5);
    }
}
