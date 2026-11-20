//! The view-mode combo: section titles, the reference's mesh map and channel names, "Other..." with its image picker and the unlit channel view switch.

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use texturecraft_engine::ViewMode;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

/// The view picker of a view header, which shows `value`
fn picker<'a>(h: &'a Harness<'_, TextureCraftApp>, value: &str) -> egui_kittest::Node<'a> {
    h.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(value)).unwrap_or_else(|| panic!("a view picker showing {value}"))
}

/// Clicks the entry `label` of the open list (a long list scrolls inside the viewport's height)
fn pick(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(12);
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn open(h: &mut Harness<'_, TextureCraftApp>, value: &str) {
    picker(h, value).click();
    h.run_steps(3);
}

/// The image picker's query field, which holds the reference's default query
fn query_field<'a>(h: &'a Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'a> {
    h.get_all_by_role(Role::TextInput).find(|node| node.value().as_deref() == Some("u:image PRs")).expect("the query starts as in the reference")
}

#[test]
fn the_combo_lists_the_reference_sections_and_names() {
    let mut h = harness();
    open(&mut h, "Material");
    for label in [
        "Lighting",
        "Material",
        "Single channel",
        "Base color",
        "Base metalness",
        "Specular roughness",
        "Normal",
        "Height",
        "Normal+height+mesh",
        "Mesh maps",
        "World space normal",
        "ID",
        "Ambient occlusion",
        "Curvature",
        "Position",
        "Thickness",
        "Bent normals",
        "Opacity",
        "Other...",
    ] {
        assert!(h.query_all_by_label(label).next().is_some(), "{label}");
    }
    for old in ["Metallic", "Roughness", "Mesh map: Normal", "Mesh map: World Space Normal", "Ambient Occlusion", "Bent Normals"] {
        assert!(h.query_by_label(old).is_none(), "{old} is not offered");
    }
    assert!(h.query_all_by_label("Normal").count() >= 2, "Normal is a channel and a mesh map");
}

#[test]
fn the_section_titles_cannot_be_picked() {
    let mut h = harness();
    open(&mut h, "Material");
    pick(&mut h, "Single channel");
    assert_eq!(h.state().session.view, ViewMode::Material, "a title is no mode");
}

#[test]
fn a_channel_shows_under_its_reference_name_in_this_combo_only() {
    let mut h = harness();
    open(&mut h, "Material");
    pick(&mut h, "Specular roughness");
    assert_eq!(h.state().session.view, ViewMode::Channel(Channel::Roughness));
    picker(&h, "Specular roughness");
    assert_eq!(Channel::Roughness.label(), "Roughness", "other places keep the channel's name");
}

#[test]
fn a_mesh_map_shows_without_a_prefix() {
    let mut h = harness();
    open(&mut h, "Material");
    pick(&mut h, "Ambient occlusion");
    assert_eq!(h.state().session.view, ViewMode::MeshMap(texturecraft_engine::core::BakedMap::AmbientOcclusion));
    picker(&h, "Ambient occlusion");
}

fn png() -> Vec<u8> {
    let mut png = Vec::new();
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 40, 40, 255]));
    image::DynamicImage::ImageRgba8(image).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

#[test]
fn other_opens_an_image_picker_and_shows_the_chosen_image() {
    let mut h = harness();
    let name = h.state_mut().session.import_resource_bytes("stone.png", png()).unwrap();
    h.run_steps(3);
    open(&mut h, "Material");
    pick(&mut h, "Other...");

    query_field(&h);
    h.get_by_label("reset query to default");
    h.get_by_label(&name).click();
    h.run_steps(3);

    assert_eq!(h.state().session.view, ViewMode::Image);
    assert_eq!(h.state().session.view_image.as_deref(), Some(name.as_str()));
    picker(&h, &name);
}

#[test]
fn the_image_picker_filters_by_name_and_the_reset_button_restores_the_query() {
    let mut h = harness();
    let first = h.state_mut().session.import_resource_bytes("stone.png", png()).unwrap();
    let second = h.state_mut().session.import_resource_bytes("brick.png", png()).unwrap();
    h.run_steps(3);
    open(&mut h, "Material");
    pick(&mut h, "Other...");
    assert!(h.query_by_label(&first).is_some() && h.query_by_label(&second).is_some(), "the default query lists every image");

    let field = query_field(&h);
    field.click();
    field.type_text(" stone");
    h.run_steps(3);
    assert!(h.query_by_label(&second).is_none(), "{second} does not match");
    assert!(h.query_by_label(&first).is_some());

    h.get_by_label("reset query to default").click();
    h.run_steps(3);
    query_field(&h);
    assert!(h.query_by_label(&second).is_some(), "the reset lists every image again");
}

#[test]
fn display_settings_offers_the_unlit_switch_on_by_default() {
    let mut h = harness();
    h.state_mut().ui.dock.focus(texturecraft_ui_egui::docking::Panel::Display);
    h.run_steps(4);
    let label = "Display solo view without lighting (unlit)";
    assert!(h.state().session.doc.project.display.solo_unlit);
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    h.run_steps(3);
    assert!(!h.state().session.doc.project.display.solo_unlit, "the switch turns the lighting on for channel views");
}

#[test]
fn the_image_picker_starts_from_the_default_query_each_time_it_opens() {
    let mut h = harness();
    let name = h.state_mut().session.import_resource_bytes("stone.png", png()).unwrap();
    h.run_steps(3);
    open(&mut h, "Material");
    pick(&mut h, "Other...");
    let field = query_field(&h);
    field.click();
    field.type_text(" stone");
    h.run_steps(3);
    pick(&mut h, &name);
    assert_eq!(h.state().session.view, ViewMode::Image);

    open(&mut h, &name);
    pick(&mut h, "Other...");
    query_field(&h);
}

#[test]
fn removing_the_shown_image_brings_the_material_back() {
    let mut h = harness();
    let name = h.state_mut().session.import_resource_bytes("stone.png", png()).unwrap();
    h.state_mut().run("viewport.view", serde_json::json!({"mode": "image", "image": name}));
    h.run_steps(3);
    h.state_mut().run("resource.remove", serde_json::json!({"name": name}));
    h.run_steps(3);
    assert_eq!(h.state().session.view, ViewMode::Material);
    picker(&h, "Material");
}

#[test]
fn bake_mode_keeps_its_own_view_list() {
    let mut h = harness();
    h.state_mut().ui.mode = texturecraft_ui_egui::state::Mode::Baking;
    h.run_steps(4);
    open(&mut h, "Neutral Material");
    h.get_by_label("Mesh map: Ambient Occlusion").scroll_to_me();
    h.run_steps(3);
    assert!(h.query_by_label("Mesh map: Ambient Occlusion").is_some(), "mesh maps keep their prefix in Bake mode");
    assert!(h.query_by_label("Lighting").is_none() && h.query_by_label("Mesh maps").is_none(), "no section titles in Bake mode");
}
