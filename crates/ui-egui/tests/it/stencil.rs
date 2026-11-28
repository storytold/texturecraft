//! The brush tools' stencil: an Assets image dropped on its slot, position fields, Tiling mode and the section reset in the tool panel, S + Shift + left-drag to turn it in snapped steps, the 2D view saying that projection paints in the 3D view, and no stencil while the quick mask is painted

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use serde_json::json;
use texturecraft_engine::Tool;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::shelf::{Asset, ShelfDrag};
use texturecraft_ui_egui::state::ViewLayout;

use crate::{BAKE_RESOLUTION, REDRAW_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// The sample project with the Projection tool equipped, and an imported image's resource name
fn projection_app() -> (Harness<'static, TextureCraftApp>, String) {
    tool_app(Tool::Projection)
}

/// Window the stencil tests run in
const WINDOW_SIZE: egui::Vec2 = vec2(1600.0, 960.0);

/// Side of the plain white image imported for the stencil
const IMAGE_SIZE: u32 = 4;

/// The sample project with `tool` equipped and its Stencil section in view, and an imported image's resource name
fn tool_app(tool: Tool) -> (Harness<'static, TextureCraftApp>, String) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut png = Vec::new();
    let white = image::RgbaImage::from_pixel(IMAGE_SIZE, IMAGE_SIZE, image::Rgba([255, 255, 255, 255]));
    white.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("logo.png", png).unwrap();
    session.tool.tool = tool;

    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);

    harness.get_by_role_and_label(Role::Tab, "Stencil").click();
    harness.run_steps(REDRAW_FRAMES);
    (harness, name)
}

/// The number field a click on a slider's value opens, on the slider's line
fn number_field<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &str) -> egui_kittest::Node<'a> {
    let row = harness.get_by_role_and_label(Role::Slider, label).rect().center().y;
    harness.get_all(by().role(Role::TextInput)).find(|node| (node.rect().center().y - row).abs() < 4.0).unwrap()
}

/// Type a complete value in the number field of a uniquely labelled slider
fn set_number(harness: &mut Harness<'_, TextureCraftApp>, label: &str, value: &str) {
    harness.get_by_label(&format!("{label} value")).click();
    harness.run_steps(2);
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

    let slot = harness.get_by_role_and_label(Role::ComboBox, "None").rect().center();
    egui::DragAndDrop::set_payload(&harness.ctx, ShelfDrag { asset: Asset::Texture(name.clone()), label: "Dragged".into(), thumb: None });
    harness.hover_at(slot);
    harness.run_steps(2);
    harness.event(Event::PointerButton { pos: slot, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);

    assert_eq!(harness.state().session.tool.stencil.as_ref().map(|stencil| stencil.image.as_str()), Some(name.as_str()));
}

#[test]
fn the_position_fields_move_the_stencil_and_the_section_reset_puts_it_back() {
    let (mut harness, name) = projection_app();
    texturecraft_engine::execute(
        &mut harness.state_mut().session,
        "tool.stencil",
        &json!({"image": name, "center": [0.5, -0.25], "scale": 2.0, "rotation": 90.0, "opacity": 0.3, "tile": true}),
    )
    .unwrap();
    harness.run_steps(3);

    // Typing in a position field moves the stencil
    set_number(&mut harness, "Position X", "-0.75");
    let moved = harness.state().session.tool.stencil.clone().unwrap();
    assert_eq!(moved.center, [-0.75, -0.25], "only X changed");
    harness.get_by_role_and_label(Role::Slider, "Position Y");

    harness.get_by_label("Stencil: Reset to defaults").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label("Stencil: Reset to defaults").click();
    harness.run_steps(3);
    let reset = harness.state().session.tool.stencil.clone().unwrap();
    assert_eq!((reset.center, reset.scale, reset.rotation), ([0.0, 0.0], 0.8, 0.0));
    assert_eq!((reset.image.as_str(), reset.opacity, reset.tile), (name.as_str(), 0.3, true), "image, opacity and tiling stay");
}

#[test]
fn the_tiling_mode_list_tiles_the_stencil() {
    let (mut harness, name) = tool_app(Tool::Paint);
    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &json!({"image": name})).unwrap();
    harness.run_steps(3);

    harness.get_by_role_and_label(Role::ComboBox, "No Tiling").click();
    harness.run_steps(3);
    harness.get_by_label("H and V Tiling").click();
    harness.run_steps(3);

    assert!(harness.state().session.tool.stencil.as_ref().is_some_and(|stencil| stencil.tile));
}

/// The Paint tool's stencil turns like the Projection tool's
#[test]
fn s_shift_left_drag_turns_the_stencil_in_snapped_steps_without_painting() {
    let (mut harness, name) = tool_app(Tool::Paint);
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
fn the_stencil_hides_and_stays_put_while_the_quick_mask_is_painted() {
    let (mut harness, name) = tool_app(Tool::Paint);
    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &json!({"image": name})).unwrap();
    texturecraft_engine::execute(&mut harness.state_mut().session, "viewport.quickMask", &json!({"enabled": true})).unwrap();
    texturecraft_engine::execute(&mut harness.state_mut().session, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
    harness.run_steps(REDRAW_FRAMES);
    let from = harness.state().viewport_rect.center();
    let before = harness.state().session.tool.stencil.clone().unwrap();

    assert!(harness.query_by_role_and_label(Role::Tab, "Stencil").is_none(), "quick mask strokes skip the stencil, so it has no tab");
    drag_holding(&mut harness, &[Key::S], Modifiers::SHIFT, from, vec2(100.0, -40.0));
    assert_eq!(harness.state().session.tool.stencil.clone().unwrap().rotation, before.rotation, "S + drag leaves it alone");

    texturecraft_engine::execute(&mut harness.state_mut().session, "viewport.quickMask.edit", &json!({"editing": false})).unwrap();
    harness.run_steps(REDRAW_FRAMES);

    assert!(harness.query_by_role_and_label(Role::Tab, "Stencil").is_some(), "back when the quick mask is done");
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

#[test]
fn picking_another_stencil_releases_the_last_ones_overlay_texture() {
    const OVERLAY_SIDE_PX: u32 = 1024;
    let (mut harness, _) = tool_app(Tool::Paint);
    let stencil_textures = |harness: &Harness<'_, TextureCraftApp>| {
        harness
            .ctx
            .tex_manager()
            .read()
            .allocated()
            .filter(|(_, texture)| texture.name.starts_with("stencil:"))
            .fold((0, 0), |(count, bytes), (_, texture)| (count + 1, bytes + texture.size[0] * texture.size[1] * texture.bytes_per_pixel))
    };

    // Switch among three full-size overlays
    for gray in [64, 128, 192] {
        let mut png = Vec::new();
        let image = image::RgbaImage::from_pixel(OVERLAY_SIDE_PX, OVERLAY_SIDE_PX, image::Rgba([gray, gray, gray, 255]));
        image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let name = harness.state_mut().session.import_resource_bytes(&format!("stencil_{gray}.png"), png).unwrap();
        texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &json!({ "image": name })).unwrap();
        harness.run_steps(REDRAW_FRAMES);
    }

    let (count, bytes) = stencil_textures(&harness);
    eprintln!("selected stencil retains {count} texture(s) and {bytes} texture bytes");
    assert_eq!(count, 1, "only the current stencil keeps its big texture");

    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &json!({"clear": true})).unwrap();
    harness.run_steps(REDRAW_FRAMES);

    let (count, bytes) = stencil_textures(&harness);
    eprintln!("cleared stencil retains {count} texture(s) and {bytes} texture bytes");
    assert_eq!(count, 0, "clearing the stencil releases its overlay");
}
