//! The Reference panel: it draws with images, a missing file and a pinned overlay, and a click on
//! the image picks that colour into the brush.
//!
//! `TC_UI_SNAPSHOTS=<dir> cargo test -p texturecraft-ui-egui --release --test reference_panel -- --ignored`
//! writes a picture of the window with the panel and the overlay (needs a GPU adapter).

use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

/// An original picture: left half red, right half a mid grey, a white dot in the grey half.
fn picture() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(64, 32, |x, y| {
        if (x as i32 - 48).pow(2) + (y as i32 - 16).pow(2) < 25 {
            image::Rgba([255, 255, 255, 255])
        } else if x < 32 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([128, 128, 128, 255])
        }
    });
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn click(h: &mut Harness<'_, TextureCraftApp>, pos: egui::Pos2) {
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
}

fn app_with_reference() -> (Harness<'static, TextureCraftApp>, u64) {
    let mut session = Session::new_project("t", texturecraft_engine::geom::primitives::builtin("sphere").unwrap(), "builtin:sphere", None, 256).unwrap();
    let id = session.add_reference_bytes("picture.png", picture()).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.dock.focus(Panel::Reference);
    h.run_steps(5);
    (h, id)
}

#[test]
fn a_click_on_the_image_picks_its_colour_for_the_brush() {
    let (mut h, id) = app_with_reference();
    let rect = texturecraft_ui_egui::reference::panel_rect(&h.ctx).expect("the panel was drawn");
    let before = h.state().session.tool.values[&texturecraft_engine::core::Channel::BaseColor];
    click(&mut h, rect.center() - egui::vec2(40.0, 0.0));
    h.run_steps(3);
    let picked = h.state().session.tool.values[&texturecraft_engine::core::Channel::BaseColor];
    assert_eq!(picked, [1.0, 0.0, 0.0], "the left half is red (it was {before:?})");
    // Flipped, the same spot shows the grey half.
    execute(&mut h.state_mut().session, "reference.set", &json!({"id": id, "flipH": true})).unwrap();
    h.run_steps(3);
    click(&mut h, rect.center() - egui::vec2(40.0, 0.0));
    h.run_steps(3);
    let grey = h.state().session.tool.values[&texturecraft_engine::core::Channel::BaseColor][0];
    assert!((grey - 0.2158).abs() < 1e-3, "{grey}");
}

#[test]
fn the_panel_and_overlay_draw_with_every_kind_of_reference() {
    let (mut h, id) = app_with_reference();
    let s = &mut h.state_mut().session;
    execute(s, "reference.overlay", &json!({"reference": id, "opacity": 0.6, "x": 0.3, "y": 0.7, "scale": 0.4})).unwrap();
    // A linked file that is not there.
    s.doc.project.references.push(texturecraft_engine::core::Reference {
        id: 9001,
        name: "gone.png".into(),
        source: texturecraft_engine::core::ReferenceSource::Linked { path: "gone.png".into() },
        flip_h: false,
        flip_v: false,
        grayscale: false,
    });
    s.sync_references();
    h.run_steps(4);
    h.run_steps(2);
    assert!(h.state().session.reference_error(9001).is_some());
    // With the good image gone the missing one is shown: its error, not a panic.
    execute(&mut h.state_mut().session, "reference.remove", &json!({"id": id})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().session.doc.project.references.len(), 1);
}

#[test]
#[ignore]
fn reference_snapshot() {
    let Some(dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let mut session = texturecraft_engine::sample_session(512, 64).unwrap();
    let id = session.add_reference_bytes("picture.png", picture()).unwrap();
    execute(&mut session, "reference.overlay", &json!({"reference": id, "opacity": 0.5, "x": 0.5, "y": 0.5, "scale": 0.35})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.dock.focus(Panel::Reference);
    h.run_steps(8);
    h.render().unwrap().save(dir.join("reference-panel.png")).unwrap();
}
