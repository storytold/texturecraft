//! The decal editor's Mirror across and Flip controls set the decal's mirrored copy

use egui_kittest::Harness;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{DecalFlip, LayerKind, Projection};
use texturecraft_ui_egui::TextureCraftApp;

/// Tall enough that Properties shows the whole decal editor without scrolling
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 1400.0);

/// Small texture and bake sizes keep the sample session fast; the panel doesn't depend on them
const TEXTURE_RESOLUTION: u32 = 256;
const BAKE_RESOLUTION: u32 = 64;

/// Frames for the first layout and fonts to settle
const STARTUP_FRAMES: usize = 4;

/// Frames for a click or selection change to reach the panel
const SETTLE_FRAMES: usize = 3;

fn red_png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// The app with a new decal layer selected in Properties, and its layer id
fn app_with_decal() -> (Harness<'static, TextureCraftApp>, u64) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let image = session.import_resource_bytes("logo.png", red_png()).unwrap();
    let placed = json!({"image": image, "position": [0.0, 1.0, 0.0], "normal": [0.0, 1.0, 0.0], "up": [0.0, 0.0, 1.0], "size": 0.3});
    let layer = texturecraft_engine::execute(&mut session, "layer.add.decal", &placed).unwrap()["layer"].as_u64().unwrap();

    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    (harness, layer)
}

fn symmetry_of(harness: &Harness<'_, TextureCraftApp>, layer: u64) -> Option<(u8, DecalFlip)> {
    let project = &harness.state().session.doc.project;
    let LayerKind::Fill(fill) = &project.layer(texturecraft_engine::core::LayerId(layer)).unwrap().kind else { panic!("a fill layer") };
    let Projection::Decal(decal) = &fill.projection else { panic!("a decal") };
    decal.symmetry.map(|symmetry| (symmetry.axis, symmetry.flip))
}

/// Opens the combo showing `current` and picks `choice` in its menu
fn pick(harness: &mut Harness<'_, TextureCraftApp>, current: &str, choice: &str) {
    harness.get_by_role_and_label(Role::ComboBox, current).click();
    harness.run_steps(SETTLE_FRAMES);
    harness.get_all_by_label(choice).last().expect("the menu entry").click();
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn the_mirror_axis_and_flip_are_set_from_the_decal_editor() {
    let (mut harness, layer) = app_with_decal();
    assert!(harness.query_by_label("Mirror across").is_some());
    assert!(harness.query_by_label("Flip the copy's image").is_none(), "the flip needs a copy to flip");
    assert_eq!(symmetry_of(&harness, layer), None);

    pick(&mut harness, "Off", "X");
    assert_eq!(symmetry_of(&harness, layer), Some((0, DecalFlip::None)));
    assert!(harness.query_by_label("Flip the copy's image").is_some());

    pick(&mut harness, "None", "V");
    assert_eq!(symmetry_of(&harness, layer), Some((0, DecalFlip::V)));

    pick(&mut harness, "X", "Z");
    assert_eq!(symmetry_of(&harness, layer), Some((2, DecalFlip::V)), "a new axis keeps the flip");

    pick(&mut harness, "Z", "Off");
    assert_eq!(symmetry_of(&harness, layer), None);
    assert!(harness.query_by_label("Flip the copy's image").is_none());
}
