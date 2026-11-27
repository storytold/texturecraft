//! The wear generators' controls through `effect.add.generator` and `effect.set`, beside their earlier level, softness and noise

use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, Generator, GeneratorKind, LayerId, WearParameters};
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

#[test]
fn new_wear_generators_start_from_the_reference_defaults() {
    let mut session = session();
    for kind in ["edgeWear", "dirt", "fiberGlassEdgeWear"] {
        let effect = add(&mut session, json!({"generator": kind}));
        let added = generator(&session, effect);
        assert_eq!(added.wear, WearParameters::defaults(added.kind), "{kind}");
    }
}

#[test]
fn level_softness_or_noise_keep_a_generator_on_the_earlier_model() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "edgeWear", "level": 0.4}));
    assert_eq!(generator(&session, effect).wear, None, "agents' earlier parameters still drive it");
    assert_eq!(generator(&session, effect).level, 0.4);
}

#[test]
fn a_wear_control_moves_an_earlier_generator_onto_the_defaults_with_that_change() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "dirt", "softness": 0.3}));

    execute(&mut session, "effect.set", &json!({"effect": effect, "wearLevel": 0.6, "grungeScale": 99.0})).unwrap();

    let wear = generator(&session, effect).wear.expect("on the wear controls now");
    let defaults = WearParameters::defaults(GeneratorKind::Dirt).unwrap();
    assert_eq!(
        wear,
        WearParameters { level: 0.6, grunge_scale: *WearParameters::GRUNGE_SCALE_RANGE.end(), ..defaults },
        "the rest at defaults, values clamped"
    );
}

#[test]
fn mixing_the_models_or_wear_controls_on_other_kinds_is_refused() {
    let mut session = session();
    let edge_wear = add(&mut session, json!({"generator": "edgeWear"}));
    let curvature = add(&mut session, json!({"generator": "curvature"}));

    assert!(execute(&mut session, "effect.set", &json!({"effect": edge_wear, "level": 0.5, "wearLevel": 0.5})).is_err());
    assert!(execute(&mut session, "effect.set", &json!({"effect": curvature, "wearLevel": 0.5})).is_err());
    assert!(execute(&mut session, "effect.add.generator", &json!({"generator": "fiberGlassEdgeWear", "level": 0.5})).is_err());
    assert_eq!(generator(&session, edge_wear).wear, WearParameters::defaults(GeneratorKind::EdgeWear), "a refused edit changes nothing");
}

#[test]
fn controls_a_kind_does_not_use_are_refused_rather_than_ignored() {
    let mut session = session();
    let edge_wear = add(&mut session, json!({"generator": "edgeWear"}));
    let dirt = add(&mut session, json!({"generator": "dirt"}));
    let fiber_glass = add(&mut session, json!({"generator": "fiberGlassEdgeWear"}));

    assert!(execute(&mut session, "effect.set", &json!({"effect": edge_wear, "edgesMasking": 0.2})).is_err(), "edge wear has no edges masking");
    assert!(execute(&mut session, "effect.set", &json!({"effect": dirt, "edgesSmoothness": 2.0})).is_err(), "dirt has no edges smoothness");
    assert!(execute(&mut session, "effect.set", &json!({"effect": fiber_glass, "grungeScale": 8.0})).is_err(), "fiber glass has no grunge scale");
    assert!(execute(&mut session, "effect.set", &json!({"effect": edge_wear, "blur": 0.5})).is_err(), "the wear controls have no global blur");
}

#[test]
fn mode_and_sizes_keep_edge_wear_on_the_earlier_model_where_they_apply() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "edgeWear", "mode": "both"}));
    let added = generator(&session, effect);
    assert_eq!(added.wear, None, "detail sizes belong to the earlier model");
    assert!(added.detail.is_some());
    execute(&mut session, "effect.set", &json!({"effect": effect, "blur": 0.5})).unwrap();
    assert_eq!(generator(&session, effect).global.blur, 0.5, "the earlier model keeps its global blur");
}

#[test]
fn a_wear_control_drops_the_earlier_models_offset() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "edgeWear", "offset": 0.4}));
    assert_eq!(generator(&session, effect).wear, None, "the offset keeps it on the earlier model");
    assert_eq!(generator(&session, effect).offset, 0.4);

    execute(&mut session, "effect.set", &json!({"effect": effect, "wearLevel": 0.6})).unwrap();
    assert_eq!(generator(&session, effect).offset, 0.0, "on the wear controls the offset would shift the result unseen");
    assert!(execute(&mut session, "effect.set", &json!({"effect": effect, "offset": 0.2, "wearLevel": 0.5})).is_err(), "not both at once");
}
