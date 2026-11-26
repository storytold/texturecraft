//! A wear generator's custom grunge through `effect.set`: Use Custom Grunge, its image as the `grunge` input, and the kinds without one

use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, Generator, LayerId, Source};
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
fn use_custom_grunge_is_a_wear_control_of_every_wear_kind() {
    let mut session = session();
    for kind in ["edgeWear", "fiberGlassEdgeWear", "dirt"] {
        let effect = add(&mut session, json!({"generator": kind}));
        let wear = |session: &Session| generator(session, effect).wear.map(|wear| wear.custom_grunge);
        assert_ne!(wear(&session), Some(true), "{kind}: off until turned on");

        execute(&mut session, "effect.set", &json!({"effect": effect, "useCustomGrunge": true})).unwrap();
        assert_eq!(wear(&session), Some(true), "{kind}");
    }
}

#[test]
fn the_grunge_image_is_the_grunge_input_and_stays_a_used_resource() {
    let mut session = session();
    let name = gray_image(&mut session);
    let effect = add(&mut session, json!({"generator": "dirt", "wearLevel": 0.6}));

    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "grunge", "inputImage": name})).unwrap();
    let plugged = generator(&session, effect);
    assert_eq!(plugged.grunge, Some(Source::image(name.clone())));
    assert!(plugged.inputs.is_empty(), "the grunge is not a mesh map input");

    execute(&mut session, "file.removeUnused", &Value::Null).unwrap();
    assert!(session.doc.resources.contains_key(&name), "the image is still read by the grunge");

    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "grunge", "inputBaked": true})).unwrap();
    assert_eq!(generator(&session, effect).grunge, None, "inputBaked empties the slot");
}

#[test]
fn a_grunge_image_moves_an_earlier_wear_generator_onto_its_wear_controls() {
    let mut session = session();
    let name = gray_image(&mut session);
    let effect = add(&mut session, json!({"generator": "edgeWear", "level": 0.8}));
    assert_eq!(generator(&session, effect).wear, None);

    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "grunge", "inputImage": name})).unwrap();
    assert!(generator(&session, effect).wear.is_some());

    let both = json!({"effect": effect, "input": "grunge", "inputImage": name, "level": 0.2});
    assert!(execute(&mut session, "effect.set", &both).is_err(), "a grunge image and the earlier model at once");
    let both = json!({"effect": effect, "useCustomGrunge": true, "softness": 0.2});
    assert!(execute(&mut session, "effect.set", &both).is_err(), "Use Custom Grunge and the earlier model at once");
}

#[test]
fn kinds_without_wear_controls_refuse_a_custom_grunge() {
    let mut session = session();
    let name = gray_image(&mut session);
    for kind in ["curvature", "ambientOcclusion", "position", "worldSpaceNormals", "direction", "thinAreas"] {
        let effect = add(&mut session, json!({"generator": kind}));
        assert!(execute(&mut session, "effect.set", &json!({"effect": effect, "useCustomGrunge": true})).is_err(), "{kind} useCustomGrunge");
        assert!(execute(&mut session, "effect.set", &json!({"effect": effect, "input": "grunge", "inputImage": name})).is_err(), "{kind} grunge input");
        assert_eq!(generator(&session, effect).grunge, None, "{kind}");
    }
}

#[test]
fn swapping_between_wear_kinds_keeps_the_grunge_image() {
    let mut session = session();
    let name = gray_image(&mut session);
    let effect = add(&mut session, json!({"generator": "edgeWear"}));
    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "grunge", "inputImage": name})).unwrap();

    execute(&mut session, "effect.set", &json!({"effect": effect, "generator": "dirt"})).unwrap();
    assert_eq!(generator(&session, effect).grunge, Some(Source::image(name)), "Dirt reads a custom grunge too");

    execute(&mut session, "effect.set", &json!({"effect": effect, "generator": "curvature"})).unwrap();
    assert_eq!(generator(&session, effect).grunge, None, "Curvature has no grunge");
}
