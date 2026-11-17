//! Clone and Smudge strokes on a layer mask read and write the mask's own values

use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const SIZE: u32 = 256;

/// A paint layer with a black mask selected, a 2D-view brush of about 13 texels radius
fn session_on_mask() -> (Session, u64) {
    let mut session = Session::new_project("mask", primitives::plane(2.0, 4), "builtin:plane", None, SIZE).unwrap();
    session.update();
    let layer = execute(&mut session, "layer.add.paint", &json!({})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({"layer": layer})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": layer, "target": "mask"})).unwrap();
    execute(&mut session, "tool.brush", &json!({"size": 10, "hardness": 100, "spacing": 10})).unwrap();
    execute(&mut session, "tool.material", &json!({"maskValue": 1.0})).unwrap();
    (session, layer)
}

fn paint_white_blob(session: &mut Session, uv: [f32; 2]) {
    execute(session, "tool.paint", &serde_json::Value::Null).unwrap();
    execute(session, "tool.strokeUv", &json!({"points": [uv, uv]})).unwrap();
}

fn mask_at(session: &Session, layer: u64, uv: [f32; 2]) -> f32 {
    let texel = ((uv[0] * SIZE as f32) as u32, (uv[1] * SIZE as f32) as u32);
    session.mask_at(texturecraft_engine::core::LayerId(layer), &[texel]).unwrap()[0]
}

/// A stroke of `tool` that changes the mask next to the white blob at u = 0.25: clone copies the
/// blob to u = 0.75, smudge drags it to the right
fn stroke_with(session: &mut Session, tool: &str) {
    execute(session, tool, &serde_json::Value::Null).unwrap();
    execute(session, "tool.cloneSource", &json!({"uv": [0.25, 0.5]})).unwrap();
    let points = if tool == "tool.clone" { json!([[0.75, 0.5], [0.75, 0.5]]) } else { json!([[0.25, 0.5], [0.3, 0.5], [0.35, 0.5], [0.4, 0.5], [0.45, 0.5]]) };
    execute(session, "tool.strokeUv", &json!({"points": points})).unwrap();
}

/// Mask values at probes inside the blob, where smudge drags it and where clone copies it
const PROBES: [[f32; 2]; 3] = [[0.25, 0.5], [0.4, 0.5], [0.75, 0.5]];

#[test]
fn clone_on_a_mask_copies_the_mask_values_from_the_source() {
    let (mut session, layer) = session_on_mask();
    paint_white_blob(&mut session, [0.25, 0.5]);
    assert!(mask_at(&session, layer, [0.25, 0.5]) > 0.95);
    assert!(mask_at(&session, layer, [0.75, 0.5]) < 0.05);

    execute(&mut session, "tool.clone", &serde_json::Value::Null).unwrap();
    execute(&mut session, "tool.cloneSource", &json!({"uv": [0.25, 0.5]})).unwrap();
    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.75, 0.5], [0.75, 0.5]]})).unwrap();

    assert!(mask_at(&session, layer, [0.75, 0.5]) > 0.95, "the white blob is cloned");
    assert!(mask_at(&session, layer, [0.75, 0.8]) < 0.05, "only the stroke is painted");
    assert!(mask_at(&session, layer, [0.25, 0.5]) > 0.95, "the source stays");
}

#[test]
fn clone_on_a_mask_copies_black_over_white_too() {
    let (mut session, layer) = session_on_mask();
    paint_white_blob(&mut session, [0.75, 0.5]);

    execute(&mut session, "tool.clone", &serde_json::Value::Null).unwrap();
    execute(&mut session, "tool.cloneSource", &json!({"uv": [0.25, 0.5]})).unwrap();
    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.75, 0.5], [0.75, 0.5]]})).unwrap();

    assert!(mask_at(&session, layer, [0.75, 0.5]) < 0.05, "the black source erases the white blob");
}

#[test]
fn smudge_on_a_mask_drags_values_along_the_stroke() {
    let (mut session, layer) = session_on_mask();
    paint_white_blob(&mut session, [0.25, 0.5]);
    assert!(mask_at(&session, layer, [0.4, 0.5]) < 0.05);

    execute(&mut session, "tool.smudge", &serde_json::Value::Null).unwrap();
    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.25, 0.5], [0.3, 0.5], [0.35, 0.5], [0.4, 0.5], [0.45, 0.5]]})).unwrap();

    assert!(mask_at(&session, layer, [0.4, 0.5]) > 0.3, "white is dragged to the right");
    assert!(mask_at(&session, layer, [0.4, 0.8]) < 0.05, "nothing beside the stroke");
}

#[test]
fn a_clone_or_smudge_stroke_on_a_mask_leaves_the_channels_alone() {
    for tool in ["tool.clone", "tool.smudge"] {
        let (mut session, _) = session_on_mask();
        paint_white_blob(&mut session, [0.25, 0.5]);
        let channels_before = session.doc.pixels.layers.clone();
        let colour_before = session.caches[0].composite.value(Channel::BaseColor, 192, 128);

        stroke_with(&mut session, tool);
        session.update();

        assert_eq!(session.doc.pixels.layers.len(), channels_before.len(), "{tool}: no channel image is created");
        assert!(session.doc.pixels.layers.keys().all(|key| channels_before.contains_key(key)), "{tool}");
        assert_eq!(session.caches[0].composite.value(Channel::BaseColor, 192, 128), colour_before, "{tool}: the composite colour is unchanged");
    }
}

#[test]
fn one_undo_restores_the_mask_after_a_clone_or_smudge_stroke() {
    for tool in ["tool.clone", "tool.smudge"] {
        let (mut session, layer) = session_on_mask();
        paint_white_blob(&mut session, [0.25, 0.5]);
        let steps = session.history_labels().len();
        let before: Vec<f32> = PROBES.iter().map(|uv| mask_at(&session, layer, *uv)).collect();

        stroke_with(&mut session, tool);

        assert_eq!(session.history_labels().len(), steps + 1, "{tool}: one undo step per stroke");
        let after: Vec<f32> = PROBES.iter().map(|uv| mask_at(&session, layer, *uv)).collect();
        assert_ne!(before, after, "{tool}: the stroke changed the mask");

        session.undo().unwrap();

        let restored: Vec<f32> = PROBES.iter().map(|uv| mask_at(&session, layer, *uv)).collect();
        assert_eq!(restored, before, "{tool}");
    }
}

#[test]
fn clone_works_on_a_folder_mask() {
    let mut session = Session::new_project("mask", primitives::plane(2.0, 4), "builtin:plane", None, SIZE).unwrap();
    session.update();
    let folder = execute(&mut session, "layer.add.group", &json!({})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut session, "mask.add.black", &json!({"layer": folder})).unwrap();
    execute(&mut session, "layer.select", &json!({"layer": folder, "target": "mask"})).unwrap();
    execute(&mut session, "tool.brush", &json!({"size": 10, "hardness": 100, "spacing": 10})).unwrap();
    paint_white_blob(&mut session, [0.25, 0.5]);

    execute(&mut session, "tool.clone", &serde_json::Value::Null).unwrap();
    execute(&mut session, "tool.cloneSource", &json!({"uv": [0.25, 0.5]})).unwrap();
    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.75, 0.5], [0.75, 0.5]]})).unwrap();

    assert!(mask_at(&session, folder, [0.75, 0.5]) > 0.95, "a folder mask clones too");
}

#[test]
fn clone_from_the_layer_reads_the_mask_on_a_folder_mask_and_a_paint_layer_mask() {
    for group in [true, false] {
        let mut session = Session::new_project("mask", primitives::plane(2.0, 4), "builtin:plane", None, SIZE).unwrap();
        session.update();
        let command = if group { "layer.add.group" } else { "layer.add.paint" };
        let layer = execute(&mut session, command, &json!({})).unwrap()["layer"].as_u64().unwrap();
        execute(&mut session, "mask.add.black", &json!({"layer": layer})).unwrap();
        execute(&mut session, "layer.select", &json!({"layer": layer, "target": "mask"})).unwrap();
        execute(&mut session, "tool.brush", &json!({"size": 10, "hardness": 100, "spacing": 10, "cloneSource": "layer"})).unwrap();
        paint_white_blob(&mut session, [0.25, 0.5]);

        execute(&mut session, "tool.clone", &serde_json::Value::Null).unwrap();
        execute(&mut session, "tool.cloneSource", &json!({"uv": [0.25, 0.5]})).unwrap();
        execute(&mut session, "tool.strokeUv", &json!({"points": [[0.75, 0.5], [0.75, 0.5]]})).unwrap();

        assert!(mask_at(&session, layer, [0.75, 0.5]) > 0.95, "group {group}");
    }
}
