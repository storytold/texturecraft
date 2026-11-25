//! Painting a mask with an image: the tool's Grayscale slot set to an image paints its gray per texel

use serde_json::{Value, json};
use texturecraft_engine::core::LayerId;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const RESOLUTION: u32 = 256;

/// An 8 × 8 picture: black on its left half, white on its right half
fn halves_png() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(8, 8, |x, _| if x < 4 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) });
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

/// A paint layer under a black mask, the mask selected for painting, a big brush
fn masked_layer() -> (Session, LayerId) {
    let mut session = Session::new_project("Mask image", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, RESOLUTION).unwrap();
    let layer = LayerId(execute(&mut session, "layer.add.paint", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    execute(&mut session, "mask.add.black", &json!({"layer": layer.0})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer.0, "target": "mask"})).unwrap();
    execute(&mut session, "tool.paint", &Value::Null).unwrap();
    execute(&mut session, "tool.brush", &json!({"size": 40.0, "spacing": 5.0, "hardness": 100})).unwrap();
    (session, layer)
}

/// The mask at texture coordinates (`u`, `v`)
fn mask_at(session: &Session, layer: LayerId, u: f32, v: f32) -> f32 {
    let texel = ((u * RESOLUTION as f32) as u32, (v * RESOLUTION as f32) as u32);
    session.mask_at(layer, &[texel]).unwrap()[0]
}

#[test]
fn a_mask_stroke_paints_the_tools_image_gray_and_null_goes_back_to_the_value() {
    let (mut session, layer) = masked_layer();
    let image = session.import_resource_bytes("halves.png", halves_png()).unwrap();
    let set = execute(&mut session, "tool.material", &json!({"maskImage": image})).unwrap();

    assert_eq!(set["maskImage"], json!(image));
    assert!(execute(&mut session, "tool.material", &json!({"maskImage": "missing.png"})).is_err(), "an unknown image is refused");
    assert!(execute(&mut session, "tool.material", &json!({"maskImage": 3})).is_err());

    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.1, 0.5], [0.9, 0.5]]})).unwrap();
    let (left, right) = (mask_at(&session, layer, 0.25, 0.5), mask_at(&session, layer, 0.75, 0.5));
    assert!(left < 0.05 && right > 0.95, "the image's halves: left {left}, right {right}");

    // Back to one value: the next stroke paints white on the left too
    let set = execute(&mut session, "tool.material", &json!({"maskImage": null})).unwrap();

    assert_eq!(set["maskImage"], Value::Null);

    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.1, 0.5], [0.9, 0.5]]})).unwrap();

    assert!(mask_at(&session, layer, 0.25, 0.5) > 0.95);
}

#[test]
fn a_refused_mask_image_leaves_the_mask_value_as_it_was() {
    let (mut session, _) = masked_layer();
    execute(&mut session, "tool.material", &json!({"maskValue": 0.25})).unwrap();

    let refused = execute(&mut session, "tool.material", &json!({"maskValue": 0.75, "maskImage": "missing.png"}));

    assert!(refused.is_err());
    assert_eq!(execute(&mut session, "tool.material", &Value::Null).unwrap()["maskValue"], json!(0.25));
}

#[test]
fn a_resource_that_is_no_image_is_refused_as_the_mask_image() {
    let (mut session, _) = masked_layer();
    session.doc.resources.insert("notes.png".to_string(), std::sync::Arc::new(b"not a picture".to_vec()));
    session.sync_images();

    let refused = execute(&mut session, "tool.material", &json!({"maskImage": "notes.png"}));

    assert!(refused.is_err(), "the brush could not read it");
    assert_eq!(execute(&mut session, "tool.material", &Value::Null).unwrap()["maskImage"], Value::Null);
}

#[test]
fn a_mask_image_that_is_gone_paints_the_value() {
    let (mut session, layer) = masked_layer();
    let image = session.import_resource_bytes("halves.png", halves_png()).unwrap();
    execute(&mut session, "tool.material", &json!({"maskImage": image, "maskValue": 0.5})).unwrap();
    session.images.remove(&image);
    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.1, 0.5], [0.9, 0.5]]})).unwrap();
    let painted = mask_at(&session, layer, 0.25, 0.5);
    assert!((painted - 0.5).abs() < 0.02, "painted {painted}");
}

/// A one-colour picture, `level` 0 for black or 255 for white
fn plain_png(level: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([level, level, level, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

#[test]
fn a_mask_stroke_paints_the_image_it_began_with_when_the_resource_is_replaced_mid_stroke() {
    let (mut session, layer) = masked_layer();
    let image = session.import_resource_bytes("brush.png", plain_png(255)).unwrap();
    execute(&mut session, "tool.material", &json!({"maskImage": image, "maskValue": 0.0})).unwrap();

    session.begin_stroke_uv(Vec2::new(0.1, 0.5), 1.0).unwrap();
    session.continue_stroke_uv(Vec2::new(0.4, 0.5), 1.0);
    execute(&mut session, "resource.remove", &json!({"name": image})).unwrap();
    let replaced = session.import_resource_bytes("brush.png", plain_png(0)).unwrap();
    session.continue_stroke_uv(Vec2::new(0.9, 0.5), 1.0);
    session.end_stroke().unwrap();

    assert_eq!(replaced, image, "the replacement kept the resource name");

    let (left, right) = (mask_at(&session, layer, 0.25, 0.5), mask_at(&session, layer, 0.75, 0.5));
    assert!(left > 0.95 && right > 0.95, "the whole stroke uses the captured white image: left {left}, right {right}");
}

#[test]
fn polygon_fill_uses_its_mask_value_while_paint_keeps_its_image() {
    let (mut session, layer) = masked_layer();
    let image = session.import_resource_bytes("black.png", plain_png(0)).unwrap();
    execute(&mut session, "tool.material", &json!({"maskImage": image, "maskValue": 1.0})).unwrap();
    let uv = Vec2::new(0.27, 0.41);

    session.polygon_fill_uv(uv).unwrap();

    assert!(mask_at(&session, layer, uv.x, uv.y) > 0.95);
    assert_eq!(session.tool.mask_image.as_ref(), Some(&image));
}
