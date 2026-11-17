//! Dragging a decal over the curved ball in the 3D view keeps its frame valid and its up turning smoothly.

use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;
use texturecraft_engine::core::{Decal, LayerKind, Projection};
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_ui_egui::TextureCraftApp;

fn decal(harness: &Harness<'_, TextureCraftApp>) -> Decal {
    let session = &harness.state().session;
    let layer = session.selected_layer().unwrap();
    let LayerKind::Fill(fill) = &session.doc.project.layer(layer).unwrap().kind else { panic!("a fill layer") };
    let Projection::Decal(decal) = fill.projection else { panic!("a decal") };
    decal
}

/// The decal's up inside the surface it lies on
fn plane_up(decal: &Decal) -> Vec3 {
    let normal = Vec3::from(decal.normal).normalize();
    let up = Vec3::from(decal.up);
    (up - normal * normal.dot(up)).normalize_or_zero()
}

#[test]
fn dragging_a_decal_over_the_ball_keeps_a_valid_frame_and_never_flips_its_up() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    let image = harness.state_mut().session.import_resource_bytes("white.png", white_png()).unwrap();
    let rect = harness.state().viewport_rect;
    let center = rect.center();
    let local = center - rect.min;
    harness.state_mut().run("layer.add.decal", serde_json::json!({"image": image, "x": local.x, "y": local.y, "size": 0.2})).unwrap();
    harness.run_steps(2);
    let start = decal(&harness);
    let history = harness.state().session.history_labels().len();

    harness.hover_at(center);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    let mut last = plane_up(&start);
    assert!(last.length() > 0.5);
    for step in 1..=20 {
        harness.hover_at(center + egui::vec2(8.0 * step as f32, -4.0 * step as f32));
        harness.run_steps(2);
        let now = plane_up(&decal(&harness));
        assert!((now.length() - 1.0).abs() < 1e-3, "step {step}: no frame");
        assert!(now.dot(last) > 0.0, "step {step}: up turned from {last} to {now}");
        last = now;
    }
    harness.event(Event::PointerButton { pos: center + egui::vec2(160.0, -80.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(3);

    let end = decal(&harness);
    assert_ne!(end.normal, start.normal, "the drag moved it onto a surface facing elsewhere");
    assert_eq!(end.size, start.size);
    assert_eq!(end.rotation, start.rotation);
    assert_eq!(harness.state().session.history_labels().len(), history + 1, "one undo step for the drag");
}

fn white_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([255; 4]));
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}
