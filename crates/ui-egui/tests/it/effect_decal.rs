//! A decal held by a mask fill effect has the same editor as a layer's decal: sliders and the placing click

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{Decal, EffectKind, LayerId, Projection};
use texturecraft_ui_egui::TextureCraftApp;

/// The sample session with a new decal mask fill effect selected, and that effect's id
fn selected_mask_decal() -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let image = {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        session.import_resource_bytes("decal.png", png).unwrap()
    };
    let placed = json!({"image": image, "channel": "mask", "position": [0.0, 1.0, 0.0], "normal": [0.0, 1.0, 0.0], "up": [0.0, 0.0, 1.0], "size": 0.5});
    let effect = LayerId(texturecraft_engine::execute(&mut session, "layer.add.decal", &placed).unwrap()["maskEffect"].as_u64().unwrap());
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|creation_context| TextureCraftApp::new(creation_context, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);
    (harness, effect)
}

fn decal(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> Decal {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Fill { projection: Projection::Decal(decal), .. } => *decal,
        _ => panic!("a decal fill effect"),
    }
}

#[test]
fn the_width_slider_resizes_a_mask_effects_decal_in_one_undo_step() {
    let (mut harness, effect) = selected_mask_decal();

    // Free width and height: with Keep proportions (on by default) the height would follow
    harness.get_by_label("Keep proportions").click();
    harness.run_steps(2);
    let before = decal(&harness, effect);
    let steps = harness.state().session.history_labels().len();

    let label_rect = harness.get_by_label("Width").rect();
    let y = label_rect.bottom() + 10.0;
    harness.drag_at(egui::pos2(label_rect.left() + 10.0, y));
    harness.run_steps(2);
    harness.hover_at(egui::pos2(label_rect.left() + 150.0, y));
    harness.run_steps(2);
    harness.drop_at(egui::pos2(label_rect.left() + 150.0, y));
    harness.run_steps(3);

    let after = decal(&harness, effect);
    assert!(after.size[0] > before.size[0], "the dragged width applies: {} -> {}", before.size[0], after.size[0]);
    assert_eq!(after.size[1], before.size[1]);
    assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step for the drag");
    harness.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(decal(&harness, effect), before);
}

#[test]
fn place_on_the_surface_stays_armed_for_a_mask_effects_decal() {
    let (mut harness, effect) = selected_mask_decal();
    harness.get_by_label_contains("Place on the surface").click();
    harness.run_steps(3);
    assert_eq!(harness.state().placing_decal, Some(effect), "the click arms placing for the effect, not its layer");
}
