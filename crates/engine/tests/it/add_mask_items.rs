//! The layer toolbar's Add mask items that add a mask with an effect in it, and Add effect's colour selection: one undo step each, the new effect selected

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, CompareSource, EffectKind, LayerId, MaskBackground, Source};
use texturecraft_engine::fill_alpha::resource_name;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::paint::alphas;
use texturecraft_engine::{Session, StackTarget, execute};

fn fill() -> (Session, LayerId) {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
    let layer = LayerId(execute(&mut s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    (s, layer)
}

/// A 2 × 2 mid-gray picture
fn gray_png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 128, 128, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

/// The effect `command` added and selected, and its kind
fn added(s: &mut Session, command: &str, params: Value) -> (LayerId, EffectKind) {
    let steps = s.history_labels().len();
    let effect = LayerId(execute(s, command, &params).unwrap()["effect"].as_u64().unwrap());

    assert_eq!(s.history_labels().len(), steps + 1, "{command} is one undo step");
    assert_eq!(s.target, StackTarget::Effect(effect), "{command} selects the effect it adds");

    let kind = s.doc.project.effect(effect).unwrap().1.kind.clone();
    (effect, kind)
}

#[test]
fn a_bitmap_mask_is_a_black_mask_with_a_fill_reading_the_picked_alpha() {
    let (mut s, layer) = fill();
    let name = resource_name(alphas::find("scratches").unwrap());
    let (_, kind) = added(&mut s, "mask.add.bitmap", json!({"layer": layer.0, "alpha": "scratches"}));

    assert!(matches!(kind, EffectKind::Fill { source: Source::Image { name: ref read, .. }, .. } if *read == name), "{kind:?}");

    let mask = s.doc.project.layer(layer).unwrap().mask.clone().unwrap();

    assert_eq!(mask.background, MaskBackground::Black);
    assert!(s.doc.resources.contains_key(&name));

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert!(s.doc.project.layer(layer).unwrap().mask.is_none(), "one undo takes the mask back");
    assert!(!s.doc.resources.contains_key(&name), "and the alpha's picture with it");

    // Without a picture there is nothing to add
    assert!(execute(&mut s, "mask.add.bitmap", &json!({"layer": layer.0})).is_err());
    assert!(execute(&mut s, "mask.add.bitmap", &json!({"layer": layer.0, "image": "missing.png"})).is_err());

    let image = s.import_resource_bytes("gray.png", gray_png()).unwrap();

    assert!(execute(&mut s, "mask.add.bitmap", &json!({"layer": layer.0, "image": image, "alpha": "scratches"})).is_err(), "one picture, not two");
    assert!(s.doc.project.layer(layer).unwrap().mask.is_none());
}

#[test]
fn a_height_combination_is_a_mask_comparing_this_layers_height_with_the_layers_below() {
    let (mut s, layer) = fill();
    let (_, kind) = added(&mut s, "mask.add.heightCombination", json!({"layer": layer.0}));

    match kind {
        EffectKind::Compare { left, right, channel, hardness, .. } => {
            assert_eq!((left, right, channel, hardness), (CompareSource::ThisLayer, CompareSource::LayersBelow, Channel::Height, 0.9));
        }
        other => panic!("not a compare mask: {other:?}"),
    }
    assert_eq!(s.history_labels().last().map(String::as_str), Some("Add mask with height combination"));
}

#[test]
fn color_selections_go_in_the_mask_and_are_selected() {
    let (mut s, layer) = fill();
    let (first, kind) = added(&mut s, "mask.add.colorSelection", json!({"layer": layer.0}));

    assert!(matches!(kind, EffectKind::ColorSelection(_)));
    assert_eq!(s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().background, MaskBackground::Black, "as colour selection masks were");

    // Add effect's item adds another to the same mask, whatever target it is given
    let (second, kind) = added(&mut s, "effect.add.colorSelection", json!({"layer": layer.0, "target": "content"}));
    assert!(matches!(kind, EffectKind::ColorSelection(_)));

    let mask: Vec<LayerId> = s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects.iter().map(|effect| effect.id).collect();

    assert_eq!(mask, vec![first, second]);
    assert!(s.doc.project.layer(layer).unwrap().content_effects.is_empty());
}

/// An instance with no mask of its own: the Add mask items start one on the instance and leave the source's mask (or its lack of one) alone
#[test]
fn add_mask_on_an_instance_starts_its_own_mask() {
    for source_mask in [None, Some("mask.add.white")] {
        for (command, params) in [
            ("mask.add.heightCombination", json!({})),
            ("mask.add.colorSelection", json!({"color": [1.0, 0.0, 0.0]})),
            ("mask.add.bitmap", json!({"alpha": "scratches"})),
        ] {
            let (mut s, source) = fill();
            if let Some(add) = source_mask {
                execute(&mut s, add, &json!({"layer": source.0})).unwrap();
            }
            let before = s.doc.project.layer(source).unwrap().mask.clone();
            let instance = LayerId(execute(&mut s, "layer.instance", &json!({"layer": source.0})).unwrap()["layer"].as_u64().unwrap());

            let mut params = params;
            params["layer"] = json!(instance.0);
            let (effect, _) = added(&mut s, command, params);

            assert_eq!(s.doc.project.layer(source).unwrap().mask, before, "{command} left the source's mask as it was");

            let own = s.doc.project.layer(instance).unwrap().mask.as_ref().map(|mask| mask.effects.iter().map(|effect| effect.id).collect::<Vec<_>>());

            assert_eq!(own, Some(vec![effect]), "{command} started the instance's own mask");
        }
    }
}
