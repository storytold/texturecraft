//! A generator's texture (Use Texture) through `effect.add.generator` and `effect.set`: its controls, its image as the `texture` input, and the kinds without one

use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, Generator, GeneratorGrayscale, LayerId, Source, TextureBlend, TextureLayer};
use texturecraft_engine::mask_cmds::section_key_applies;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut session = texturecraft_engine::sample_session(128, 64).unwrap();
    execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    session
}

fn add(session: &mut Session, params: Value) -> u64 {
    execute(session, "effect.add.generator", &params).unwrap()["effect"].as_u64().unwrap()
}

fn generator(session: &Session, effect: u64) -> Generator {
    match &session.doc.project.effect(LayerId(effect)).unwrap().1.kind {
        EffectKind::Generator(generator) => generator.clone(),
        other => panic!("not a generator: {other:?}"),
    }
}

/// A gray image resource, by the name it was imported under
fn gray_image(session: &mut Session) -> String {
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([128, 128, 128, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    session.import_resource_bytes("grunge.png", bytes).unwrap()
}

#[test]
fn texture_controls_start_from_the_reference_defaults_and_clamp() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "curvature"}));
    assert_eq!(generator(&session, effect).texture, None, "untouched until a texture control is given");

    let edit =
        json!({"effect": effect, "useTexture": true, "textureScale": 40.0, "textureContrast": -3.0, "textureBlend": "Soft light", "textureGrayscale": "red"});
    execute(&mut session, "effect.set", &edit).unwrap();
    let expected = TextureLayer {
        enabled: true,
        scale: *TextureLayer::SCALE_RANGE.end(),
        contrast: -1.0,
        blend: TextureBlend::SoftLight,
        grayscale: GeneratorGrayscale::Red,
        ..TextureLayer::default()
    };
    assert_eq!(generator(&session, effect).texture, Some(Box::new(expected)));
    assert_eq!(TextureLayer::default().opacity, 0.0, "the reference starts the texture at opacity 0");

    assert!(execute(&mut session, "effect.set", &json!({"effect": effect, "textureBlend": "dissolve"})).is_err());
    assert!(execute(&mut session, "effect.set", &json!({"effect": effect, "textureGrayscale": "hue"})).is_err());
}

#[test]
fn the_texture_image_is_the_texture_input_and_stays_a_used_resource() {
    let mut session = session();
    let name = gray_image(&mut session);
    let effect = add(&mut session, json!({"generator": "ambientOcclusion"}));

    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "texture", "inputImage": name})).unwrap();
    let plugged = generator(&session, effect);
    assert_eq!(plugged.texture.as_ref().and_then(|texture| texture.source.clone()), Some(Source::image(name.clone())));
    assert!(plugged.inputs.is_empty(), "the texture is not a mesh map input");

    execute(&mut session, "file.removeUnused", &Value::Null).unwrap();
    assert!(session.doc.resources.contains_key(&name), "the image is still read by the texture");

    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "texture", "inputBaked": true})).unwrap();
    assert_eq!(generator(&session, effect).texture.and_then(|texture| texture.source), None, "inputBaked empties the slot");
}

#[test]
fn a_texture_moves_an_earlier_generator_onto_its_section_controls() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "position", "level": 0.8}));
    assert_eq!(generator(&session, effect).section, None);

    execute(&mut session, "effect.set", &json!({"effect": effect, "textureOpacity": 0.5})).unwrap();
    let moved = generator(&session, effect);
    assert!(moved.section.is_some());
    assert_eq!(moved.texture.map(|texture| texture.opacity), Some(0.5));

    let both = json!({"effect": effect, "useTexture": true, "level": 0.2});
    assert!(execute(&mut session, "effect.set", &both).is_err(), "a texture control and the earlier model at once");
}

#[test]
fn kinds_without_a_texture_refuse_its_controls_and_its_input() {
    let mut session = session();
    let name = gray_image(&mut session);
    for kind in ["dirt", "edgeWear", "fiberGlassEdgeWear", "direction", "thinAreas"] {
        let effect = add(&mut session, json!({"generator": kind}));
        assert!(execute(&mut session, "effect.set", &json!({"effect": effect, "useTexture": true})).is_err(), "{kind} useTexture");
        assert!(execute(&mut session, "effect.set", &json!({"effect": effect, "input": "texture", "inputImage": name})).is_err(), "{kind} texture input");
        assert_eq!(generator(&session, effect).texture, None, "{kind}");
    }
    for kind in ["curvature", "ambientOcclusion", "position", "worldSpaceNormals"] {
        let effect = add(&mut session, json!({"generator": kind}));
        let added = generator(&session, effect);
        assert!(section_key_applies(added.kind, "textureTriplanar"), "{kind}");
    }
}
