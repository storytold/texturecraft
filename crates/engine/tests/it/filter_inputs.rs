//! Filter image inputs: Bevel Smooth's and Directional Distance's Distance Map and Warp's Custom Noise read a source set on the effect

use serde_json::{Value, json};
use texturecraft_engine::core::{Effect, LayerId, Source};
use texturecraft_engine::{Session, execute};

use crate::filter_shape::shape_edge;
use crate::filter_support::{SIZE, add_filter, roughness, session, set};

/// `layer.fill.set` on image input `params` of `effect`: its result, the error as text
fn input(s: &mut Session, effect: LayerId, params: Value) -> Result<Value, String> {
    let mut params = params;
    params["effect"] = json!(effect.0);
    execute(s, "layer.fill.set", &params).map_err(|error| error.to_string())
}

/// The image inputs `effect` holds, by input id
fn inputs(s: &Session, effect: LayerId) -> Vec<(String, Source)> {
    let (_, found) = s.doc.project.effect(effect).unwrap();
    found.inputs.iter().map(|(id, source)| (id.clone(), source.clone())).collect()
}

/// Bevel Smooth's Distance Map scales its distance texel by texel: a gray, an anchor's gray, back to white; its default is not stored; one undo step
#[test]
fn a_distance_map_scales_the_bevel_smooth_distance() {
    let mut s = session();

    // A layer below records a roughness of 0.5 at an anchor point
    execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5], "roughness": 0.5})).unwrap();
    let anchor = LayerId(execute(&mut s, "effect.add.anchor", &Value::Null).unwrap()["effect"].as_u64().unwrap());
    let (at, _, _) = shape_edge(&mut s);
    let effect = add_filter(&mut s, json!({"filter": "bevelSmooth"}));

    set(&mut s, effect, json!({"smoothing": 0.0}));
    let full = roughness(&mut s, at(4));
    assert!(full > 0.0 && full < 1.0, "on the ramp: {full}");

    input(&mut s, effect, json!({"input": "distanceMap", "value": 0.5})).unwrap();
    let half = roughness(&mut s, at(4));
    assert!(half < full - 0.05, "half the distance fades sooner: {half} vs {full}");
    assert_eq!(inputs(&s, effect), vec![("distanceMap".to_string(), Source::Uniform([0.5; 3]))]);

    let inspected = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();

    assert!(inspected.contains("\"inputs\":{\"distanceMap\""), "inspect shows the input");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert!(inputs(&s, effect).is_empty(), "one undo step");
    assert_eq!(roughness(&mut s, at(4)), full);

    input(&mut s, effect, json!({"input": "distanceMap", "anchor": anchor.0, "anchorChannel": "roughness"})).unwrap();

    assert!((roughness(&mut s, at(4)) - half).abs() < 1e-3, "the anchor's 0.5 reads as the uniform 0.5");

    input(&mut s, effect, json!({"input": "distanceMap", "clear": true})).unwrap();

    assert!(inputs(&s, effect).is_empty() && roughness(&mut s, at(4)) == full, "cleared: white, as before");

    let steps = s.history_labels().len();
    input(&mut s, effect, json!({"input": "distanceMap", "value": 1.0})).unwrap();
    input(&mut s, effect, json!({"input": "distanceMap", "clear": true})).unwrap();

    assert!(inputs(&s, effect).is_empty(), "the default gray is not stored");
    assert_eq!(s.history_labels().len(), steps, "setting what the input already reads adds no undo step");

    for bad in [json!({"input": "nope", "value": 0.5}), json!({"input": "distanceMap", "channel": "roughness", "value": 0.5}), json!({"input": "distanceMap"})]
    {
        assert!(input(&mut s, effect, bad.clone()).is_err(), "{bad}");
    }

    assert!(input(&mut s, anchor, json!({"input": "distanceMap", "value": 0.5})).is_err(), "not a filter");
}

/// Directional Distance: a black Distance Map leaves no trail
#[test]
fn a_black_distance_map_leaves_no_trail() {
    let mut s = session();
    let (at, _, outward) = shape_edge(&mut s);
    let effect = add_filter(&mut s, json!({"filter": "directionalDistance"}));

    set(&mut s, effect, json!({"angle": if outward > 0 { 0.0 } else { 180.0 }, "distance": 0.05}));

    assert!(roughness(&mut s, at(1)) > 0.0, "a trail");

    input(&mut s, effect, json!({"input": "distanceMap", "value": 0.0})).unwrap();

    assert_eq!((roughness(&mut s, at(1)), roughness(&mut s, at(-2))), (0.0, 1.0), "no trail, the shapes stay");
}

/// Warp's Custom Noise moves texels by its source; projects saved before image inputs load with none and save without them
#[test]
fn a_custom_noise_moves_the_warp() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5], "roughness": 0.25})).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"channel": "roughness", "procedural": "gradient"})).unwrap();
    let row = SIZE / 2;
    let before: Vec<f32> = (0..SIZE).map(|x| roughness(&mut s, (x, row))).collect();
    let moved = |s: &mut Session| (0..SIZE).map(|x| (roughness(s, (x, row)) - before[x as usize]).abs()).fold(0.0f32, f32::max);

    let effect = add_filter(&mut s, json!({"filter": "warp"}));
    set(&mut s, effect, json!({"intensity": 0.05, "divider": 1, "warpMode": "directional", "sourceMode": "customNoise"}));

    assert!(moved(&mut s) < 2e-3, "its default mid gray moves nothing");

    input(&mut s, effect, json!({"input": "customNoise", "procedural": "gradient"})).unwrap();

    assert!(moved(&mut s) > 0.01, "a source moves texels: {}", moved(&mut s));

    set(&mut s, effect, json!({"sourceMode": "defaultNoise"}));

    assert_eq!(inputs(&s, effect).len(), 1, "kept while another source mode is on");

    let (_, found) = s.doc.project.effect(effect).unwrap();
    let saved = serde_json::to_value(Effect { inputs: Default::default(), ..found.clone() }).unwrap();

    assert!(saved.get("inputs").is_none(), "no inputs, no key: saves as before");

    let loaded: Effect = serde_json::from_value(saved).unwrap();

    assert!(loaded.inputs.is_empty(), "an effect saved before loads with none");
}
