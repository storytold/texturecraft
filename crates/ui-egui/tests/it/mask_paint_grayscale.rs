//! A folder's mask selected: the paint tool's third tab is Grayscale, a slot card for the gray a stroke paints (its ramp and invert) or an image picked from the slot's picker

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::PropsTab;

use crate::{BAKE_RESOLUTION, DRAG_STEP_FRAMES, FRAME_SECONDS, PROPERTIES_WINDOW, SETTLE_FRAMES, SLIDER_END_INSET, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// An 8 × 8 gray picture
fn gray_png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([90, 90, 90, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

/// A folder with a white mask, the mask selected and the tool's Grayscale tab open: (harness, the imported image's name)
fn folder_mask_harness() -> (Harness<'static, TextureCraftApp>, String) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let image = session.import_resource_bytes("scratches.png", gray_png()).unwrap();
    let folder = LayerId(execute(&mut session, "layer.add.group", &json!({})).unwrap()["layer"].as_u64().unwrap());
    execute(&mut session, "mask.add.white", &json!({"layer": folder.0})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": folder.0, "target": "mask"})).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).with_step_dt(FRAME_SECONDS).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().ui.props_tab = PropsTab::Material;
    harness.run_steps(STARTUP_FRAMES);
    (harness, image)
}

fn click(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).click();
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn the_grayscale_slot_paints_one_gray_its_ramp_drags_and_inverts() {
    let (mut harness, _) = folder_mask_harness();
    assert!(harness.query_by_label("Mask value (gray)").is_none(), "the old slider is gone");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Grayscale").is_some(), "the Grayscale slot card");

    click(&mut harness, Role::Button, "Invert");

    assert_eq!(harness.state().session.tool.mask_value, 0.0, "white inverts to black");

    // A real drag along the ramp to its right end
    let track = harness.get_by_role_and_label(Role::Slider, "Grayscale").rect();
    let at = |t: f32| egui::pos2(track.left() + SLIDER_END_INSET + (track.width() - 2.0 * SLIDER_END_INSET) * t, track.center().y);
    harness.drag_at(at(0.1));
    harness.run_steps(DRAG_STEP_FRAMES);
    for t in [0.6, 1.0] {
        harness.hover_at(at(t));
        harness.run_steps(DRAG_STEP_FRAMES);
    }
    harness.drop_at(at(1.0));
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().session.tool.mask_value > 0.95, "dragged to {}", harness.state().session.tool.mask_value);
}

#[test]
fn the_slot_picks_an_image_to_paint_and_clear_goes_back_to_the_gray() {
    let (mut harness, image) = folder_mask_harness();
    harness.state_mut().run("tool.material", json!({"maskValue": 0.25}));
    click(&mut harness, Role::ComboBox, "Grayscale");

    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Import...").is_some(), "Textures starts with Import...");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Import...").is_none(), "Escape closes the picker");
    assert_eq!(harness.state().session.tool.mask_image, None);

    click(&mut harness, Role::ComboBox, "Grayscale");
    click(&mut harness, Role::ListBoxOption, &image);

    assert_eq!(harness.state().session.tool.mask_image.as_deref(), Some(image.as_str()));
    assert!(harness.query_by_role_and_label(Role::Slider, "Grayscale").is_none(), "an image has no ramp");

    click(&mut harness, Role::Button, "Clear");

    assert_eq!(harness.state().session.tool.mask_image, None);
    assert!(harness.query_by_role_and_label(Role::Slider, "Grayscale").is_some(), "the ramp is back");
    assert_eq!(harness.state().session.tool.mask_value, 0.25);
}
