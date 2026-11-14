//! Layer locks, placeholder effects, the import report and the imported blend modes, through engine commands.

use serde_json::{Value, json};
use texturecraft_engine::core::import_report::original_result_layer;
use texturecraft_engine::core::{
    Channel, Effect, Fidelity, GeometryMask, ImportReport, LayerId, LayerReport, LayerRole, Lock, LockReason, SafetyFlag, SafetyFlagKind,
};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, command_specs, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

/// A paint layer with painted pixels, a painted mask, a content levels effect and a placeholder, then locked; a
/// fill layer below it stays unlocked. Returns (locked paint layer, fill layer, placeholder effect).
fn locked_session() -> (Session, LayerId, LayerId, LayerId) {
    let mut s = session();
    let fill = LayerId(execute(&mut s, "layer.add.fill", &json!({"color": [0.2, 0.3, 0.4]})).unwrap()["layer"].as_u64().unwrap());
    let paint = LayerId(execute(&mut s, "layer.add.paint", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[180, 200], [220, 200]]})).unwrap();
    execute(&mut s, "mask.add.white", &json!({"layer": paint.0})).unwrap();
    execute(&mut s, "effect.add.levels", &json!({"layer": paint.0, "target": "content"})).unwrap();
    let placeholder = s.doc.project.alloc_id();
    let layer = s.doc.project.layer_mut(paint).unwrap();
    layer.content_effects.push(Effect::placeholder(placeholder, "Filter \"warp\"", "compiled graphs are not computed yet"));
    layer.lock = Some(Lock::import(vec![LockReason::new("graph:warp", "content filter \"warp\"")]));
    s.selected.insert(0, paint);
    s.update();
    (s, paint, fill, placeholder)
}

/// What a lock protects: the layer's content view and every pixel image of it and its effects.
fn content(s: &Session, id: LayerId) -> Option<(texturecraft_engine::core::Layer, Vec<Option<texturecraft_engine::texel::TileImage>>)> {
    let l = s.doc.project.layers.get(&id).filter(|_| s.doc.project.locate(id).is_some())?;
    let ids: Vec<LayerId> = std::iter::once(id).chain(l.mask.iter().flat_map(|m| m.effects.iter()).chain(&l.content_effects).map(|e| e.id)).collect();
    let pixels = ids
        .iter()
        .flat_map(|e| Channel::ALL.into_iter().map(|c| s.doc.pixels.layer(*e, c).cloned()).chain(std::iter::once(s.doc.pixels.masks.get(e).cloned())))
        .collect();
    Some((l.content_view(), pixels))
}

#[test]
fn a_locked_layer_refuses_content_edits_and_stays_as_it_was() {
    let (mut s, paint, _, _) = locked_session();
    let before = content(&s, paint).unwrap();
    assert!(before.1.iter().any(Option::is_some), "the fixture has painted pixels");
    let edits = [
        ("tool.stroke", json!({"points": [[150, 180], [250, 220]]})),
        ("mask.invertBackground", json!({"layer": paint.0})),
        ("mask.invert", json!({"layer": paint.0})),
        ("mask.remove", json!({"layer": paint.0})),
        ("effect.add.levels", json!({"layer": paint.0, "target": "content"})),
        ("effect.add.paint", json!({"layer": paint.0, "target": "mask"})),
        ("layer.set", json!({"layer": paint.0, "heightMask": true})),
        ("layer.set", json!({"layer": paint.0, "geometryMask": {"uvTiles": [1001]}})),
        ("layer.flatten", json!({"layer": paint.0})),
    ];
    for (id, p) in edits {
        let err = execute(&mut s, id, &p).expect_err(id).to_string();
        assert!(err.contains("locked"), "{id}: {err}");
        assert_eq!(content(&s, paint).as_ref(), Some(&before), "{id} left the layer as it was");
    }
    // Effects of the locked layer cannot change either.
    let levels = s.doc.project.layer(paint).unwrap().content_effects[0].id;
    for p in [json!({"effect": levels.0, "gamma": 2.0}), json!({"effect": levels.0, "visible": false})] {
        assert!(execute(&mut s, "effect.set", &p).is_err(), "{p}");
    }
    assert!(execute(&mut s, "effect.remove", &json!({"effect": levels.0})).is_err());
    assert_eq!(content(&s, paint), Some(before));
}

#[test]
fn a_locked_layer_can_be_arranged_shown_and_blended_then_unlocked() {
    let (mut s, paint, fill, _) = locked_session();
    let before = content(&s, paint).unwrap();
    for (id, p) in [
        ("layer.rename", json!({"layer": paint.0, "name": "Imported"})),
        ("layer.visible", json!({"layer": paint.0, "visible": false})),
        ("layer.colorTag", json!({"layer": paint.0, "tag": 2})),
        ("layer.blend", json!({"layer": paint.0, "channel": "baseColor", "mode": "Multiply", "opacity": 0.5})),
        ("mask.toggle", json!({"layer": paint.0, "enabled": false})),
        ("layer.move", json!({"layer": paint.0, "index": 0})),
    ] {
        execute(&mut s, id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    assert_eq!(content(&s, paint), Some(before.clone()));
    let l = s.doc.project.layer(paint).unwrap();
    assert_eq!((l.name.as_str(), l.visible, l.color_tag), ("Imported", false, Some(2)));
    // A content edit that is refused is not an undo step.
    let depth = s.history_labels().len();
    assert!(execute(&mut s, "mask.invert", &json!({"layer": paint.0})).is_err());
    assert_eq!(s.history_labels().len(), depth);

    let r = execute(&mut s, "layer.unlock", &json!({"layer": paint.0})).unwrap();
    assert_eq!(r["unlocked"]["source"], "Project import");
    assert!(!s.doc.project.layer(paint).unwrap().is_locked());
    execute(&mut s, "mask.invert", &json!({"layer": paint.0})).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.doc.project.layer(paint).unwrap().is_locked(), "undo brings the lock back");
    assert!(execute(&mut s, "layer.unlock", &json!({"layer": fill.0})).unwrap_err().to_string().contains("not locked"));
    // Deleting a locked layer is allowed.
    execute(&mut s, "layer.remove", &json!({"layer": paint.0})).unwrap();
    assert!(s.doc.project.locate(paint).is_none());
}

#[test]
fn every_command_with_any_params_leaves_a_locked_layer_alone() {
    let params = [
        Value::Null,
        json!({}),
        json!({"layer": 0, "enabled": false, "visible": false, "name": "x", "index": 0}),
        json!({"color": [1.0, 0.0, 0.0], "value": 0.5, "channel": "baseColor", "mode": "Hue", "opacity": 0.3, "procedural": "clouds"}),
        json!({"points": [[150, 180], [250, 220]], "heightMask": true, "geometryMask": ["x"], "target": "mask", "filter": "blur"}),
        json!({"layer": u64::MAX, "effect": u64::MAX, "delta": -1, "above": null, "inside": true, "tag": 1}),
    ];
    // Undo and redo restore history, which here goes back to before the fixture locked the layer.
    let skip = |id: &str| id == "project.sample" || id.starts_with("bake.") || matches!(id, "edit.undo" | "edit.redo" | "edit.history");
    for spec in command_specs().iter().filter(|c| !skip(c.id)) {
        for p in &params {
            let (mut s, paint, _, placeholder) = locked_session();
            let mut p = p.clone();
            if p.get("layer").is_some_and(|v| v == 0) {
                p["layer"] = json!(paint.0);
                p["effect"] = json!(placeholder.0);
            }
            let (project, before) = (s.project_id, content(&s, paint));
            // A refused command leaves the document as it was: only one that ran needs the frame that settles it.
            if execute(&mut s, spec.id, &p).is_ok() {
                s.update();
            }
            if s.project_id != project {
                continue; // another project was opened or created
            }
            if let Some(after) = content(&s, paint) {
                assert_eq!(Some(after), before, "{} {p} changed the locked layer", spec.id);
            }
        }
    }
}

#[test]
fn placeholders_composite_as_identity_and_cannot_be_edited() {
    let (mut s, paint, _, placeholder) = locked_session();
    execute(&mut s, "layer.unlock", &json!({"layer": paint.0})).unwrap();
    s.update();
    let hit = s.pick(Vec2::new(200.0, 200.0), s.viewport).unwrap();
    let (x, y) = ((hit.uv.x * 256.0) as u32, (hit.uv.y * 256.0) as u32);
    let with = s.caches[0].composite.value(Channel::BaseColor, x, y);
    // Even on an unlocked layer a placeholder only hides, moves or goes.
    for p in [json!({"effect": placeholder.0, "name": "renamed"}), json!({"effect": placeholder.0, "filter": "blur"})] {
        assert!(execute(&mut s, "effect.set", &p).is_err(), "{p}");
    }
    execute(&mut s, "effect.set", &json!({"effect": placeholder.0, "visible": false})).unwrap();
    s.update();
    assert_eq!(s.caches[0].composite.value(Channel::BaseColor, x, y), with, "hiding a placeholder changes nothing");
    execute(&mut s, "effect.remove", &json!({"effect": placeholder.0})).unwrap();
    assert!(s.doc.project.effect(placeholder).is_none());
}

#[test]
fn inspect_shows_locks_placeholders_the_report_and_original_results() {
    let (mut s, paint, fill, placeholder) = locked_session();
    let channels = s.doc.project.texture_sets[0].channel_list();
    let original = s.doc.project.alloc_id();
    s.doc.project.insert_layer(0, original_result_layer(original, &channels, false), None, false).unwrap();
    let mut report = ImportReport::new("robot.ext (saved with version 1.2)", "texturecraft-import 0.1.0");
    report.add("texture set \"Body\" / layer stack", Fidelity::Approximated, "2 layers");
    report.layers.insert(original, LayerReport { role: LayerRole::OriginalResult, fidelity: Fidelity::Baked, ..LayerReport::default() });
    let flag = SafetyFlag::new(SafetyFlagKind::UnhandledSetting, "Class.field", "setting Class.field = 2 not handled");
    report.layers.insert(paint, LayerReport { fidelity: Fidelity::Approximated, path: "Rust".into(), flags: vec![flag], ..LayerReport::default() });
    s.doc.project.import = Some(report);
    s.doc.project.layer_mut(fill).unwrap().geometry_mask = Some(GeometryMask::UvTiles(vec![1001]));

    let v = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let layer = |id: LayerId| v["layers"].as_array().unwrap().iter().find(|l| l["id"] == id.0).unwrap().clone();
    let p = layer(paint);
    assert_eq!(p["locked"]["source"], "Project import");
    assert_eq!(p["locked"]["reasons"][0]["key"], "graph:warp");
    assert_eq!(p["import"]["flags"][0]["kind"], "UnhandledSetting");
    assert_eq!(p["contentEffects"][1]["placeholder"]["label"], "Filter \"warp\"");
    assert_eq!(layer(fill)["geometryMask"], json!({"uvTiles": [1001]}));
    let o = layer(original);
    assert_eq!(
        (o["name"].as_str(), o["visible"].as_bool(), o["import"]["role"].as_str()),
        (Some("Original result (reference)"), Some(false), Some("OriginalResult"))
    );
    assert_eq!(v["import"]["originalResults"], json!([original.0]));
    assert_eq!(v["import"]["counts"], json!({"exact": 0, "approximated": 2, "baked": 1}));
    assert_eq!(v["import"]["source"], "robot.ext (saved with version 1.2)");
    assert_eq!(v["locks"]["lockedLayers"].as_array().unwrap().len(), 2);
    assert_eq!(v["locks"]["placeholders"][0]["effect"], placeholder.0);
    // Projects that were not imported show no report.
    assert_eq!(execute(&mut session(), "project.inspect", &Value::Null).unwrap()["import"], Value::Null);
}

#[test]
fn geometry_masks_by_uv_tile_through_layer_set() {
    let mut s = session();
    let fill = execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap()["layer"].as_u64().unwrap();
    let r = execute(&mut s, "layer.set", &json!({"layer": fill, "geometryMask": {"uvTiles": [1002]}})).unwrap();
    assert_eq!(r["geometryMask"], json!({"UvTiles": [1002]}));
    s.update();
    let hit = s.pick(Vec2::new(200.0, 200.0), s.viewport).unwrap();
    let (x, y) = ((hit.uv.x * 256.0) as u32, (hit.uv.y * 256.0) as u32);
    assert!(s.caches[0].composite.value(Channel::BaseColor, x, y)[1] > 0.1, "tile 1002 is not in this set: the fill does not show");
    execute(&mut s, "layer.set", &json!({"layer": fill, "geometryMask": {"uvTiles": [1001]}})).unwrap();
    s.update();
    assert!(s.caches[0].composite.value(Channel::BaseColor, x, y)[1] < 0.01);
    for bad in [json!({"uvTiles": [5]}), json!({"uvTiles": "x"}), json!({"uvTiles": [1001], "other": 1})] {
        assert!(execute(&mut s, "layer.set", &json!({"layer": fill, "geometryMask": bad})).is_err(), "{bad}");
    }
}

#[test]
fn imported_blend_modes_are_set_by_command() {
    let mut s = session();
    let fill = execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap()["layer"].as_u64().unwrap();
    for (mode, stored) in [("Hue", "Hue"), ("HSV tint", "HsvTint"), ("hsvSaturation", "HsvSaturation"), ("HsvColor", "HsvColor"), ("HSV value", "HsvValue")] {
        execute(&mut s, "layer.blend", &json!({"layer": fill, "channel": "baseColor", "mode": mode})).unwrap();
        let b = s.doc.project.layer(LayerId(fill)).unwrap().channels[&Channel::BaseColor];
        assert_eq!(serde_json::to_value(b.mode).unwrap(), json!(stored));
        s.update();
    }
}
