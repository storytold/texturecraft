//! The 2D view of a UV tile project: the tile info button left of the view mode, and the one mesh wireframe switch of Display Settings

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::Session;
use texturecraft_engine::geom::primitives;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

const BUTTON: &str = "uvtile_info_menu_button";
const SETTLE_FRAMES: usize = 3;

fn app_with(session: Session) -> Harness<'static, TextureCraftApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(1.0).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.layout = ViewLayout::Only3D;
    harness.run_steps(4);

    // The 3D view then fills the rect the 2D view fills
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(4);
    harness
}

fn tiled_app() -> Harness<'static, TextureCraftApp> {
    app_with(Session::new_project("Tiles", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap())
}

#[test]
fn the_tile_info_button_exists_only_in_uv_tile_projects_and_toggles_the_numbers() {
    let plain = app_with(texturecraft_engine::sample_session(256, 64).unwrap());
    assert!(plain.query_by_label(BUTTON).is_none(), "no UV tiles, no button");

    let mut harness = tiled_app();
    assert!(harness.state().ui.uv_tile_info, "numbers and label on to start with");
    harness.get_by_label(BUTTON).click();
    harness.run_steps(SETTLE_FRAMES);
    assert!(!harness.state().ui.uv_tile_info, "hidden after the click");
    harness.get_by_label(BUTTON).click();
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.state().ui.uv_tile_info, "shown again");
}

#[test]
fn the_tile_info_button_sits_left_of_the_view_mode_picker_and_has_no_tooltip() {
    let mut harness = tiled_app();
    let button = harness.get_by_label(BUTTON).rect();
    let combos: Vec<egui::Rect> =
        harness.get_all_by_role(Role::ComboBox).map(|node| node.rect()).filter(|rect| (rect.center().y - button.center().y).abs() < 6.0).collect();
    assert_eq!(combos.len(), 2, "the camera and the view mode picker share the row");
    let (camera, picker) =
        (combos.iter().min_by(|a, b| a.left().total_cmp(&b.left())).unwrap(), combos.iter().max_by(|a, b| a.left().total_cmp(&b.left())).unwrap());
    assert!(camera.right() <= button.left() + 0.5, "right of the camera picker: {camera:?} {button:?}");
    assert!(button.right() <= picker.left() + 0.5, "left of the view mode: {button:?} {picker:?}");
    assert!((button.center().y - picker.center().y).abs() < 4.0, "on the picker's row");

    harness.hover_at(button.center());
    harness.run_steps(8);
    assert!(harness.query_by_role(Role::Tooltip).is_none(), "no tooltip");

    // The same wait over a button that has a tooltip shows one, so the check above can fail
    let view = harness.state().viewport_rect;
    let pan =
        texturecraft_ui_egui::nav_buttons::button_rect(view, texturecraft_ui_egui::nav_buttons::NavView::Uv, texturecraft_ui_egui::nav_buttons::NavButton::Pan)
            .unwrap();
    harness.hover_at(pan.center());
    harness.run_steps(8);
    assert!(harness.query_by_role(Role::Tooltip).is_some(), "a tooltip shows for the pan button");
}

/// The app with Display Settings open from the rail
fn display_settings_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    harness
        .get_all_by_role(Role::Button)
        .find(|b| b.accesskit_node().label().is_some_and(|l| l == "Display Settings" || l.starts_with("Display Settings (")))
        .unwrap()
        .click();
    harness.run_steps(SETTLE_FRAMES);
    harness
}

#[test]
fn display_settings_have_one_mesh_wireframe_switch_for_both_views() {
    let mut harness = display_settings_app();
    assert!(
        harness.query_by_label("Show wireframe in the 3D view").is_none() && harness.query_by_label("Show UV wireframe in the 2D view").is_none(),
        "the two checkboxes are gone"
    );

    harness.get_by_label("Show mesh wireframe").scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);
    assert!(!harness.state().ui.mesh_wireframe, "off by default");
    harness.get_by_label("Show mesh wireframe").click();
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.state().ui.mesh_wireframe);
    harness.get_by_label("Show mesh wireframe").click();
    harness.run_steps(SETTLE_FRAMES);
    assert!(!harness.state().ui.mesh_wireframe);
}

#[test]
fn the_wireframe_opacity_slider_starts_at_40_and_the_color_is_greyed() {
    let mut harness = display_settings_app();
    harness.get_by_label("Wireframe opacity").scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);
    assert_eq!(harness.state().ui.wireframe_opacity, 40.0);
    assert!(harness.query_by_label("Wireframe opacity").is_some() && harness.query_by_label("Wireframe color").is_some());

    // A function TextureCraft lacks says so on hover
    let color = harness.get_by_label("Wireframe color").rect();
    harness.hover_at(color.center());
    harness.run_steps(8);
    assert!(harness.query_by_label("Not available yet").is_some(), "the hint of a control without a function");
}
