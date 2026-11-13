//! A fill channel takes an image resource from its source menu.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{Channel, LayerId, LayerKind, Source};
use texturecraft_ui_egui::TextureCraftApp;

/// The Roughness channel's name in Properties (the rightmost "Roughness" label).
fn roughness<'a>(h: &'a Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'a> {
    h.get_all_by_label("Roughness").max_by(|a, b| a.rect().right().total_cmp(&b.rect().right())).unwrap()
}

/// Opens the source menu of the Roughness channel, which sits in the row of the channel's name.
fn open_roughness_source_menu(h: &mut Harness<'_, TextureCraftApp>) {
    // The row is scrolled into view first, and the scroll animates for about twenty frames.
    roughness(h).scroll_to_me();
    h.run_steps(30);
    let row = roughness(h).rect().center().y;
    let menu = h.get_all_by_label(egui_phosphor::regular::DOTS_THREE).find(|n| (n.rect().center().y - row).abs() < 6.0).unwrap();
    menu.click();
    h.run_steps(2);
}

#[test]
fn a_fill_channel_takes_an_image_from_its_source_menu() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 90, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("rough.png", png).unwrap();
    let out = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"roughness": 0.6})).unwrap();
    let layer = LayerId(out["layer"].as_u64().unwrap());
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The labels below are English; a first run would speak the system's language.
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    open_roughness_source_menu(&mut h);
    h.get_by_label(&name).click();
    h.run_steps(3);

    let LayerKind::Fill(fill) = &h.state().session.doc.project.layers[&layer].kind else { panic!("a fill layer") };
    assert_eq!(fill.source(Channel::Roughness), Source::image(name));
}

/// A resource that did not decode is not offered, since the commands that take an image refuse it.
#[test]
fn the_source_menu_offers_only_images_that_were_decoded() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 90, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("rough.png", png).unwrap();
    // Bytes that are not an image never reach the decoded images
    session.doc.resources.insert("broken.png".into(), std::sync::Arc::new(vec![1, 2, 3]));
    texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"roughness": 0.6})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    // The labels below are English; a first run would speak the system's language.
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    open_roughness_source_menu(&mut h);

    assert!(h.query_by_label(&name).is_some(), "a decoded image is offered");
    assert!(h.query_by_label("broken.png").is_none(), "an image that did not decode is not");
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
