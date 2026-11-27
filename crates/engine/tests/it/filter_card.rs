//! The Filter effect card: an empty filter, its content channels switched on and off, Intensity's divider, Restore defaults and the Attributes it lists

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, Filter, LayerId, SourceSlopeBlur, SourceWarp};
use texturecraft_engine::{Session, execute};

use crate::filter_support::{SIZE, add_filter, center, facing_session as session, filter_of, set};

fn assert_close(actual: [f32; 3], expected: [f32; 3], what: &str) {
    assert!((0..3).all(|i| (actual[i] - expected[i]).abs() < 4e-3), "{what}: {actual:?}, expected {expected:?}");
}

/// A fill layer of base color 0.2 and roughness 0.3 under content filter `params`: (layer, effect)
fn filtered_layer(s: &mut Session, params: Value) -> (u64, LayerId) {
    let layer = execute(s, "layer.add.fill", &json!({"color": [0.2, 0.2, 0.2], "roughness": 0.3})).unwrap()["layer"].as_u64().unwrap();
    (layer, add_filter(s, params))
}

/// An empty filter changes nothing and is named for it; picking a filter names it after the filter, × empties it again
#[test]
fn an_empty_filter_renders_nothing_until_one_is_picked() {
    let mut s = session();
    let (_, effect) = filtered_layer(&mut s, json!({"filter": "none"}));

    assert_eq!(filter_of(&s, effect), Filter::Empty);
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Filter (empty)");

    assert_close(center(&mut s, Channel::BaseColor), [0.2; 3], "an empty filter leaves the color");

    set(&mut s, effect, json!({"filter": "invert"}));

    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Invert");

    assert_close(center(&mut s, Channel::BaseColor), [0.8; 3], "the picked filter works");

    set(&mut s, effect, json!({"filter": "none"}));

    assert_eq!(filter_of(&s, effect), Filter::Empty);

    assert_close(center(&mut s, Channel::BaseColor), [0.2; 3], "emptied again");
}

/// A content filter switched off for a channel leaves that channel; inspect lists it; masks refuse `channels`
#[test]
fn a_content_filter_leaves_the_channels_switched_off() {
    let mut s = session();
    let (layer, effect) = filtered_layer(&mut s, json!({"filter": "invert"}));

    assert_close(center(&mut s, Channel::Roughness), [0.7; 3], "every channel at first");

    set(&mut s, effect, json!({"channels": {"roughness": false}}));
    assert_close(center(&mut s, Channel::Roughness), [0.3; 3], "roughness switched off");
    assert_close(center(&mut s, Channel::BaseColor), [0.8; 3], "the color still inverted");

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|row| row["id"] == layer).unwrap().clone();
    let filter_row = row["contentEffects"].as_array().unwrap().iter().find(|filter_row| filter_row["id"] == effect.0).unwrap();

    assert_eq!(filter_row["offChannels"], json!(["roughness"]));

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_close(center(&mut s, Channel::Roughness), [0.7; 3], "one undo step");

    set(&mut s, effect, json!({"channels": {"roughness": false}}));
    set(&mut s, effect, json!({"channels": {"roughness": true}}));

    assert!(s.doc.project.effect(effect).unwrap().1.off_channels.is_empty());

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "channels": {"nope": false}})).is_err());

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "channels": {"roughness": 1}})).is_err());

    let masked = execute(&mut s, "effect.add.filter", &json!({"filter": "invert", "target": "mask"})).unwrap()["effect"].as_u64().unwrap();

    assert!(execute(&mut s, "effect.set", &json!({"effect": masked, "channels": {"roughness": false}})).is_err());
}

/// Intensity ÷ Divider: warp 0.5 ÷ 10 renders as warp 0.05; only 1, 10, 100 and 1000 are taken; a divider of 1 is not saved
#[test]
fn the_divider_divides_the_intensity() {
    let warp = |intensity: f32, divider: u16| {
        let mut s = session();
        execute(&mut s, "layer.add.fill", &Value::Null).unwrap();
        execute(&mut s, "layer.fill.set", &json!({"channel": "baseColor", "procedural": "checker", "scale": 8.0})).unwrap();
        let params = json!({"filter": "warp", "target": "content", "intensity": intensity, "seed": 5});
        let effect = execute(&mut s, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap();
        execute(&mut s, "effect.set", &json!({"effect": effect, "divider": divider})).unwrap();
        s.update();
        (0..SIZE).step_by(7).map(|x| s.caches[0].composite.value(Channel::BaseColor, x, SIZE / 2)[0]).collect::<Vec<f32>>()
    };
    let divided = warp(0.5, 10);
    assert_eq!(divided, warp(0.05, 1), "0.5 ÷ 10 = 0.05");
    assert_ne!(divided, warp(0.5, 1), "the divider changes the warp");

    let mut s = session();
    let (_, effect) = filtered_layer(&mut s, json!({"filter": "blurSlope"}));
    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "divider": 7})).is_err());

    set(&mut s, effect, json!({"divider": 100}));

    assert!(matches!(filter_of(&s, effect), Filter::SourceSlopeBlur(SourceSlopeBlur { divider: 100, .. })));

    let saved = serde_json::to_value(Filter::Warp { intensity: 1.0, scale: 2.0, seed: 3, divider: 1 }).unwrap();

    assert_eq!(saved, json!({"Warp": {"intensity": 1.0, "scale": 2.0, "seed": 3}}), "saved as before dividers");

    let loaded: Filter = serde_json::from_value(saved).unwrap();

    assert_eq!(loaded, Filter::Warp { intensity: 1.0, scale: 2.0, seed: 3, divider: 1 });
}

/// Restore defaults puts the parameters back in one undo step, keeping the name and switched-off channels; none at defaults
#[test]
fn restore_defaults_puts_the_filters_parameters_back() {
    let mut s = session();
    let (_, effect) = filtered_layer(&mut s, json!({"filter": "warp", "name": "Wobble"}));
    let defaults = filter_of(&s, effect);
    let steps = s.history_labels().len();
    assert_eq!(execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap(), json!({"changed": false}));
    assert_eq!(s.history_labels().len(), steps, "no step at defaults");

    set(&mut s, effect, json!({"intensity": 0.5, "seed": 9, "divider": 100, "channels": {"height": false}}));

    assert_eq!(execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap(), json!({"changed": true}));
    assert_eq!(filter_of(&s, effect), defaults);

    let restored = s.doc.project.effect(effect).unwrap().1.clone();

    assert_eq!(restored.name, "Wobble");
    assert!(restored.off_channels.contains(&Channel::Height));

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert!(matches!(filter_of(&s, effect), Filter::SourceWarp(SourceWarp { divider: 100, seed: 9, .. })), "one undo step");

    let empty = filtered_layer(&mut s, json!({"filter": "none"})).1;
    assert_eq!(execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": empty.0})).unwrap(), json!({"changed": false}));

    let fill = execute(&mut s, "effect.add.fill", &json!({"target": "content"})).unwrap()["effect"].as_u64().unwrap();

    assert!(execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": fill})).is_err());
}

#[test]
fn filter_list_describes_and_tags_every_filter() {
    let mut s = session();
    let listed = execute(&mut s, "filter.list", &Value::Null).unwrap();
    let entries = listed.as_array().unwrap();
    assert_eq!(entries.len(), texturecraft_engine::filter_lib::FILTERS.len());
    for def in entries {
        assert!(def["description"].as_str().is_some_and(|text| !text.is_empty()), "{def}");
        assert!(def["tags"].as_array().is_some_and(|tags| !tags.is_empty()), "{def}");
    }
}
