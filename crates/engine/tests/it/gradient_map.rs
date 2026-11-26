//! Gradient maps on a fill's base color: color stops for a procedural or an anchor point,
//! through `layer.fill.set {colors}`, undo, inspect and a project round trip.

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId, LayerKind, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

/// Composited value of `c` under the viewport center.
fn center(s: &mut Session, c: Channel) -> [f32; 3] {
    s.update();
    let hit = s.pick(texturecraft_engine::geom::glam::Vec2::new(200.0, 200.0), s.viewport).unwrap();
    s.caches[0].composite.value(c, (hit.uv.x * 256.0) as u32, (hit.uv.y * 256.0) as u32)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 2e-3)
}

fn id(v: Value, key: &str) -> u64 {
    v[key].as_u64().unwrap()
}

/// The stops on `layer`'s base color source.
fn ramp(s: &Session, layer: u64) -> Vec<[f32; 3]> {
    match &s.doc.project.layer(LayerId(layer)).unwrap().kind {
        LayerKind::Fill(fill) => fill.source(Channel::BaseColor).ramp().to_vec(),
        _ => panic!("not a fill layer"),
    }
}

const RGB: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[test]
fn an_anchor_gradient_map_recolors_the_gray_below_and_undoes() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5]})).unwrap();
    let anchor = id(execute(&mut s, "effect.add.anchor", &json!({})).unwrap(), "effect");
    let top = id(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0], "roughness": 0.3, "metallic": 0.6})).unwrap(), "layer");
    execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "baseColor", "anchor": anchor})).unwrap();
    assert!(close(center(&mut s, Channel::BaseColor), [0.5; 3]), "{:?}", center(&mut s, Channel::BaseColor));

    // Mid gray picks the middle stop; the other channels keep their values
    execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "baseColor", "colors": RGB})).unwrap();
    assert!(close(center(&mut s, Channel::BaseColor), [0.0, 1.0, 0.0]), "{:?}", center(&mut s, Channel::BaseColor));
    assert!(close(center(&mut s, Channel::Roughness), [0.3; 3]));
    assert!(close(center(&mut s, Channel::Metallic), [0.6; 3]));
    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == top).unwrap().clone();
    assert_eq!(row["fill"]["sources"]["baseColor"]["colors"], json!(RGB), "{row}");

    // One undo step each way
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(ramp(&s, top).is_empty());
    assert!(close(center(&mut s, Channel::BaseColor), [0.5; 3]));
    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    assert_eq!(ramp(&s, top), RGB.to_vec());
    assert!(close(center(&mut s, Channel::BaseColor), [0.0, 1.0, 0.0]));

    // Picking the anchor again keeps the stops; hex colors work; an empty list removes them
    execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "baseColor", "colors": ["#000000", "#ff0000"]})).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "baseColor", "anchor": anchor})).unwrap();
    assert!(close(center(&mut s, Channel::BaseColor), [0.5, 0.0, 0.0]), "{:?}", center(&mut s, Channel::BaseColor));
    execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "baseColor", "colors": []})).unwrap();
    assert!(ramp(&s, top).is_empty());
    assert!(close(center(&mut s, Channel::BaseColor), [0.5; 3]));
}

#[test]
fn a_procedural_gradient_map_saves_and_loads() {
    let mut s = session();
    let layer = id(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0], "roughness": 0.4})).unwrap(), "layer");
    execute(&mut s, "layer.fill.set", &json!({"layer": layer, "channel": "baseColor", "procedural": "Checker", "low": [0.2, 0.2, 0.2], "high": [0.8, 0.8, 0.8], "colors": [[1, 0, 0], [0, 0, 1]]})).unwrap();
    let value = center(&mut s, Channel::BaseColor);
    assert!(close(value, [1.0, 0.0, 0.0]) || close(value, [0.0, 0.0, 1.0]), "a stop, not low or high: {value:?}");
    assert!(close(center(&mut s, Channel::Roughness), [0.4; 3]));

    // Changing the pattern's settings without `colors` (as its sliders do) keeps the stops
    execute(
        &mut s,
        "layer.fill.set",
        &json!({"layer": layer, "channel": "baseColor", "procedural": "Checker", "scale": 8.0, "low": [0.2, 0.2, 0.2], "high": [0.8, 0.8, 0.8]}),
    )
    .unwrap();
    assert_eq!(ramp(&s, layer), vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);

    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert_eq!(back.doc.project, s.doc.project);
    assert_eq!(ramp(&back, layer), vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
    match &back.doc.project.layer(LayerId(layer)).unwrap().kind {
        LayerKind::Fill(fill) => assert!(matches!(fill.source(Channel::BaseColor), Source::Procedural { .. })),
        _ => panic!("not a fill layer"),
    }
}

#[test]
fn gradient_map_colors_are_refused_where_they_cannot_apply() {
    let mut s = session();
    let layer = id(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0], "roughness": 0.4})).unwrap(), "layer");
    let set = |s: &mut Session, extra: Value| {
        let mut p = json!({"layer": layer});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        execute(s, "layer.fill.set", &p)
    };
    assert!(set(&mut s, json!({"channel": "baseColor", "colors": RGB})).is_err(), "a uniform color has no gray to map");
    assert!(set(&mut s, json!({"channel": "roughness", "procedural": "Clouds", "colors": RGB})).is_err(), "only color channels");
    assert!(set(&mut s, json!({"channel": "baseColor", "procedural": "Clouds", "colors": vec![[0.5; 3]; 17]})).is_err(), "at most 16 stops");
    assert!(set(&mut s, json!({"channel": "baseColor", "procedural": "Clouds", "colors": ["red"]})).is_err());
    assert!(ramp(&s, layer).is_empty(), "a refused edit changes nothing");

    // Removing stops where there are none (a uniform color) changes nothing either
    let before = s.doc.project.clone();
    set(&mut s, json!({"channel": "baseColor", "colors": []})).unwrap();
    assert_eq!(s.doc.project, before);
    set(&mut s, json!({"channel": "baseColor", "procedural": "Clouds", "colors": [[2.0, -1.0, 0.5]]})).unwrap();
    assert_eq!(ramp(&s, layer), vec![[1.0, 0.0, 0.5]], "stops clamp to 0..1");
}

#[test]
fn a_scalar_channel_drops_stuck_stops_when_its_source_is_set() {
    let mut s = session();
    let layer = id(execute(&mut s, "layer.add.fill", &json!({"roughness": 0.4})).unwrap(), "layer");

    // Replacing a scalar source must discard color stops hidden by the panel
    let stuck = |s: &mut Session| {
        let LayerKind::Fill(fill) = &mut s.doc.project.layer_mut(LayerId(layer)).unwrap().kind else { panic!("not a fill layer") };
        let pattern = texturecraft_engine::core::Procedural::new(texturecraft_engine::core::ProceduralKind::Clouds);
        fill.sources.insert(Channel::Roughness, Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: RGB.to_vec() });
    };
    let roughness_ramp = |s: &Session| match &s.doc.project.layer(LayerId(layer)).unwrap().kind {
        LayerKind::Fill(fill) => fill.source(Channel::Roughness).ramp().to_vec(),
        _ => panic!("not a fill layer"),
    };
    stuck(&mut s);
    execute(&mut s, "layer.fill.set", &json!({"layer": layer, "channel": "roughness", "procedural": "Clouds", "scale": 8.0})).unwrap();
    assert!(roughness_ramp(&s).is_empty(), "a new pattern on roughness keeps no stops");
    stuck(&mut s);
    execute(&mut s, "layer.fill.set", &json!({"layer": layer, "channel": "roughness", "colors": []})).unwrap();
    assert!(roughness_ramp(&s).is_empty(), "`colors: []` removes them");
}

#[test]
fn removing_stops_from_an_absent_channel_adds_no_channel() {
    let mut s = session();

    // A content fill effect that supplies only roughness: a default gray base color would cover the layer
    let effect = id(execute(&mut s, "effect.add.fill", &json!({"target": "content", "roughness": 0.7})).unwrap(), "effect");
    let before = s.doc.project.clone();
    execute(&mut s, "layer.fill.set", &json!({"effect": effect, "channel": "baseColor", "colors": []})).unwrap();
    assert_eq!(s.doc.project, before, "no base color source appears on the effect");

    // The same on a fill layer that has no base color channel
    let layer = id(execute(&mut s, "layer.add.fill", &json!({"roughness": 0.4})).unwrap(), "layer");
    let before = s.doc.project.clone();
    execute(&mut s, "layer.fill.set", &json!({"layer": layer, "channel": "baseColor", "colors": []})).unwrap();
    assert_eq!(s.doc.project, before, "no base color channel appears on the layer");

    // Turning the channel on still adds it
    execute(&mut s, "layer.fill.set", &json!({"effect": effect, "channel": "baseColor", "enabled": true})).unwrap();
    let (_, x) = s.doc.project.effect(texturecraft_engine::core::LayerId(effect)).unwrap();
    match &x.kind {
        texturecraft_engine::core::EffectKind::Material(f) => assert!(f.sources.contains_key(&Channel::BaseColor), "enabled adds the channel"),
        _ => panic!("not a content fill effect"),
    }
}

#[test]
fn gradient_map_true_starts_the_stops_from_the_current_source() {
    let mut s = session();
    let layer = id(execute(&mut s, "layer.add.fill", &json!({})).unwrap(), "layer");
    let set = |s: &mut Session, p: Value| {
        let mut p = p;
        p["layer"] = json!(layer);
        p["channel"] = json!("baseColor");
        execute(s, "layer.fill.set", &p)
    };

    // A uniform color has nothing to start from
    assert!(set(&mut s, json!({"gradientMap": true})).is_err());

    // A procedural starts at low, halfway and high, so its look holds
    set(&mut s, json!({"procedural": "Clouds", "low": [0.0, 0.25, 0.5], "high": [1.0, 0.75, 0.5]})).unwrap();
    set(&mut s, json!({"gradientMap": true})).unwrap();
    assert_eq!(ramp(&s, layer), vec![[0.0, 0.25, 0.5], [0.5; 3], [1.0, 0.75, 0.5]]);

    // Stops already there stay
    set(&mut s, json!({"colors": RGB})).unwrap();
    set(&mut s, json!({"gradientMap": true})).unwrap();
    assert_eq!(ramp(&s, layer), RGB.to_vec());

    // Set together with a new pattern, the stops stay
    set(&mut s, json!({"procedural": "Voronoi", "gradientMap": true})).unwrap();
    assert_eq!(ramp(&s, layer), RGB.to_vec());

    // It cannot go with `colors`, and it undoes in one step
    assert!(set(&mut s, json!({"gradientMap": true, "colors": RGB})).is_err());
    set(&mut s, json!({"colors": []})).unwrap();
    set(&mut s, json!({"gradientMap": true})).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(ramp(&s, layer).is_empty());

    // An anchor starts at black, gray and white
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    let anchor = id(execute(&mut s, "effect.add.anchor", &json!({})).unwrap(), "effect");
    let top = id(execute(&mut s, "layer.add.fill", &json!({})).unwrap(), "layer");
    execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "baseColor", "anchor": anchor})).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "baseColor", "gradientMap": true})).unwrap();
    assert_eq!(ramp(&s, top), vec![[0.0; 3], [0.5; 3], [1.0; 3]]);

    // A scalar channel has no stops
    assert!(execute(&mut s, "layer.fill.set", &json!({"layer": top, "channel": "roughness", "gradientMap": true})).is_err());
}
