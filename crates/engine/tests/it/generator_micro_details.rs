//! A generator's micro details through `effect.add.generator` and `effect.set`: their controls per kind, their images as the `microHeight` and `microNormal` inputs, and the kinds without them

use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, Generator, LayerId, MicroCurvatureType, Source};
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
    session.import_resource_bytes("height.png", bytes).unwrap()
}

#[test]
fn curvature_takes_its_micro_details_controls() {
    let mut session = session();
    let params = json!({"generator": "curvature", "useMicroDetails": true, "microNormal": false, "microCurvatureType": "smooth", "microCurvatureIntensity": 4.0, "microHeightIntensity": 20.0});
    let effect = add(&mut session, params);

    let micro = generator(&session, effect).micro.unwrap();
    assert!(micro.enabled && micro.height && !micro.normal);
    assert_eq!(micro.curvature_type, MicroCurvatureType::Smooth);
    assert_eq!((micro.curvature_intensity, micro.height_intensity), (4.0, 10.0), "the height intensity stops at 10");
    let inspected = execute(&mut session, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspected.contains("\"micro\""), "inspect shows the micro details");
}

#[test]
fn each_kind_refuses_the_micro_controls_it_lacks() {
    let mut session = session();
    let refused = [
        ("curvature", "microAoRadius"),
        ("ambientOcclusion", "microCurvatureType"),
        ("ambientOcclusion", "microHeightIntensity"),
        ("edgeWear", "useMicroDetails"),
        ("dirt", "useMicroDetails"),
        ("position", "microHeight"),
        ("worldSpaceNormals", "microNormal"),
        ("direction", "microAoDepth"),
    ];
    for (kind, key) in refused {
        let effect = add(&mut session, json!({"generator": kind}));
        let value = if key == "microCurvatureType" { json!("sobel") } else { json!(true) };
        let value = if key.starts_with("microAo") || key.ends_with("Intensity") { json!(0.5) } else { value };
        assert!(execute(&mut session, "effect.set", &json!({"effect": effect, key: value})).is_err(), "{kind} {key}");
        assert_eq!(generator(&session, effect).micro, None, "{kind} {key}");
    }
}

#[test]
fn the_wear_kinds_cap_their_curvature_intensity_at_one() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "fiberGlassEdgeWear", "microHeight": true, "microCurvatureIntensity": 3.0}));

    assert_eq!(generator(&session, effect).micro.unwrap().curvature_intensity, 1.0);
}

#[test]
fn the_micro_images_are_inputs_and_stay_used_resources() {
    let mut session = session();
    let name = gray_image(&mut session);
    let effect = add(&mut session, json!({"generator": "ambientOcclusion", "useMicroDetails": true}));

    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "microHeight", "inputImage": name})).unwrap();
    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "microNormal", "inputValue": 0.5})).unwrap();
    let micro = generator(&session, effect).micro.unwrap();
    assert_eq!(micro.height_source, Some(Source::image(name.clone())));
    assert_eq!(micro.normal_source, Some(Source::Uniform([0.5; 3])));
    assert!(generator(&session, effect).inputs.is_empty(), "micro images are not mesh map inputs");

    execute(&mut session, "file.removeUnused", &Value::Null).unwrap();
    assert!(session.doc.resources.contains_key(&name), "the image is still read by the micro height");

    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "microHeight", "inputBaked": true})).unwrap();
    assert_eq!(generator(&session, effect).micro.unwrap().height_source, None, "inputBaked empties the slot");
    let anchor = json!({"effect": effect, "input": "microNormal", "inputAnchor": effect});
    assert!(execute(&mut session, "effect.set", &anchor).is_err(), "the micro normal reads no anchor point");
}

#[test]
fn micro_details_move_an_earlier_generator_onto_the_reference_controls() {
    let mut session = session();
    let name = gray_image(&mut session);
    let wear = add(&mut session, json!({"generator": "edgeWear", "level": 0.8, "offset": 0.2}));
    let curvature = add(&mut session, json!({"generator": "curvature", "level": 0.8}));
    assert!(generator(&session, wear).on_earlier_model() && generator(&session, curvature).on_earlier_model());

    execute(&mut session, "effect.set", &json!({"effect": wear, "input": "microHeight", "inputImage": name})).unwrap();
    execute(&mut session, "effect.set", &json!({"effect": curvature, "useMicroDetails": true})).unwrap();
    let moved = generator(&session, wear);
    assert!(moved.wear.is_some() && moved.offset == 0.0, "Metal Edge Wear on its wear controls, without the earlier offset");
    assert!(generator(&session, curvature).section.is_some(), "Curvature on its section controls");

    let both = json!({"effect": wear, "microHeight": true, "softness": 0.2});
    assert!(execute(&mut session, "effect.set", &both).is_err(), "micro details and the earlier model at once");
    let global = json!({"effect": wear, "microHeight": true, "blur": 0.2});
    assert!(execute(&mut session, "effect.set", &global).is_err(), "the wear kinds have no global blur");
}

#[test]
fn malformed_micro_detail_switches_leave_the_saved_model_intact() {
    let mut session = session();
    let effect = add(&mut session, json!({"generator": "curvature", "level": 0.8}));
    let original = generator(&session, effect);
    assert!(original.on_earlier_model());

    for key in ["microHeight", "microNormal", "useMicroDetails"] {
        let result = execute(&mut session, "effect.set", &json!({"effect": effect, key: "true"}));

        assert!(result.is_err(), "{key} must be a JSON boolean");
        assert_eq!(generator(&session, effect), original, "{key} must leave the saved generator intact");
    }
}

#[test]
fn swapping_kinds_keeps_the_micro_images_where_the_kind_has_micro_details() {
    let mut session = session();
    let name = gray_image(&mut session);
    let effect = add(&mut session, json!({"generator": "curvature", "useMicroDetails": true, "microCurvatureIntensity": 4.0}));
    execute(&mut session, "effect.set", &json!({"effect": effect, "input": "microHeight", "inputImage": name})).unwrap();

    execute(&mut session, "effect.set", &json!({"effect": effect, "generator": "edgeWear"})).unwrap();
    let micro = generator(&session, effect).micro.unwrap();
    assert_eq!(micro.height_source, Some(Source::image(name)));
    assert_eq!(micro.curvature_intensity, 0.5, "the controls start from Metal Edge Wear's defaults");

    execute(&mut session, "effect.set", &json!({"effect": effect, "generator": "position"})).unwrap();
    assert_eq!(generator(&session, effect).micro, None, "Position has no micro details");
}
