//! A simple shelf material can be the brush's material (`tool.material {material}`): no layer, and the
//! next stroke paints it. Shelf drops place new layers with `below` and `top`.

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::materials::{MATERIALS, brush_values};
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

fn value_of(material: &str, channel: Channel) -> [f32; 3] {
    brush_values(material).unwrap().into_iter().find(|(c, _)| *c == channel).map(|(_, v)| v).unwrap()
}

fn stack(s: &Session) -> Vec<LayerId> {
    s.doc.project.texture_set(0).unwrap().stack.clone()
}

#[test]
fn every_simple_material_has_brush_values_and_no_smart_material_has() {
    for material in MATERIALS {
        let values = brush_values(material.id);
        assert_eq!(values.is_some(), !material.smart, "{}", material.id);
        if let Some(values) = values {
            let channels: Vec<Channel> = values.iter().map(|(channel, _)| *channel).collect();
            for channel in [Channel::BaseColor, Channel::Metallic, Channel::Roughness] {
                assert!(channels.contains(&channel), "{} has no {channel:?}", material.id);
            }
        }
    }
    assert!(brush_values("nope").is_none());
}

#[test]
fn a_material_set_on_the_brush_adds_no_layer_and_no_history() {
    let mut s = session();
    let (layers, history) = (stack(&s), s.history_labels());

    execute(&mut s, "tool.material", &json!({"material": "copper_hammered"})).unwrap();

    assert_eq!(stack(&s), layers);
    assert_eq!(s.history_labels(), history);
    for (channel, value) in brush_values("copper_hammered").unwrap() {
        assert_eq!(s.tool.values[&channel], value, "{channel:?}");
    }
    assert!(s.tool.values[&Channel::Height][0] < 0.0, "the material's height comes along");
    assert!(s.tool.enabled.contains(&Channel::Roughness), "the channel toggles stay as they were");
}

#[test]
fn a_smart_material_or_an_unknown_one_cannot_be_the_brush_material() {
    let mut s = session();
    let before = s.tool.values.clone();

    for id in ["iron_rusted", "nope"] {
        assert!(execute(&mut s, "tool.material", &json!({"material": id})).is_err(), "{id}");
    }
    assert_eq!(s.tool.values, before);
}

#[test]
fn a_stroke_after_picking_a_material_paints_its_values_in_the_enabled_channels() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"material": "copper"})).unwrap();
    execute(&mut s, "tool.material", &json!({"channel": "metallic", "enabled": false})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 20, "hardness": 100})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[180, 200], [220, 200]]})).unwrap();

    let hit = s.pick(Vec2::new(200.0, 200.0), s.viewport).unwrap();
    let (x, y) = ((hit.uv.x * 256.0) as u32, (hit.uv.y * 256.0) as u32);
    let composite = &s.caches[0].composite;
    let (colour, rough, metal) = (composite.value(Channel::BaseColor, x, y), composite.value(Channel::Roughness, x, y), composite.value(Channel::Metallic, x, y));
    let (want_colour, want_rough) = (value_of("copper", Channel::BaseColor), value_of("copper", Channel::Roughness));
    for i in 0..3 {
        assert!((colour[i] - want_colour[i]).abs() < 0.02, "{colour:?} vs {want_colour:?}");
    }
    assert!((rough[0] - want_rough[0]).abs() < 0.02, "{rough:?} vs {want_rough:?}");
    assert!(metal[0] < 0.5, "the metallic channel was off: {metal:?}");
}

/// Add copper with `place` params (above, below, top, index)
fn apply(s: &mut Session, place: Value) -> LayerId {
    let mut params = json!({"material": "copper"});
    params.as_object_mut().unwrap().extend(place.as_object().unwrap().clone());
    LayerId(execute(s, "material.apply", &params).unwrap()["layer"].as_u64().unwrap())
}

#[test]
fn a_material_dropped_on_a_row_goes_directly_below_it_and_one_dropped_on_the_view_goes_on_top() {
    let mut s = session();
    let paint = stack(&s)[0];
    let upper = apply(&mut s, json!({}));
    let lower = apply(&mut s, json!({"index": 0}));
    assert_eq!(stack(&s), vec![lower, paint, upper]);

    let below_paint = apply(&mut s, json!({"below": paint.0}));
    assert_eq!(stack(&s), vec![lower, below_paint, paint, upper]);

    let below_bottom = apply(&mut s, json!({"below": lower.0}));
    assert_eq!(stack(&s)[0], below_bottom, "below the bottom row is the bottom");

    s.selected.insert(0, lower);
    let top = apply(&mut s, json!({"top": true}));
    assert_eq!(stack(&s).last(), Some(&top), "the top, wherever the selection is");
    assert_eq!(s.selected_layer(), Some(top));
    s.doc.project.validate().unwrap();
}
