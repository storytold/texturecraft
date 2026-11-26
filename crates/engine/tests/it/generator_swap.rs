//! Swapping a generator effect's kind in place with `effect.set {generator}`

use serde_json::{Value, json};
use texturecraft_engine::core::{BlendMode, EffectKind, Generator, GeneratorKind, LayerId};
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    texturecraft_engine::sample_session(128, 64).unwrap()
}

/// A fill layer whose black mask holds a filter, then a Dirt generator at half opacity, multiplied: (filter, generator)
fn masked(session: &mut Session) -> (u64, u64) {
    execute(session, "layer.add.fill", &Value::Null).unwrap();
    execute(session, "mask.add.black", &Value::Null).unwrap();
    let filter = execute(session, "effect.add.filter", &json!({"filter": "invert"})).unwrap()["effect"].as_u64().unwrap();
    let generator =
        execute(session, "effect.add.generator", &json!({"generator": "Dirt", "level": 0.9, "blend": "multiply"})).unwrap()["effect"].as_u64().unwrap();
    execute(session, "effect.set", &json!({"effect": generator, "opacity": 0.5})).unwrap();
    (filter, generator)
}

fn effect_kind(session: &Session, effect: u64) -> EffectKind {
    session.doc.project.effect(LayerId(effect)).unwrap().1.kind.clone()
}

#[test]
fn another_kind_keeps_the_shared_settings_and_starts_its_own_controls_from_defaults() {
    let mut session = session();
    let (_, generator) = masked(&mut session);
    let (owner, _) = session.doc.project.effect(LayerId(generator)).unwrap();
    let position = |session: &Session| session.doc.project.layer(owner).unwrap().mask.as_ref().unwrap().effects.iter().position(|e| e.id.0 == generator);
    let before = position(&session);

    execute(&mut session, "effect.set", &json!({"effect": generator, "generator": "Curvature"})).unwrap();

    let defaults = Generator::new(GeneratorKind::Curvature);
    let EffectKind::Generator(curvature) = effect_kind(&session, generator) else { panic!("still a generator") };
    assert_eq!(curvature.kind, GeneratorKind::Curvature);
    assert_eq!(curvature.level, 0.9, "keeps the shared settings");
    assert_eq!(curvature.detail, defaults.detail, "its own controls start from defaults");
    let (_, swapped) = session.doc.project.effect(LayerId(generator)).unwrap();
    assert_eq!((swapped.blend, swapped.opacity), (BlendMode::Multiply, 0.5), "keeps blending and opacity");
    assert_eq!(position(&session), before, "keeps its place in the mask");
}

#[test]
fn naming_the_current_kind_edits_the_generator_without_resetting_it() {
    let mut session = session();
    let (_, generator) = masked(&mut session);

    execute(&mut session, "effect.set", &json!({"effect": generator, "generator": "Dirt", "softness": 0.4})).unwrap();

    let EffectKind::Generator(edited) = effect_kind(&session, generator) else { panic!("still a generator") };
    assert_eq!((edited.level, edited.softness), (0.9, 0.4));
}

#[test]
fn only_generator_effects_take_a_generator_kind() {
    let mut session = session();
    let (filter, generator) = masked(&mut session);

    assert!(execute(&mut session, "effect.set", &json!({"effect": filter, "generator": "Curvature"})).is_err());
    assert!(execute(&mut session, "effect.set", &json!({"effect": generator, "generator": "nope"})).is_err());
    assert!(matches!(effect_kind(&session, filter), EffectKind::Filter(_)), "a refused swap changes nothing");
}

#[test]
fn use_texture_goes_along_between_the_kinds_that_have_it() {
    let mut session = session();
    execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    execute(&mut session, "mask.add.black", &Value::Null).unwrap();
    let added = json!({"generator": "curvature", "useTexture": true, "textureOpacity": 0.75, "textureScale": 2.0});
    let effect = execute(&mut session, "effect.add.generator", &added).unwrap()["effect"].as_u64().unwrap();
    let texture = |session: &Session| match effect_kind(session, effect) {
        EffectKind::Generator(generator) => generator.texture.map(|texture| (texture.enabled, texture.opacity, texture.scale)),
        other => panic!("not a generator: {other:?}"),
    };
    let before = texture(&session);

    execute(&mut session, "effect.set", &json!({"effect": effect, "generator": "ambientOcclusion"})).unwrap();
    assert_eq!(texture(&session), before, "Ambient Occlusion has Use Texture too");

    execute(&mut session, "effect.set", &json!({"effect": effect, "generator": "dirt"})).unwrap();
    assert_eq!(texture(&session), None, "Dirt has none");
}
