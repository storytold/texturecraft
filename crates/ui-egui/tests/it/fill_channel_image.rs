//! A fill channel takes an image resource from its source picker

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{Channel, LayerId, LayerKind, Source};
use texturecraft_ui_egui::TextureCraftApp;

/// The Roughness channel's source slot in Properties
fn roughness<'a>(harness: &'a Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(egui::Role::ComboBox, "Roughness")
}

/// Opens the source picker of the Roughness channel from its slot
fn open_roughness_source_picker(harness: &mut Harness<'_, TextureCraftApp>) {
    // Allow the scroll animation to finish before the click
    roughness(harness).scroll_to_me();
    harness.run_steps(30);
    roughness(harness).click();
    harness.run_steps(3);
}

#[test]
fn a_fill_channel_takes_an_image_from_its_source_picker() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png_bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 90, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("rough.png", png_bytes).unwrap();
    let result = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"roughness": 0.6})).unwrap();
    let layer = LayerId(result["layer"].as_u64().unwrap());
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // Use English so first-run locale does not change queried labels
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);

    open_roughness_source_picker(&mut harness);
    harness.get_by_role_and_label(egui::Role::Button, &name).click();
    harness.run_steps(3);

    let LayerKind::Fill(fill) = &harness.state().session.doc.project.layers[&layer].kind else { panic!("a fill layer") };
    assert_eq!(fill.source(Channel::Roughness), Source::image(name));
}

/// A resource that did not decode is not offered, since the commands that take an image refuse it.
#[test]
fn the_source_picker_offers_only_images_that_were_decoded() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png_bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 90, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("rough.png", png_bytes).unwrap();

    // Bytes that are not an image never reach the decoded images
    session.doc.resources.insert("broken.png".into(), std::sync::Arc::new(vec![1, 2, 3]));
    texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"roughness": 0.6})).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // Use English so first-run locale does not change queried labels
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);

    open_roughness_source_picker(&mut harness);

    assert!(harness.query_by_label(&name).is_some(), "a decoded image is offered");
    assert!(harness.query_by_label("broken.png").is_none(), "an image that did not decode is not");
}

#[test]
fn switching_projects_releases_image_previews_with_equal_revisions() {
    use std::sync::Arc;

    use texturecraft_engine::{Session, geom::primitives};
    use texturecraft_ui_egui::thumbs::Thumbs;

    let new_session = || Session::new_project("Image previews", primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    let context = egui::Context::default();
    let mut session = new_session();
    let mut thumbnails = Thumbs::default();
    let mut png_bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 90, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).unwrap();
    let preview_count = || context.tex_manager().read().allocated().filter(|(_, metadata)| metadata.name.starts_with("res_reload-")).count();
    let mut counts = Vec::new();
    let mut decoded_images = Vec::new();

    for index in 0..8 {
        let mut saved = new_session();
        let name = saved.import_resource_bytes(&format!("reload-{index}.png"), png_bytes.clone()).unwrap();
        let loaded = Session::from_project_bytes(&saved.to_project_bytes().unwrap()).unwrap();
        drop(saved);

        // Freshly loaded projects can have equal revisions despite different resources
        assert_eq!(loaded.revision, 1);
        session.switch_to(loaded);
        thumbnails.sync_images(&session.images, session.revision);
        let image = &session.images[&name];
        decoded_images.push(Arc::downgrade(image));
        drop(thumbnails.image(&context, &name, image));
        counts.push(preview_count());
    }

    session.switch_to(new_session());
    thumbnails.sync_images(&session.images, session.revision);
    let after_close = preview_count();
    drop(thumbnails);

    eprintln!("preview counts across eight loaded projects: {counts:?}; after close: {after_close}; after cache drop: {}", preview_count());
    assert!(decoded_images.iter().all(|image| image.upgrade().is_none()), "old decoded images are released");
    assert_eq!(preview_count(), 0, "cache shutdown releases its texture handles");
    assert_eq!(counts, vec![1; 8], "only the current project's preview should remain cached");
    assert_eq!(after_close, 0, "closing the project releases the last preview");
}

#[test]
fn a_mask_fill_selects_an_image_edits_projection_and_returns_to_uniform() {
    use texturecraft_engine::core::{EffectKind, Projection};

    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png_bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 90, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).unwrap();
    let image_name = session.import_resource_bytes("mask.png", png_bytes).unwrap();
    texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"roughness": 0.6})).unwrap();
    texturecraft_engine::execute(&mut session, "mask.add.black", &json!({})).unwrap();
    let output = texturecraft_engine::execute(&mut session, "effect.add.fill", &json!({"target": "mask"})).unwrap();
    let effect_id = LayerId(output["effect"].as_u64().unwrap());
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(4);

    // The mask fill's slot opens the source picker
    harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").scroll_to_me();
    harness.run_steps(30);
    harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::Role::Button, &image_name).click();
    harness.run_steps(3);

    let effect = &harness.state().session.doc.project.effect(effect_id).unwrap().1.kind;
    assert!(matches!(effect, EffectKind::Fill { source: Source::Image { name, .. }, .. } if name == &image_name));

    harness.get_all_by_role(egui::Role::ComboBox).find(|node| node.value().as_deref() == Some("UV projection")).unwrap().click();
    harness.run_steps(2);
    harness.get_by_label("Tri-planar projection").click();
    harness.run_steps(3);

    let effect = &harness.state().session.doc.project.effect(effect_id).unwrap().1.kind;
    assert!(matches!(effect, EffectKind::Fill { projection: Projection::Triplanar { .. }, .. }));

    assert_eq!(harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").value().as_deref(), Some(image_name.as_str()), "the slot names the image");
    harness.get_by_role_and_label(egui::Role::ComboBox, "Grayscale").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::Role::Button, "Uniform value").click();
    harness.run_steps(3);

    let effect = &harness.state().session.doc.project.effect(effect_id).unwrap().1.kind;
    assert!(matches!(effect, EffectKind::Fill { source: Source::Uniform(_), .. }));
}

/// An image on a fill's Normal channel offers a DirectX toggle that flips its green.
#[test]
fn a_normal_image_can_be_marked_directx() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([128, 128, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("normal_dx.png", png).unwrap();
    let out = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    let layer = LayerId(out["layer"].as_u64().unwrap());
    texturecraft_engine::execute(&mut session, "layer.fill.set", &json!({"layer": layer.0, "channel": "normal", "image": name})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    h.get_by_label("DirectX normal map").scroll_to_me();
    h.run_steps(30);
    h.get_by_label("DirectX normal map").click();
    h.run_steps(3);

    let LayerKind::Fill(fill) = &h.state().session.doc.project.layers[&layer].kind else { panic!("a fill layer") };
    assert_eq!(fill.source(Channel::Normal), Source::Image { name, normal_format: texturecraft_engine::core::NormalFormat::DirectX });
}
