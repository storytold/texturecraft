//! A fill's procedural pattern through `layer.fill.set`: a parameter given alone changes only itself

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId, LayerKind, Procedural, ProceduralKind, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// A fill layer whose roughness is a Clouds pattern with every parameter away from its default
fn clouds_fill() -> (Session, LayerId) {
    let mut session = Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
    let layer = LayerId(execute(&mut session, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    let pattern = json!({
        "layer": layer.0, "channel": "roughness", "procedural": "Clouds", "scale": 9.0, "contrast": 1.5, "seed": 7,
        "balance": 0.25, "invert": true, "low": [0.2, 0.2, 0.2], "high": [0.8, 0.8, 0.8],
    });
    execute(&mut session, "layer.fill.set", &pattern).unwrap();
    (session, layer)
}

/// The roughness pattern and its Low and High colours
type Roughness = (Procedural, [f32; 3], [f32; 3]);

fn roughness(session: &Session, layer: LayerId) -> Roughness {
    match &session.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(fill) => match fill.source(Channel::Roughness) {
            Source::Procedural { pattern, low, high, .. } => (pattern, low, high),
            other => panic!("not a procedural: {other:?}"),
        },
        other => panic!("not a fill: {other:?}"),
    }
}

/// One parameter sent alone, its value, and what it changes in the stored pattern
type PatternEdit = (&'static str, Value, fn(&mut Roughness));

/// What the Scale, Contrast, Seed, Balance and Invert controls and the Low/High swatches send: one parameter each
#[test]
fn a_pattern_parameter_given_alone_keeps_every_other_one() {
    let (mut session, layer) = clouds_fill();
    let edits: [PatternEdit; 7] = [
        ("scale", json!(3.0), |(pattern, ..)| pattern.scale = 3.0),
        ("contrast", json!(0.5), |(pattern, ..)| pattern.contrast = 0.5),
        ("seed", json!(42), |(pattern, ..)| pattern.seed = 42),
        ("balance", json!(-0.5), |(pattern, ..)| pattern.balance = -0.5),
        ("invert", json!(false), |(pattern, ..)| pattern.invert = false),
        ("low", json!([0.1, 0.1, 0.1]), |(_, low, _)| *low = [0.1; 3]),
        ("high", json!([0.9, 0.9, 0.9]), |(.., high)| *high = [0.9; 3]),
    ];
    for (key, value, change) in edits {
        let mut expected = roughness(&session, layer);
        change(&mut expected);
        let mut edit = json!({"layer": layer.0, "channel": "roughness"});
        edit[key] = value;
        execute(&mut session, "layer.fill.set", &edit).unwrap();

        assert_eq!(roughness(&session, layer), expected, "`{key}` alone changes only itself");
    }

    let inspected = execute(&mut session, "project.inspect", &Value::Null).unwrap().to_string();

    assert!(inspected.contains("\"balance\":-0.5") && inspected.contains("\"invert\":false"), "inspect shows the balance and invert");
}

/// Another kind is a new pattern: its defaults between black and white, as before
#[test]
fn another_pattern_kind_starts_from_its_defaults() {
    let (mut session, layer) = clouds_fill();
    execute(&mut session, "layer.fill.set", &json!({"layer": layer.0, "channel": "roughness", "procedural": "Cells"})).unwrap();

    assert_eq!(roughness(&session, layer), (Procedural::new(ProceduralKind::Cells), [0.0; 3], [1.0; 3]));
}
