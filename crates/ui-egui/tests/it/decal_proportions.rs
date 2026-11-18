//! A decal's Width and Height keep the image's proportions, and Flip U / Flip V mirror its image

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use serde_json::json;
use texturecraft_engine::core::{Decal, LayerId, LayerKind, Projection};
use texturecraft_ui_egui::TextureCraftApp;

/// Frames for a click or a typed value to reach the document and the panel
const SETTLE_FRAMES: usize = 3;

/// The longest text a side's number field shows, cleared before typing a value
const FIELD_TEXT_LIMIT: usize = 12;

/// The sample app with a decal of a twice-as-wide-as-tall image selected, and its layer
fn app_with_decal() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("wide.png", png).unwrap();
    let placed = texturecraft_engine::execute(
        &mut session,
        "layer.add.decal",
        &json!({"image": name, "position": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0], "size": [0.8, 0.4]}),
    )
    .unwrap();
    let layer = LayerId(placed["layer"].as_u64().unwrap());
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    (harness, layer)
}

fn decal(harness: &Harness<'_, TextureCraftApp>, layer: LayerId) -> Decal {
    let LayerKind::Fill(fill) = &harness.state().session.doc.project.layer(layer).unwrap().kind else { panic!("a fill layer") };
    let Projection::Decal(decal) = &fill.projection else { panic!("a decal") };
    *decal
}

/// Type `value` into the number field of the row labelled `label` and confirm it
fn type_side(harness: &mut Harness<'_, TextureCraftApp>, label: &str, value: &str) {
    let field = |harness: &Harness<'_, TextureCraftApp>| {
        let row = harness.get_by_label(label).rect().center().y;
        harness.get_all(by().role(Role::TextInput)).find(|node| (node.rect().center().y - row).abs() < 4.0).map(|node| node.rect().center()).unwrap()
    };
    harness.drag_at(field(harness));
    harness.drop_at(field(harness));
    harness.run_steps(2);
    harness.key_press(egui::Key::End);
    for _ in 0..FIELD_TEXT_LIMIT {
        harness.key_press(egui::Key::Backspace);
        harness.run_steps(1);
    }
    for character in value.chars() {
        harness.get_all(by().role(Role::TextInput)).find(|node| node.is_focused()).unwrap().type_text(&character.to_string());
        harness.run_steps(1);
    }
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn width_and_height_follow_each_other_by_the_image_aspect() {
    let (mut harness, layer) = app_with_decal();

    type_side(&mut harness, "Width", "0.6");
    assert_eq!(decal(&harness, layer).size, [0.6, 0.3], "the width sets the height");

    type_side(&mut harness, "Height", "0.1");
    assert_eq!(decal(&harness, layer).size, [0.2, 0.1], "the height sets the width");
}

#[test]
fn unticking_keep_proportions_frees_each_side() {
    let (mut harness, layer) = app_with_decal();
    harness.get_by_label("Keep proportions").click();
    harness.run_steps(SETTLE_FRAMES);

    type_side(&mut harness, "Width", "0.6");
    assert_eq!(decal(&harness, layer).size, [0.6, 0.4], "only the width changes");

    // Ticking it again fits the height to the width, in one undo step
    let steps = harness.state().session.history_labels().len();
    harness.get_by_label("Keep proportions").click();
    harness.run_steps(SETTLE_FRAMES);
    assert_eq!(decal(&harness, layer).size, [0.6, 0.3]);
    assert_eq!(harness.state().session.history_labels().len(), steps + 1);
}

#[test]
fn flip_toggles_mirror_the_image_and_undo_one_step_at_a_time() {
    let (mut harness, layer) = app_with_decal();
    assert_eq!(decal(&harness, layer).flip, [false, false]);

    harness.get_by_label("Flip U").click();
    harness.run_steps(SETTLE_FRAMES);
    assert_eq!(decal(&harness, layer).flip, [true, false]);

    harness.get_by_label("Flip V").click();
    harness.run_steps(SETTLE_FRAMES);
    assert_eq!(decal(&harness, layer).flip, [true, true]);

    harness.get_by_label("Flip U").click();
    harness.run_steps(SETTLE_FRAMES);
    assert_eq!(decal(&harness, layer).flip, [false, true], "Flip U toggles back and leaves Flip V");

    texturecraft_engine::execute(&mut harness.state_mut().session, "edit.undo", &json!({})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    assert_eq!(decal(&harness, layer).flip, [true, true]);
}
