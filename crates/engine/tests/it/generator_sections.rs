//! The section and light controls through `effect.add.generator` and `effect.set`, beside the earlier level, softness and noise

use serde_json::{Value, json};
use texturecraft_engine::core::{AxisBlend, EffectKind, Generator, GeneratorKind, LayerId, LightParameters, SectionParameters};
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

#[test]
fn new_generators_start_on_the_reference_controls() {
    let mut session = session();
    for kind in ["curvature", "ambientOcclusion", "position", "worldSpaceNormals"] {
        let effect = add(&mut session, json!({"generator": kind}));
        let added = generator(&session, effect);
        assert_eq!(added.section, SectionParameters::defaults(added.kind), "{kind}");
    }
    let effect = add(&mut session, json!({"generator": "direction"}));
    let light = generator(&session, effect);
    assert_eq!(light.light, LightParameters::defaults(GeneratorKind::Direction));
    assert_eq!(GeneratorKind::parse("World Space Normals"), Some(GeneratorKind::WorldSpaceNormals));
}

#[test]
fn a_section_control_moves_an_earlier_generator_onto_the_defaults_with_that_change() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "position", "level": 0.8}));
    assert_eq!(generator(&session, effect).section, None, "level keeps it on the earlier model");

    let edit = json!({"effect": effect, "sectionBalance": 0.3, "rightLeftBlend": "Max (Lighten)", "frontBackWeight": 4.0, "sectionBrightness": -3.0});
    execute(&mut session, "effect.set", &edit).unwrap();

    let moved = generator(&session, effect);
    let mut expected = SectionParameters::defaults(GeneratorKind::Position).unwrap();
    expected.balance = 0.3;
    expected.brightness = -1.0;
    expected.axes[SectionParameters::RIGHT_TO_LEFT].blend = AxisBlend::Max;
    expected.axes[SectionParameters::FRONT_TO_BACK].weight = 1.0;
    assert_eq!(moved.section, Some(expected), "the rest at defaults, values clamped");
    assert_eq!(moved.level, Generator::new(GeneratorKind::Position).level, "the earlier model's values start over");

    execute(&mut session, "effect.set", &json!({"effect": effect, "noise": 0.5})).unwrap();
    assert_eq!(generator(&session, effect).section, None, "noise moves it back");
}

#[test]
fn keys_a_kind_lacks_or_both_models_at_once_are_refused() {
    let mut session = session();
    let curvature = add(&mut session, json!({"generator": "curvature"}));
    let occlusion = add(&mut session, json!({"generator": "ambientOcclusion"}));
    let position = add(&mut session, json!({"generator": "position"}));
    let normals = add(&mut session, json!({"generator": "worldSpaceNormals"}));
    let dirt = add(&mut session, json!({"generator": "dirt"}));

    for (effect, key) in [
        (curvature, "sectionBalance"),
        (curvature, "sectionBlur"),
        (curvature, "topBottomWeight"),
        (occlusion, "sectionBrightness"),
        (occlusion, "rightLeftInvert"),
        (position, "sectionBlur"),
        (position, "lightHorizontal"),
        (dirt, "sectionContrast"),
    ] {
        assert!(execute(&mut session, "effect.set", &json!({"effect": effect, key: 0.5})).is_err(), "{key} on {effect}");
    }
    for (effect, key) in [(curvature, "sectionBrightness"), (occlusion, "sectionBlur"), (normals, "sectionBlur"), (normals, "frontBackWeight")] {
        execute(&mut session, "effect.set", &json!({"effect": effect, key: 0.5})).unwrap();
    }

    assert!(execute(&mut session, "effect.set", &json!({"effect": position, "level": 0.5, "sectionContrast": 0.5})).is_err());
    assert!(execute(&mut session, "effect.set", &json!({"effect": normals, "softness": 0.5})).is_err(), "World Space Normals has no earlier model");
    assert!(execute(&mut session, "effect.set", &json!({"effect": position, "topBottomBlend": "overlay"})).is_err());
    assert_eq!(generator(&session, position).section, SectionParameters::defaults(GeneratorKind::Position), "a refused edit changes nothing");

    let direction = json!([1.0, 0.0, 0.0]);
    assert!(execute(&mut session, "effect.set", &json!({"effect": curvature, "direction": direction})).is_err(), "Curvature points no way");
    assert!(execute(&mut session, "effect.set", &json!({"effect": occlusion, "direction": direction})).is_err(), "nor does Ambient Occlusion");

    assert!(!section_key_applies(GeneratorKind::Position, "sectionBlur"));
    assert!(section_key_applies(GeneratorKind::WorldSpaceNormals, "rightLeftBlend"));
}

#[test]
fn light_takes_its_controls_and_refuses_the_global_ones() {
    let mut session = session();
    let light = add(&mut session, json!({"generator": "direction"}));
    execute(&mut session, "effect.set", &json!({"effect": light, "lightHorizontal": 400.0, "highlightGlossiness": 1.0, "invert": true})).unwrap();
    let edited = generator(&session, light);
    let expected = LightParameters {
        horizontal: 360.0,
        glossiness: *LightParameters::GLOSSINESS_RANGE.end(),
        ..LightParameters::defaults(GeneratorKind::Direction).unwrap()
    };
    assert_eq!(edited.light, Some(expected));
    assert!(edited.invert);

    assert!(execute(&mut session, "effect.set", &json!({"effect": light, "blur": 0.5})).is_err(), "Light has no global blur");
    assert!(execute(&mut session, "effect.set", &json!({"effect": light, "sectionInvert": true})).is_err());

    let earlier = add(&mut session, json!({"generator": "direction", "level": 0.4, "blur": 0.3}));
    assert_eq!(generator(&session, earlier).light, None, "the earlier model keeps its global adjustments");
    execute(&mut session, "effect.set", &json!({"effect": earlier, "lightVertical": 60.0})).unwrap();
    let moved = generator(&session, earlier);
    assert_eq!(moved.light.map(|light| light.vertical), Some(60.0));
    assert_eq!(moved.global, Generator::new(GeneratorKind::Direction).global, "the light controls have no global adjustments");
}

/// Position and Light read the mesh itself on the earlier model, so they take no Position input there
#[test]
fn an_input_the_generator_does_not_read_is_refused() {
    let mut session = session();
    let earlier = add(&mut session, json!({"generator": "position", "level": 0.4}));
    let plug = |effect: u64| json!({"effect": effect, "input": "position", "inputValue": 0.5});
    assert!(execute(&mut session, "effect.set", &plug(earlier)).is_err());
    assert!(generator(&session, earlier).inputs.is_empty());

    let on_section = add(&mut session, json!({"generator": "position"}));
    execute(&mut session, "effect.set", &plug(on_section)).unwrap();
    assert_eq!(generator(&session, on_section).inputs.len(), 1);
}

#[test]
fn the_offset_belongs_to_the_earlier_model() {
    let mut session = session();
    for (kind, control) in [("position", json!({"sectionBalance": 0.3})), ("direction", json!({"highlightLevel": 0.5}))] {
        let effect = add(&mut session, json!({"generator": kind, "offset": 0.4}));
        assert_eq!(generator(&session, effect).offset, 0.4, "{kind}: the offset keeps it on the earlier model");

        let mut edit = control.clone();
        edit["effect"] = json!(effect);
        execute(&mut session, "effect.set", &edit).unwrap();
        assert_eq!(generator(&session, effect).offset, 0.0, "{kind}: moving onto the reference controls drops the unseen offset");

        let mut both = control;
        both["effect"] = json!(effect);
        both["offset"] = json!(0.2);
        assert!(execute(&mut session, "effect.set", &both).is_err(), "{kind}: offset and the reference controls at once are refused");
    }
}

#[test]
fn inspect_reports_the_reference_controls_set() {
    let mut session = session();
    add(&mut session, json!({"generator": "position", "sectionContrast": 0.25}));
    add(&mut session, json!({"generator": "direction", "lightHorizontal": 90.0}));
    add(&mut session, json!({"generator": "dirt", "wearLevel": 0.125}));

    let inspected = execute(&mut session, "project.inspect", &Value::Null).unwrap().to_string();
    for (key, value) in [("section", "\"contrast\":0.25"), ("light", "\"horizontal\":90.0"), ("wear", "\"level\":0.125")] {
        assert!(inspected.contains(&format!("\"{key}\"")) && inspected.contains(value), "{key} with {value} in {inspected}");
    }
}
