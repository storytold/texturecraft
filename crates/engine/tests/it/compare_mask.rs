//! Compare masks: one channel of the layer, the layers below, the mask so far or a constant on each side

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const SIZE: u32 = 256;
const BELOW_COLOR: [f32; 3] = [0.0, 0.0, 1.0];
const LAYER_COLOR: [f32; 3] = [1.0, 0.0, 0.0];

fn session() -> Session {
    let mut session = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, SIZE).unwrap();
    session.viewport = Vec2::new(400.0, 400.0);
    session.camera.yaw = 0.0;
    session.camera.pitch = 0.0;
    session.update();
    session
}

/// The texel under the viewport centre
fn center_texel(session: &mut Session) -> (u32, u32) {
    session.update();
    let hit = session.pick(Vec2::new(200.0, 200.0), session.viewport).unwrap();
    ((hit.uv.x * SIZE as f32) as u32, (hit.uv.y * SIZE as f32) as u32)
}

fn base_color(session: &mut Session) -> [f32; 3] {
    let (x, y) = center_texel(session);
    session.caches[0].composite.value(Channel::BaseColor, x, y)
}

fn mask(session: &mut Session, layer: LayerId) -> f32 {
    let texel = center_texel(session);
    session.mask_at(layer, &[texel]).unwrap()[0]
}

fn id(value: Value, key: &str) -> u64 {
    value[key].as_u64().unwrap()
}

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 2e-3
}

/// A blue fill of `below` roughness, a red fill of `own` roughness over it with a default compare mask on roughness: (below, layer, compare)
fn two_layers(session: &mut Session, below: f32, own: f32) -> (LayerId, LayerId, LayerId) {
    let bottom = id(execute(session, "layer.add.fill", &json!({"color": BELOW_COLOR, "roughness": below})).unwrap(), "layer");
    let top = id(execute(session, "layer.add.fill", &json!({"color": LAYER_COLOR, "roughness": own})).unwrap(), "layer");
    let compare = id(execute(session, "effect.add.compare", &json!({"layer": top, "compareChannel": "roughness"})).unwrap(), "effect");
    (LayerId(bottom), LayerId(top), LayerId(compare))
}

#[test]
fn this_layer_is_compared_with_the_layers_below() {
    let mut session = session();
    let (bottom, top, compare) = two_layers(&mut session, 0.2, 0.6);
    assert!(close(mask(&mut session, top), 1.0), "0.6 > 0.2 holds: {}", mask(&mut session, top));
    assert_eq!(base_color(&mut session), LAYER_COLOR, "the layer shows where its mask compares true");

    execute(&mut session, "effect.set", &json!({"effect": compare.0, "op": "lesser"})).unwrap();

    assert!(close(mask(&mut session, top), 0.0));
    assert_eq!(base_color(&mut session), BELOW_COLOR, "the layer is hidden where the comparison fails");

    // Changing the layers below changes the mask above them (no stale cached stack)
    execute(&mut session, "layer.fill.set", &json!({"layer": bottom.0, "channel": "roughness", "value": 0.9})).unwrap();

    assert!(close(mask(&mut session, top), 1.0), "0.6 < 0.9 holds: {}", mask(&mut session, top));
    assert_eq!(base_color(&mut session), LAYER_COLOR);
}

#[test]
fn hardness_ramps_the_mask_past_the_edge() {
    let mut session = session();

    // 0.6 is 0.03 past 0.57: a third of the default hardness 0.9's 0.1 ramp
    let (_, top, compare) = two_layers(&mut session, 0.57, 0.6);
    assert!(close(mask(&mut session, top), 0.3), "{}", mask(&mut session, top));

    execute(&mut session, "effect.set", &json!({"effect": compare.0, "hardness": 1.0})).unwrap();

    assert!(close(mask(&mut session, top), 1.0), "a hard step: {}", mask(&mut session, top));

    execute(&mut session, "effect.set", &json!({"effect": compare.0, "op": "within", "tolerance": 0.02, "hardness": 0.9})).unwrap();

    assert!(close(mask(&mut session, top), 0.9), "0.01 past the tolerance of 0.1's ramp: {}", mask(&mut session, top));
}

#[test]
fn a_constant_side_and_the_mask_so_far() {
    let mut session = session();
    let (_, top, compare) = two_layers(&mut session, 0.2, 0.6);
    execute(&mut session, "effect.set", &json!({"effect": compare.0, "right": "constant", "value": 0.7, "hardness": 1.0})).unwrap();

    assert!(close(mask(&mut session, top), 0.0), "0.6 > 0.7 fails");

    execute(&mut session, "effect.set", &json!({"effect": compare.0, "left": "thisMask", "value": 0.5})).unwrap();

    assert!(close(mask(&mut session, top), 1.0), "a white mask (1) > 0.5 holds");

    let inspected = execute(&mut session, "project.inspect", &Value::Null).unwrap().to_string();
    for part in ["\"left\":\"thisMask\"", "\"op\":\"greater\"", "\"right\":\"constant\"", "\"compareChannel\":\"roughness\"", "\"hardness\":1.0"] {
        assert!(inspected.contains(part), "inspect shows {part}");
    }
}

#[test]
fn compare_parameters_are_one_undo_step_and_refuse_unknown_values() {
    let mut session = session();
    let (_, top, compare) = two_layers(&mut session, 0.2, 0.6);
    execute(&mut session, "effect.set", &json!({"effect": compare.0, "op": "lesser", "right": "constant", "value": 0.9})).unwrap();

    assert!(close(mask(&mut session, top), 1.0), "0.6 < 0.9");

    execute(&mut session, "edit.undo", &Value::Null).unwrap();

    assert!(close(mask(&mut session, top), 1.0), "back to 0.6 > layers below 0.2");

    let inspected = execute(&mut session, "project.inspect", &Value::Null).unwrap().to_string();

    assert!(inspected.contains("\"op\":\"greater\"") && inspected.contains("\"right\":\"layersBelow\""), "one undo step restores both");

    assert!(execute(&mut session, "effect.set", &json!({"effect": compare.0, "left": "elsewhere"})).is_err());
    assert!(execute(&mut session, "effect.set", &json!({"effect": compare.0, "compareChannel": "nope"})).is_err());
    assert!(execute(&mut session, "effect.add.compare", &json!({"layer": top.0, "target": "content"})).is_err(), "compare masks go on masks");
}

#[test]
fn the_histogram_counts_what_each_side_reads() {
    let mut session = session();
    let (_, _, compare) = two_layers(&mut session, 0.25, 0.65);
    let histogram = execute(&mut session, "effect.histogram", &json!({"effect": compare.0, "bins": 10})).unwrap();
    let busiest = |side: &str| {
        let counts: Vec<u64> = histogram["compare"][side].as_array().unwrap().iter().map(|count| count.as_u64().unwrap()).collect();

        assert!(counts.iter().sum::<u64>() > 0, "{side} counts texels");
        counts.iter().enumerate().max_by_key(|(_, count)| **count).map(|(bucket, _)| bucket).unwrap()
    };

    assert_eq!(busiest("left"), 6, "this layer's roughness 0.65");
    assert_eq!(busiest("right"), 2, "the layers below's roughness 0.25");
}
