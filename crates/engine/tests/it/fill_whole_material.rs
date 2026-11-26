//! A fill holding one whole built-in material: set, saved, cleared back to its channels' defaults, on a fill layer or a content fill effect

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, EffectKind, LayerId, LayerKind, Source};
use texturecraft_engine::{Session, execute, geom::primitives};

fn session() -> Session {
    Session::new_project("Whole material", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap()
}

fn layer(v: Value) -> u64 {
    v["layer"].as_u64().unwrap()
}

fn fill(s: &Session, id: u64) -> texturecraft_engine::core::Fill {
    match &s.doc.project.layer(LayerId(id)).unwrap().kind {
        LayerKind::Fill(fill) => fill.clone(),
        _ => panic!("not a fill"),
    }
}

fn channels(s: &Session, id: u64) -> Vec<Channel> {
    s.doc.project.layer(LayerId(id)).unwrap().channels.keys().copied().collect()
}

#[test]
fn a_fill_takes_one_whole_material_names_it_and_forgets_it_once_a_channel_is_set_alone() {
    let mut s = session();
    let id = layer(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap());
    execute(&mut s, "layer.fill.set", &json!({"layer": id, "channel": "opacity", "value": 0.5})).unwrap();
    let projection = fill(&s, id).projection;
    let saved: Value = serde_json::to_value(fill(&s, id)).unwrap();

    assert!(saved.get("material").is_none(), "a fill without a whole material saves as before");

    execute(&mut s, "layer.fill.material", &json!({"layer": id, "material": "copper_hammered"})).unwrap();
    let copper = fill(&s, id);
    assert_eq!(copper.material.as_deref(), Some("copper_hammered"));
    assert!(matches!(copper.source(Channel::BaseColor), Source::Procedural { .. }), "the material's base colour");
    assert_eq!(copper.source(Channel::Metallic), Source::Uniform([1.0; 3]));
    assert_eq!(channels(&s, id), [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Height], "the material's channels on, opacity off");
    assert_eq!(copper.projection, projection, "the fill keeps its projection");
    assert_eq!(execute(&mut s, "project.inspect", &json!({})).unwrap().to_string().matches("\"material\":\"copper_hammered\"").count(), 1);

    let reopened = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();

    assert_eq!(fill(&reopened, id).material.as_deref(), Some("copper_hammered"), "saved with the project");

    execute(&mut s, "edit.undo", &json!({})).unwrap();

    assert_eq!(fill(&s, id).material, None, "one undo step");
    assert_eq!(fill(&s, id).source(Channel::BaseColor), Source::Uniform([1.0, 0.0, 0.0]));

    execute(&mut s, "edit.redo", &json!({})).unwrap();

    execute(&mut s, "layer.fill.set", &json!({"layer": id, "channel": "metallic", "enabled": false})).unwrap();

    assert!(fill(&s, id).material.is_some(), "turning a channel off keeps the material");

    execute(&mut s, "layer.fill.set", &json!({"layer": id, "channel": "roughness", "value": 0.3})).unwrap();

    assert_eq!(fill(&s, id).material, None, "a channel set on its own is no longer the whole material");

    execute(&mut s, "layer.fill.material", &json!({"layer": id, "material": "Steel"})).unwrap();
    execute(&mut s, "layer.fill.material", &json!({"layer": id, "material": null})).unwrap();
    let cleared = fill(&s, id);
    assert_eq!((cleared.material, cleared.sources.len()), (None, 0), "null: back to the channels' defaults");

    // A content fill effect takes the material too
    let effect = execute(&mut s, "effect.add.fill", &json!({"target": "content", "roughness": 0.7})).unwrap()["effect"].as_u64().unwrap();
    execute(&mut s, "layer.fill.material", &json!({"effect": effect, "material": "gold_polished"})).unwrap();
    let inspect = execute(&mut s, "project.inspect", &json!({})).unwrap().to_string();

    assert!(inspect.contains("\"material\":\"gold_polished\""));

    // The shelf names a plain material's fill; a smart one is no whole material
    let shelf = layer(execute(&mut s, "material.apply", &json!({"material": "brass"})).unwrap());

    assert_eq!(fill(&s, shelf).material.as_deref(), Some("brass"));
    assert!(execute(&mut s, "layer.fill.material", &json!({"layer": id, "material": "steel_painted_worn"})).is_err(), "smart");
    assert!(execute(&mut s, "layer.fill.material", &json!({"layer": id, "material": "nope"})).is_err());
    assert!(execute(&mut s, "layer.fill.material", &json!({"layer": id})).is_err(), "material is required");
    assert!(execute(&mut s, "layer.fill.material", &json!({"layer": id, "material": 7})).is_err());
}

#[test]
fn a_fill_in_another_texture_set_takes_the_channels_of_its_own_set() {
    let mut s = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut s, "textureSet.select", &json!({"ts": 1})).unwrap();
    let other = layer(execute(&mut s, "layer.add.fill", &Value::Null).unwrap());
    s.doc.project.texture_sets[1].channels.retain(|(channel, _)| *channel != Channel::Metallic);
    execute(&mut s, "textureSet.select", &json!({"ts": 0})).unwrap();

    execute(&mut s, "layer.fill.material", &json!({"layer": other, "material": "copper_hammered"})).unwrap();

    assert!(!channels(&s, other).contains(&Channel::Metallic), "set 1 has no metallic channel: {:?}", channels(&s, other));
    assert!(channels(&s, other).contains(&Channel::BaseColor));
}

/// The fill of content fill effect `effect`
fn effect_fill(s: &Session, effect: u64) -> texturecraft_engine::core::Fill {
    match &s.doc.project.effect(LayerId(effect)).unwrap().1.kind {
        EffectKind::Material(fill) => fill.clone(),
        other => panic!("not a content fill: {other:?}"),
    }
}

#[test]
fn clearing_a_content_fill_effects_material_lays_its_channels_defaults() {
    let mut s = crate::filter_support::facing_session();
    execute(&mut s, "layer.add.fill", &json!({"roughness": 0.2})).unwrap();
    let effect = execute(&mut s, "effect.add.fill", &json!({"target": "content", "roughness": 0.7})).unwrap()["effect"].as_u64().unwrap();
    execute(&mut s, "layer.fill.material", &json!({"effect": effect, "material": "gold_polished"})).unwrap();
    let material_channels: Vec<Channel> = effect_fill(&s, effect).sources.keys().copied().collect();

    execute(&mut s, "layer.fill.material", &json!({"effect": effect, "material": null})).unwrap();

    let cleared = effect_fill(&s, effect);
    assert_eq!(cleared.material, None);
    assert_eq!(cleared.sources.keys().copied().collect::<Vec<_>>(), material_channels, "the material's channels stay on");
    assert!(cleared.sources.iter().all(|(channel, source)| *source == Source::uniform_of(*channel)), "at their defaults");

    let roughness = crate::filter_support::center(&mut s, Channel::Roughness)[0];
    assert!((roughness - Channel::Roughness.default_value()[0]).abs() < 0.01, "the effect still lays the default roughness: {roughness}");
}

#[test]
fn setting_the_material_a_fill_already_holds_adds_no_undo_step() {
    let mut s = session();
    let id = layer(execute(&mut s, "layer.add.fill", &Value::Null).unwrap());
    let effect = execute(&mut s, "effect.add.fill", &json!({"target": "content", "roughness": 0.7})).unwrap()["effect"].as_u64().unwrap();
    let steps = s.history_labels().len();

    for target in [json!({"layer": id}), json!({"effect": effect})] {
        for material in [json!("gold_polished"), Value::Null] {
            let mut params = target.clone();
            params["material"] = material;
            execute(&mut s, "layer.fill.material", &params).unwrap();
            execute(&mut s, "layer.fill.material", &params).unwrap();
        }
    }

    assert_eq!(s.history_labels().len(), steps + 4, "one step per change, none for a repeat");
}

#[test]
fn setting_the_material_again_brings_back_a_content_fill_effects_channel_turned_off() {
    let mut s = session();
    layer(execute(&mut s, "layer.add.fill", &Value::Null).unwrap());
    let effect = execute(&mut s, "effect.add.fill", &json!({"target": "content", "roughness": 0.7})).unwrap()["effect"].as_u64().unwrap();
    let gold = json!({"effect": effect, "material": "gold_polished"});
    execute(&mut s, "layer.fill.material", &gold).unwrap();
    let whole = effect_fill(&s, effect).sources;

    for off_and_on in [vec![false], vec![false, true]] {
        for enabled in off_and_on {
            execute(&mut s, "layer.fill.set", &json!({"effect": effect, "channel": "roughness", "enabled": enabled})).unwrap();
        }

        assert_ne!(effect_fill(&s, effect).sources, whole, "roughness is off or back at its default");

        execute(&mut s, "layer.fill.material", &gold).unwrap();

        assert_eq!(effect_fill(&s, effect).sources, whole, "the material's roughness is back");
    }
}
