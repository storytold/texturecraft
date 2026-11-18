//! The Projection tool's stencil: an Assets image dropped on its slot, position fields and a reset in the tool panel,
//! S + Shift + left-drag to turn it in snapped steps, and the 2D view saying that projection paints in the 3D view

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use serde_json::json;
use texturecraft_engine::{StencilTiling, Tool};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::shelf::{Asset, ShelfDrag};
use texturecraft_ui_egui::state::ViewLayout;

/// The sample project with the Projection tool equipped, and an imported image's resource name
fn projection_app() -> (Harness<'static, TextureCraftApp>, String) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("logo.png", png).unwrap();
    session.tool.tool = Tool::Projection;
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    (harness, name)
}

/// The number field beside a uniquely labelled slider
fn number_field<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &str) -> egui_kittest::Node<'a> {
    let row = harness.get_by_label(label).rect().center().y;
    harness.get_all(by().role(Role::TextInput)).find(|node| (node.rect().center().y - row).abs() < 4.0).unwrap()
}

/// Type a complete value in the number field beside a uniquely labelled slider
fn set_number(harness: &mut Harness<'_, TextureCraftApp>, label: &str, value: &str) {
    let length = number_field(harness, label).value().unwrap().len();
    number_field(harness, label).focus();
    harness.run_steps(2);
    harness.key_press(Key::End);
    harness.run_steps(1);
    for _ in 0..length {
        harness.key_press(Key::Backspace);
        harness.run_steps(1);
    }
    number_field(harness, label).type_text(value);
    harness.run_steps(1);
    harness.key_press(Key::Enter);
    harness.run_steps(2);
}

/// A point on the UV square in the 2D-only layout
const FIRST_DRAG: Pos2 = egui::pos2(700.0, 420.0);

fn history(harness: &Harness<'_, TextureCraftApp>) -> usize {
    harness.state().session.history_labels().len()
}

fn shows_text(harness: &Harness<'_, TextureCraftApp>, text: &str) -> bool {
    harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(found) if found.galley.text() == text))
}

/// Drag with the left button from `from` by `by` in a few moves, holding `keys` and `modifiers`
fn drag_holding(harness: &mut Harness<'_, TextureCraftApp>, keys: &[Key], modifiers: Modifiers, from: Pos2, by: egui::Vec2) {
    harness.event(Event::ModifiersChanged(modifiers));
    for key in keys {
        harness.event(Event::Key { key: *key, physical_key: Some(*key), pressed: true, repeat: false, modifiers });
    }
    harness.hover_at(from);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(2);
    for step in 1..=4 {
        harness.hover_at(from + by * (step as f32 / 4.0));
        harness.run_steps(1);
    }
    harness.event(Event::PointerButton { pos: from + by, button: PointerButton::Primary, pressed: false, modifiers });
    for key in keys {
        harness.event(Event::Key { key: *key, physical_key: Some(*key), pressed: false, repeat: false, modifiers });
    }
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(3);
}

#[test]
fn an_assets_image_dropped_on_the_stencil_slot_becomes_the_stencil() {
    let (mut harness, name) = projection_app();
    assert!(harness.state().session.tool.stencil.is_none());

    let slot = harness.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("None")).unwrap().rect().center();
    egui::DragAndDrop::set_payload(&harness.ctx, ShelfDrag { asset: Asset::Texture(name.clone()), label: "Dragged".into() });
    harness.hover_at(slot);
    harness.run_steps(2);
    harness.event(Event::PointerButton { pos: slot, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);

    assert_eq!(harness.state().session.tool.stencil.as_ref().map(|stencil| stencil.image.as_str()), Some(name.as_str()));
}

#[test]
fn the_position_fields_move_the_stencil_and_reset_puts_it_back() {
    let (mut harness, name) = projection_app();
    texturecraft_engine::execute(
        &mut harness.state_mut().session,
        "tool.stencil",
        &json!({"image": name, "center": [0.5, -0.25], "scale": 2.0, "rotation": 90.0, "opacity": 0.3, "tiling": "vertical"}),
    )
    .unwrap();
    harness.run_steps(3);

    // Typing in a position field moves the stencil
    set_number(&mut harness, "Position X", "-0.75");
    let moved = harness.state().session.tool.stencil.clone().unwrap();
    assert_eq!(moved.center, [-0.75, -0.25], "only X changed");
    harness.get_by_label("Position Y");

    harness.get_by_label("Reset stencil").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label("Reset stencil").click();
    harness.run_steps(3);
    let reset = harness.state().session.tool.stencil.clone().unwrap();
    assert_eq!((reset.center, reset.scale, reset.rotation), ([0.0, 0.0], 0.8, 0.0));
    assert_eq!((reset.image.as_str(), reset.opacity, reset.tiling), (name.as_str(), 0.3, StencilTiling::Vertical), "image, opacity and tiling stay");
}

#[test]
fn s_shift_left_drag_turns_the_stencil_in_snapped_steps_without_painting() {
    let (mut harness, name) = projection_app();
    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &json!({"image": name})).unwrap();
    harness.run_steps(2);
    let from = harness.state().viewport_rect.center();
    let before = harness.state().session.tool.stencil.clone().unwrap();
    let steps = history(&harness);

    drag_holding(&mut harness, &[Key::S], Modifiers::SHIFT, from, vec2(100.0, -40.0));

    let turned = harness.state().session.tool.stencil.clone().unwrap();
    assert_ne!(turned.rotation, before.rotation, "it turned");
    assert_eq!(turned.rotation % 15.0, 0.0, "a snapped turn lands on a multiple of 15 degrees: {}", turned.rotation);
    assert_eq!(turned.center, before.center, "a turn does not move it");
    assert_eq!(history(&harness), steps, "nothing painted");

    // Without Shift the left-drag turns it freely, and still does not paint
    drag_holding(&mut harness, &[Key::S], Modifiers::NONE, from, vec2(30.0, 0.0));
    let free = harness.state().session.tool.stencil.clone().unwrap();
    assert_ne!(free.rotation % 15.0, 0.0, "a free turn is not snapped: {}", free.rotation);
    assert_eq!(free.center, turned.center);
    assert_eq!(history(&harness), steps);
}

#[test]
fn the_2d_view_says_projection_paints_in_3d_and_paints_nothing() {
    let (mut harness, name) = projection_app();
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(4);
    let from = FIRST_DRAG;
    let steps = history(&harness);

    // Without a stencil
    assert!(shows_text(&harness, "Projection paints in the 3D view"));
    drag_holding(&mut harness, &[], Modifiers::NONE, from, vec2(60.0, 0.0));
    assert_eq!(history(&harness), steps, "no stencil: nothing painted");

    // With a stencil
    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &json!({"image": name})).unwrap();
    harness.run_steps(2);
    assert!(shows_text(&harness, "Projection paints in the 3D view"));
    drag_holding(&mut harness, &[], Modifiers::NONE, from, vec2(60.0, 0.0));
    assert_eq!(history(&harness), steps, "with a stencil: nothing painted");

    // Other tools paint in the 2D view and show no hint
    harness.state_mut().session.tool.tool = Tool::Paint;
    harness.run_steps(2);
    assert!(!shows_text(&harness, "Projection paints in the 3D view"));
    drag_holding(&mut harness, &[], Modifiers::NONE, from, vec2(60.0, 0.0));
    assert!(history(&harness) > steps, "the paint tool paints there");
}

/// Total vertices of the meshes drawn this frame
fn mesh_vertices(harness: &Harness<'_, TextureCraftApp>) -> usize {
    harness.output().shapes.iter().map(|shape| if let egui::Shape::Mesh(mesh) = &shape.shape { mesh.vertices.len() } else { 0 }).sum()
}

#[test]
fn the_tiling_mode_combo_replaces_the_tile_checkbox_and_the_overlay_repeats_along_it() {
    let (mut harness, name) = projection_app();
    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &json!({"image": name})).unwrap();
    harness.run_steps(3);
    assert!(harness.query_by_label("Tile").is_none(), "the checkbox is gone");

    harness.get_by_label("Tiling mode").scroll_to_me();
    harness.run_steps(3);
    let mut counts = Vec::new();
    let mut current = "No Tiling";
    for (label, mode) in [
        ("Horizontal Tiling", StencilTiling::Horizontal),
        ("Vertical Tiling", StencilTiling::Vertical),
        ("H and V Tiling", StencilTiling::Both),
        ("No Tiling", StencilTiling::None),
    ] {
        harness.get_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(current)).unwrap().click();
        harness.run_steps(2);
        harness.get_by_label(label).click();
        harness.run_steps(4);
        current = label;
        assert_eq!(harness.state().session.tool.stencil.as_ref().unwrap().tiling, mode, "{label}");
        counts.push(mesh_vertices(&harness));
    }
    let [horizontal, vertical, both, none] = counts[..] else { unreachable!() };
    assert!(horizontal > none && vertical > none, "one axis adds copies: {horizontal} {vertical} over {none}");
    assert!(both > horizontal && both > vertical, "both axes add the most: {both}");
}
