//! Display Settings > Environment: the environment picker with its drop slot, the slider ranges, the plain background colour in Settings, and the section greyed in mesh-map views.

use egui::{Event, Modifiers, PointerButton, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;
use texturecraft_engine::core::project::DisplaySettings;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::shelf::{Asset, ShelfDrag};
use texturecraft_ui_egui::{TextureCraftApp, menus};

const SETTLE: usize = 3;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h.state_mut().ui.dock.focus(Panel::Display);
    h.run_steps(4);
    h
}

fn display(h: &Harness<'_, TextureCraftApp>) -> DisplaySettings {
    h.state().session.doc.project.display.clone()
}

/// Drags the slider labelled `label` from 60 pixels in to `x` pixels from its left end (past its right end: to the end, left of its left end: to the start)
fn drag(h: &mut Harness<'_, TextureCraftApp>, label: &str, x: f32) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(SETTLE);
    let l = h.get_by_label(label).rect();
    let y = l.bottom() + 10.0;
    h.drag_at(egui::pos2(l.left() + 60.0, y));
    h.run_steps(2);
    h.hover_at(egui::pos2(l.left() + x, y));
    h.run_steps(2);
    h.drop_at(egui::pos2(l.left() + x, y));
    h.run_steps(SETTLE);
}

/// The value fields of the page, as shown
fn values(h: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    h.get_all_by_role(Role::TextInput).filter_map(|node| node.value()).collect()
}

#[test]
fn the_sliders_run_over_the_reference_ranges_and_defaults() {
    let mut h = app();
    let shown = values(&h);
    for expected in ["0", "0.00", "35", "0"] {
        assert!(shown.iter().any(|value| value == expected), "{expected} is shown: {shown:?}");
    }
    assert_eq!(display(&h).env_blur, 0.35, "the blur default is 35 of 100");

    drag(&mut h, "Environment Opacity", 3000.0);
    assert_eq!(display(&h).env_opacity, 1.0, "opacity ends at 100");
    assert!(values(&h).iter().any(|value| value == "100"));

    drag(&mut h, "Environment Blur", 3000.0);
    assert_eq!(display(&h).env_blur, 1.0, "blur ends at 100");

    drag(&mut h, "Environment Exposure (EV)", 3000.0);
    assert_eq!(display(&h).env_exposure, 10.0, "exposure reaches +10");

    let mut h = app();
    drag(&mut h, "Environment Exposure (EV)", -300.0);
    assert!(display(&h).env_exposure < -9.0, "exposure reaches -10: {}", display(&h).env_exposure);
}

#[test]
fn the_slot_shows_the_environment_name_and_the_picker_lists_thumbnails_under_a_default_query() {
    let mut h = app();
    h.get_by_role_and_label(Role::Button, "Studio Soft").click();
    h.run_steps(SETTLE);

    assert!(h.get_all_by_role(Role::TextInput).any(|node| node.value().as_deref() == Some("environment ")), "the query starts as in the reference");
    h.get_by_label("reset query to default");
    for name in texturecraft_render::ibl::ENVIRONMENTS {
        assert!(h.query_all_by_label(name).next().is_some(), "{name} is offered");
    }
    h.get_by_role_and_label(Role::Button, "Night").click();
    h.run_steps(SETTLE);
    assert_eq!(display(&h).environment, "Night");
    h.get_by_role_and_label(Role::Button, "Night");
}

#[test]
fn the_picker_narrows_by_name_and_resets_its_query() {
    let mut h = app();
    h.get_by_role_and_label(Role::Button, "Studio Soft").click();
    h.run_steps(SETTLE);
    let field = h.get_all_by_role(Role::TextInput).find(|node| node.value().as_deref() == Some("environment ")).unwrap();
    field.click();
    field.type_text("sun");
    h.run_steps(SETTLE);
    assert!(h.query_by_role_and_label(Role::Button, "Sunset").is_some());
    assert!(h.query_by_role_and_label(Role::Button, "Night").is_none(), "Night does not match");

    h.get_by_label("reset query to default").click();
    h.run_steps(SETTLE);
    assert!(h.query_by_role_and_label(Role::Button, "Night").is_some(), "the reset lists every environment again");
}

#[test]
fn an_image_dropped_on_the_slot_becomes_the_environment() {
    let mut h = app();
    let mut hdr = Vec::new();
    let img = image::Rgb32FImage::from_fn(8, 4, |_, y| image::Rgb([if y < 2 { 10.0 } else { 0.5 }; 3]));
    image::DynamicImage::ImageRgb32F(img).write_to(&mut std::io::Cursor::new(&mut hdr), image::ImageFormat::Hdr).unwrap();
    let name = h.state_mut().session.import_resource_bytes("sky.hdr", hdr).unwrap();
    h.run_steps(SETTLE);

    let slot = h.get_by_role_and_label(Role::Button, "Studio Soft").rect().center();
    egui::DragAndDrop::set_payload(&h.ctx, ShelfDrag { asset: Asset::Texture(name.clone()), label: "Dragged".into() });
    h.hover_at(slot);
    h.run_steps(2);
    h.event(Event::PointerButton { pos: slot, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(SETTLE);

    assert_eq!(display(&h).environment, format!("resource:{name}"));
    h.get_by_role_and_label(Role::Button, &name);
}

#[test]
fn the_environment_section_is_greyed_while_a_mesh_map_shows() {
    let mut h = app();
    let label = "Environment Opacity";
    h.get_by_label(label).scroll_to_me();
    h.run_steps(SETTLE);
    assert!(!h.get_by_role_and_label(Role::Button, "Studio Soft").accesskit_node().is_disabled());

    h.state_mut().run("viewport.view", json!({"mode": "map:ambientOcclusion"}));
    h.run_steps(SETTLE);
    assert!(h.get_by_role_and_label(Role::Button, "Studio Soft").accesskit_node().is_disabled(), "the slot is greyed in a mesh-map mode");
    assert!(h.get_by_role_and_label(Role::CheckBox, "Cast shadows").accesskit_node().is_disabled(), "so is the rest of the section");

    h.state_mut().run("viewport.view", json!({"mode": "material"}));
    h.run_steps(SETTLE);
    assert!(!h.get_by_role_and_label(Role::Button, "Studio Soft").accesskit_node().is_disabled());
}

#[test]
fn the_tabs_end_with_render_and_visualization_settings() {
    let h = app();
    for label in ["Environment settings", "Camera settings", "Post effects", "Render settings", "Visualization settings"] {
        assert!(h.query_all_by_label(label).next().is_some(), "the tab {label}");
    }
    assert!(h.query_all_by_label("Viewer settings").next().is_none(), "the fourth tab is no longer Viewer settings");
}

#[test]
fn settings_general_has_a_working_background_color_with_two_eyedroppers() {
    let mut h = app();
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);

    for side in ["Top background", "Bottom background"] {
        let label = format!("Pick the {side} Color from the Screen");
        let button = h.get_by_role_and_label(Role::Button, &label);
        assert!(!button.accesskit_node().is_disabled(), "the {side} eyedropper works");
        button.hover();
        h.run_steps(30);
        assert_eq!(h.query_all_by_label(&label).count(), 1, "the {side} eyedropper shows no tooltip, as in the reference");
    }
    let [top, bottom] = h.state().ui.prefs.background;
    let srgb = |linear: [f32; 3]| linear.map(|v| (texturecraft_engine::texel::linear_to_srgb(v) * 255.0 + 0.5) as u8);
    assert_eq!((srgb(top), srgb(bottom)), ([0x24, 0x24, 0x25], [0x24, 0x24, 0x25]), "the default is #242425");

    // A colour set in the window applies with OK
    h.state_mut().ui.windows.settings_draft.as_mut().unwrap().prefs.background[0] = [0.5, 0.2, 0.1];
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.prefs.background[0], [0.5, 0.2, 0.1]);
    assert_eq!(h.state().ui.prefs.background[1], bottom, "the bottom color is its own");
}

/// The dropdown that shows `value`
fn dropdown<'a>(h: &'a Harness<'_, TextureCraftApp>, value: &str) -> egui_kittest::Node<'a> {
    h.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(value)).unwrap_or_else(|| panic!("a dropdown showing {value}"))
}

#[test]
fn environment_alignment_offers_world_and_camera_and_applies() {
    let mut h = app();
    assert!(!dropdown(&h, "World").accesskit_node().is_disabled(), "the alignment works now");
    dropdown(&h, "World").scroll_to_me();
    h.run_steps(SETTLE);
    dropdown(&h, "World").click();
    h.run_steps(SETTLE);
    assert!(h.query_all_by_label("World").next().is_some() && h.query_all_by_label("Camera").next().is_some(), "both alignments are offered");
    h.get_by_label("Camera").click();
    h.run_steps(SETTLE);
    assert_eq!(display(&h).env_alignment, texturecraft_engine::core::project::EnvAlignment::Camera);
    dropdown(&h, "Camera");
}

#[test]
fn the_shadow_quality_rows_work_only_while_shadows_are_on() {
    use texturecraft_engine::core::project::ShadowMode;

    let mut h = app();
    assert!(!display(&h).shadows);
    assert!(dropdown(&h, "Lightweight").accesskit_node().is_disabled(), "Computation mode waits for Shadows");
    assert!(values(&h).iter().any(|value| value == "100"), "Shadows opacity starts at 100");
    drag(&mut h, "Shadows opacity", 20.0);
    assert_eq!(display(&h).shadow_opacity, 1.0, "the opacity does nothing while Shadows is off");

    h.get_by_role_and_label(Role::CheckBox, "Cast shadows").click();
    h.run_steps(SETTLE);
    assert!(display(&h).shadows);
    assert!(!dropdown(&h, "Lightweight").accesskit_node().is_disabled());

    dropdown(&h, "Lightweight").click();
    h.run_steps(SETTLE);
    for mode in ["Intensive", "Average", "Lightweight"] {
        assert!(h.query_by_label(mode).is_some(), "{mode} is offered");
    }
    h.get_by_label("Intensive").click();
    h.run_steps(SETTLE);
    assert_eq!(display(&h).shadow_mode, ShadowMode::Intensive);

    drag(&mut h, "Shadows opacity", 20.0);
    let opacity = display(&h).shadow_opacity;
    assert!(opacity < 0.9, "the slider sets the opacity: {opacity}");
}

#[test]
fn the_environment_picker_starts_from_the_default_query_each_time_it_opens() {
    let mut h = app();
    h.get_by_role_and_label(Role::Button, "Studio Soft").click();
    h.run_steps(SETTLE);
    let field = h.get_all_by_role(Role::TextInput).find(|node| node.value().as_deref() == Some("environment ")).unwrap();
    field.click();
    field.type_text("sun");
    h.run_steps(SETTLE);
    h.get_by_role_and_label(Role::Button, "Sunset").click();
    h.run_steps(SETTLE);

    h.get_by_role_and_label(Role::Button, "Sunset").click();
    h.run_steps(SETTLE);
    assert!(h.get_all_by_role(Role::TextInput).any(|node| node.value().as_deref() == Some("environment ")), "the query is the reference's default again");
}
