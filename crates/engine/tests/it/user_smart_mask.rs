//! A layer's mask saved as a user smart mask (`.tcmask`) and applied to other layers and projects.

use serde_json::{Value, json};
use texturecraft_engine::core::{Effect, EffectKind, LayerId, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::smart_masks::SMART_MASK_EXT;
use texturecraft_engine::{Session, execute};
use texturecraft_texel::TileImage;

fn session(resolution: u32) -> Session {
    Session::new_project("t", primitives::sphere(1.0, 24, 12), "builtin:sphere", None, resolution).unwrap()
}

fn layer(s: &mut Session) -> LayerId {
    LayerId(execute(s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap())
}

/// A mask with a generator, a filter, an image fill and painted pixels. Returns the layer and the image's name.
fn multi_effect_mask(s: &mut Session) -> (LayerId, String) {
    let mut png = Vec::new();
    image::RgbaImage::from_fn(8, 8, |x, y| image::Rgba([(x * 30) as u8, (y * 30) as u8, 90, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = s.import_resource_bytes("stencil.png", png).unwrap();

    let id = layer(s);
    execute(s, "mask.add.black", &json!({"layer": id.0})).unwrap();
    execute(s, "effect.add.generator", &json!({"layer": id.0, "generator": "edgeWear"})).unwrap();
    execute(s, "effect.add.filter", &json!({"layer": id.0, "target": "mask", "filter": "invert"})).unwrap();
    let fill = execute(s, "effect.add.fill", &json!({"layer": id.0, "target": "mask"})).unwrap()["effect"].as_u64().unwrap();
    let paint = execute(s, "effect.add.paint", &json!({"layer": id.0, "target": "mask"})).unwrap()["effect"].as_u64().unwrap();

    let mask = s.doc.project.layer_mut(id).unwrap().mask.as_mut().unwrap();
    for effect in &mut mask.effects {
        if effect.id.0 == fill {
            effect.kind = EffectKind::Fill { source: Source::image(&name), projection: Default::default() };
        }
    }
    let size = s.doc.project.texture_set(0).unwrap().resolution;
    let mut pixels = TileImage::new(size, size, 2);
    pixels.set(10, 12, &[40000, 65535]);
    s.doc.pixels.masks.insert(LayerId(paint), pixels);
    (id, name)
}

/// The mask's effects without their ids (those are new in every project).
fn effects(s: &Session, layer: LayerId) -> Vec<Effect> {
    s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects.iter().map(|effect| Effect { id: LayerId(0), ..effect.clone() }).collect()
}

fn painted(s: &Session, layer: LayerId) -> Vec<[u16; 4]> {
    let mask = s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap();
    mask.effects.iter().filter(|effect| matches!(effect.kind, EffectKind::Paint)).map(|effect| s.doc.pixels.masks[&effect.id].get(10, 12)).collect()
}

#[test]
fn a_saved_mask_applies_to_another_layer_with_parameters_pixels_and_images() {
    let mut s = session(256);
    let (source, image) = multi_effect_mask(&mut s);
    let bytes = s.smart_mask_bytes(source, "Worn stencil").unwrap();

    let other = layer(&mut s);
    execute(&mut s, "mask.add.white", &json!({"layer": other.0})).unwrap();
    let before = s.doc.project.layers.len();
    s.apply_smart_mask_bytes(other, &bytes).unwrap();

    assert_eq!(effects(&s, other), effects(&s, source), "every effect keeps its type and parameters");
    assert_eq!(painted(&s, other), painted(&s, source), "and the painted pixels");
    assert_eq!(s.doc.project.layer(other).unwrap().mask.as_ref().unwrap().background, s.doc.project.layer(source).unwrap().mask.as_ref().unwrap().background);
    assert_eq!(s.doc.project.layers.len(), before, "no layer is added");
    let ids: Vec<_> = s.doc.project.layer(other).unwrap().mask.as_ref().unwrap().effects.iter().map(|effect| effect.id).collect();
    assert!(ids.iter().all(|id| !s.doc.project.layer(source).unwrap().mask.as_ref().unwrap().effects.iter().any(|e| e.id == *id)), "the effects are new ones");
    assert!(s.doc.resources.contains_key(&image));
    s.doc.project.validate().unwrap();

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.doc.project.layer(other).unwrap().mask.as_ref().unwrap().effects.len(), 0, "one undo step restores the old mask");
}

#[test]
fn a_saved_mask_applies_in_a_fresh_project_of_another_size() {
    let mut s = session(256);
    let (source, image) = multi_effect_mask(&mut s);
    let bytes = s.smart_mask_bytes(source, "Worn stencil").unwrap();
    let expected = effects(&s, source);

    let mut fresh = session(128);
    let target = layer(&mut fresh);
    assert!(fresh.doc.resources.is_empty());
    fresh.apply_smart_mask_bytes(target, &bytes).unwrap();

    assert_eq!(effects(&fresh, target), expected);
    assert_eq!(fresh.doc.resources.get(&image), s.doc.resources.get(&image), "the image arrives byte for byte");
    assert!(fresh.images.contains_key(&image), "and is decoded for the mask to read");
    let mask = fresh.doc.project.layer(target).unwrap().mask.as_ref().unwrap();
    let paint = mask.effects.iter().find(|effect| matches!(effect.kind, EffectKind::Paint)).unwrap();
    assert_eq!(fresh.doc.pixels.masks[&paint.id].width(), 128, "painted pixels are fitted to the project's size");
    fresh.doc.project.validate().unwrap();
}

#[test]
fn the_commands_write_a_file_and_apply_it_by_path() {
    let mut s = session(128);
    let (source, _) = multi_effect_mask(&mut s);
    let dir = std::env::temp_dir().join(format!("tc_user_smart_mask_{}", std::process::id()));
    let path = dir.join("lib").join("worn");

    let saved = execute(&mut s, "mask.smart.create", &json!({"layer": source.0, "path": path.display().to_string(), "name": "Worn"})).unwrap();
    let file = dir.join("lib").join(format!("worn.{SMART_MASK_EXT}"));
    assert_eq!(saved["path"], file.display().to_string(), "the extension is added and the folder made");

    let other = layer(&mut s);
    execute(&mut s, "mask.smart.apply", &json!({"layer": other.0, "path": file.display().to_string()})).unwrap();
    assert_eq!(effects(&s, other), effects(&s, source));
    std::fs::remove_dir_all(&dir).ok();

    let plain = layer(&mut s);
    assert!(execute(&mut s, "mask.smart.create", &json!({"layer": plain.0, "path": "unused"})).is_err(), "a layer without a mask has nothing to save");
    assert!(execute(&mut s, "mask.smart.apply", &json!({"layer": plain.0, "path": file.display().to_string()})).is_err(), "a missing file changes nothing");
    assert!(s.doc.project.layer(plain).unwrap().mask.is_none());
}

#[test]
fn a_file_without_a_mask_is_refused_and_a_disabled_mask_comes_back_on() {
    let mut s = session(128);
    let plain = layer(&mut s);
    let material = s.smart_material_bytes(plain, "Plain").unwrap();
    assert!(s.apply_smart_mask_bytes(plain, &material).is_err(), "a smart material file has no mask to take");
    assert!(s.apply_smart_mask_bytes(plain, b"junk").is_err());

    let (source, _) = multi_effect_mask(&mut s);
    s.doc.project.layer_mut(source).unwrap().mask.as_mut().unwrap().enabled = false;
    let bytes = s.smart_mask_bytes(source, "Off").unwrap();
    s.apply_smart_mask_bytes(plain, &bytes).unwrap();
    assert!(s.doc.project.layer(plain).unwrap().mask.as_ref().unwrap().enabled);
}
