//! Generator inputs that read an anchor point or an image, through engine commands.

use serde_json::{Value, json};
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// The texture set's texels per side.
const RESOLUTION: u32 = 256;

fn session() -> Session {
    let mut session = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, RESOLUTION).unwrap();
    session.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    session.camera.yaw = 0.0;
    session.camera.pitch = 0.0;
    session.update();
    session
}

/// Composited value of `channel` under the viewport center.
fn center(session: &mut Session, channel: Channel) -> [f32; 3] {
    session.update();
    let hit = session.pick(session.viewport * 0.5, session.viewport).unwrap();
    let texels = RESOLUTION as f32;
    session.caches[0].composite.value(channel, (hit.uv.x * texels) as u32, (hit.uv.y * texels) as u32)
}

fn id(value: Value, key: &str) -> u64 {
    value[key].as_u64().unwrap()
}

/// A low fill (height 0.8) with an anchor, then a white fill whose black mask holds an AO generator reading it: (anchor, low layer, generator).
fn anchored_generator(session: &mut Session) -> (u64, u64, u64) {
    let low = id(execute(session, "layer.add.fill", &json!({"height": 0.8})).unwrap(), "layer");
    let anchor = id(execute(session, "effect.add.anchor", &json!({"name": "Height"})).unwrap(), "effect");
    execute(session, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0]})).unwrap();
    execute(session, "mask.add.black", &Value::Null).unwrap();
    let generator = id(execute(session, "effect.add.generator", &json!({"generator": "ambientOcclusion", "level": 0.5, "softness": 0.5})).unwrap(), "effect");
    execute(session, "effect.set", &json!({"effect": generator, "input": "ambientOcclusion", "inputAnchor": anchor, "anchorChannel": "height"})).unwrap();
    (anchor, low, generator)
}

#[test]
fn a_generator_input_reads_the_anchor_points_value() {
    let mut session = session();
    anchored_generator(&mut session);

    // The low fill's gray, then white through the mask: an occlusion of 0.8 masks most of it, the channel default (0) none
    let base = center(&mut session, Channel::BaseColor)[0];
    assert!(base > 0.9, "the generator reads the anchor's height, not the default: {base}");
}

#[test]
fn a_generator_reading_an_anchor_cannot_be_pasted_below_it() {
    let mut session = session();
    let (_, low, generator) = anchored_generator(&mut session);
    execute(&mut session, "effect.copy", &json!({"effect": generator})).unwrap();

    let pasted = execute(&mut session, "effect.paste", &json!({"layer": low, "target": "mask"}));
    assert!(pasted.is_err(), "the anchor point is on that layer, not below it");
}

#[test]
fn removing_unused_resources_keeps_an_image_read_by_a_generator_input() {
    let mut session = session();
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([128, 128, 128, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    let name = session.import_resource_bytes("occlusion.png", bytes).unwrap();
    execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    let generator = id(execute(&mut session, "effect.add.generator", &json!({"generator": "dirt"})).unwrap(), "effect");
    execute(&mut session, "effect.set", &json!({"effect": generator, "input": "ambientOcclusion", "inputImage": name})).unwrap();

    execute(&mut session, "file.removeUnused", &Value::Null).unwrap();
    assert!(session.doc.resources.contains_key(&name), "the image is still read by the generator input");
}
