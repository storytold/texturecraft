//! Add bitmap mask, copy and paste a mask between layers, and flatten all instances: each is one
//! undo step.

use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, LayerId, LayerKind, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// Two texture sets (ball and base), so an instance can live in the second.
fn session() -> Session {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    s.update();
    s
}

fn fill(s: &mut Session) -> LayerId {
    LayerId(execute(s, "layer.add.fill", &json!({"roughness": 0.8})).unwrap()["layer"].as_u64().unwrap())
}

fn import_png(s: &mut Session, name: &str) {
    let mut bytes = Vec::new();
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255]));
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    s.import_resource_bytes(name, bytes).unwrap();
}

fn has_mask(s: &Session, id: LayerId) -> bool {
    s.doc.project.layer(id).unwrap().mask.is_some()
}

#[test]
fn a_bitmap_mask_reads_an_image_resource_and_undoes() {
    let mut s = session();
    let layer = fill(&mut s);
    import_png(&mut s, "wear.png");
    execute(&mut s, "mask.add.bitmap", &json!({"layer": layer.0, "image": "wear.png"})).unwrap();
    let mask = s.doc.project.layer(layer).unwrap().mask.clone().unwrap();
    assert_eq!(mask.effects.len(), 1);
    assert!(matches!(&mask.effects[0].kind, EffectKind::Fill { source: Source::Image { name, .. }, .. } if name == "wear.png"));

    // A second mask is refused and changes nothing
    assert!(execute(&mut s, "mask.add.bitmap", &json!({"layer": layer.0, "image": "wear.png"})).is_err());
    assert_eq!(s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects.len(), 1);

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(!has_mask(&s, layer), "one undo removes the mask");
}

#[test]
fn a_bitmap_mask_needs_an_imported_image() {
    let mut s = session();
    let layer = fill(&mut s);
    assert!(execute(&mut s, "mask.add.bitmap", &json!({"layer": layer.0, "image": "nothing.png"})).is_err());
    assert!(execute(&mut s, "mask.add.bitmap", &json!({"layer": layer.0})).is_err());
    assert!(!has_mask(&s, layer));
}

#[test]
fn a_copied_mask_pastes_into_another_layer_and_undoes() {
    let mut s = session();
    let from = fill(&mut s);
    let to = fill(&mut s);
    execute(&mut s, "mask.add.black", &json!({"layer": from.0})).unwrap();
    execute(&mut s, "effect.add.fill", &json!({"layer": from.0, "value": 0.5})).unwrap();
    let source = s.doc.project.layer(from).unwrap().mask.clone().unwrap();

    assert!(execute(&mut s, "mask.paste", &json!({"layer": to.0})).is_err(), "nothing copied yet");
    execute(&mut s, "mask.copy", &json!({"layer": from.0})).unwrap();
    execute(&mut s, "mask.paste", &json!({"layer": to.0})).unwrap();

    let pasted = s.doc.project.layer(to).unwrap().mask.clone().unwrap();
    assert_eq!(pasted.background, source.background);
    assert_eq!(pasted.effects.len(), source.effects.len());
    assert!(pasted.effects.iter().all(|e| source.effects.iter().all(|o| o.id != e.id)), "the copy has its own effect ids");
    assert!(has_mask(&s, from), "the original stays");

    // Pasting again replaces the mask instead of stacking
    execute(&mut s, "mask.paste", &json!({"layer": to.0})).unwrap();
    assert_eq!(s.doc.project.layer(to).unwrap().mask.as_ref().unwrap().effects.len(), 1);

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(!has_mask(&s, to), "undo takes the pasted mask away");
}

#[test]
fn a_copied_mask_keeps_its_painted_pixels() {
    let mut s = session();
    let from = fill(&mut s);
    let to = fill(&mut s);
    execute(&mut s, "mask.add.black", &json!({"layer": from.0})).unwrap();
    let eid = LayerId(execute(&mut s, "effect.add.paint", &json!({"layer": from.0})).unwrap()["effect"].as_u64().unwrap());
    let size = s.doc.project.texture_set(0).unwrap().size();
    s.edit("Paint", |s| {
        s.doc.pixels.mask_mut(eid, size);
        Ok(())
    })
    .unwrap();
    execute(&mut s, "mask.copy", &json!({"layer": from.0})).unwrap();
    execute(&mut s, "mask.paste", &json!({"layer": to.0})).unwrap();
    let new = s.doc.project.layer(to).unwrap().mask.as_ref().unwrap().effects[0].id;
    assert!(s.doc.pixels.masks.contains_key(&eid) && s.doc.pixels.masks.contains_key(&new), "the paint comes along");
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(!s.doc.pixels.masks.contains_key(&new), "undo drops the pasted pixels");
}

#[test]
fn copying_a_layer_without_a_mask_is_refused() {
    let mut s = session();
    let layer = fill(&mut s);
    assert!(execute(&mut s, "mask.copy", &json!({"layer": layer.0})).is_err());
    assert!(s.mask_clipboard.is_none());
}

#[test]
fn mask_copy_and_paste_default_to_ctrl_alt_c_and_v() {
    let keys: Vec<_> =
        texturecraft_engine::command_specs().iter().filter(|c| c.id.starts_with("mask.") && c.shortcut.is_some()).map(|c| (c.id, c.shortcut)).collect();
    assert!(keys.contains(&("mask.copy", Some("Ctrl+Alt+C"))));
    assert!(keys.contains(&("mask.paste", Some("Ctrl+Alt+V"))));
}

#[test]
fn flatten_all_instances_flattens_the_layer_and_its_instances_in_one_step() {
    let mut s = session();
    let source = fill(&mut s);
    execute(&mut s, "mask.add.white", &json!({"layer": source.0})).unwrap();
    let a = LayerId(execute(&mut s, "layer.instance", &json!({"layer": source.0, "ts": 1})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(execute(&mut s, "layer.instance", &json!({"layer": source.0, "ts": 0})).unwrap()["layer"].as_u64().unwrap());
    let other = fill(&mut s);

    execute(&mut s, "layer.flattenAll", &json!({"layer": source.0})).unwrap();
    for id in [source, a, b] {
        let l = s.doc.project.layer(id).unwrap();
        assert!(matches!(l.kind, LayerKind::Paint), "layer {} is flat", id.0);
        assert!(l.mask.is_none());
    }
    assert!(matches!(s.doc.project.layer(other).unwrap().kind, LayerKind::Fill(_)), "other layers stay as they are");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(matches!(s.doc.project.layer(source).unwrap().kind, LayerKind::Fill(_)), "one undo restores the source");
    assert!(matches!(s.doc.project.layer(a).unwrap().kind, LayerKind::Instance { .. }), "and the instance in the other set");
    assert!(matches!(s.doc.project.layer(b).unwrap().kind, LayerKind::Instance { .. }), "and the one in the same set");
    assert!(has_mask(&s, source));
}

#[test]
fn flatten_all_from_an_instance_starts_at_its_source() {
    let mut s = session();
    let source = fill(&mut s);
    let inst = LayerId(execute(&mut s, "layer.instance", &json!({"layer": source.0, "ts": 1})).unwrap()["layer"].as_u64().unwrap());
    execute(&mut s, "layer.flattenAll", &json!({"layer": inst.0})).unwrap();
    assert!(matches!(s.doc.project.layer(source).unwrap().kind, LayerKind::Paint));
    assert!(matches!(s.doc.project.layer(inst).unwrap().kind, LayerKind::Paint));
}
