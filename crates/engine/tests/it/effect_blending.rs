//! A content effect's blending per channel and the Blending options commands, through engine commands

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const SIZE: u32 = 128;

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, SIZE).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

/// Composited value of `channel` under the viewport center
fn center(s: &mut Session, channel: Channel) -> [f32; 3] {
    s.update();
    let hit = s.pick(Vec2::new(200.0, 200.0), s.viewport).unwrap();
    s.caches[0].composite.value(channel, (hit.uv.x * SIZE as f32) as u32, (hit.uv.y * SIZE as f32) as u32)
}

fn assert_close(actual: [f32; 3], expected: [f32; 3], what: &str) {
    assert!((0..3).all(|i| (actual[i] - expected[i]).abs() < 4e-3), "{what}: {actual:?}, expected {expected:?}");
}

fn id(value: Value, key: &str) -> u64 {
    value[key].as_u64().unwrap()
}

/// A red fill layer of roughness 0.2 under a content fill effect of grey 0.5 and roughness 0.8: (layer, effect)
fn layer_with_content_fill(s: &mut Session) -> (u64, u64) {
    let layer = id(execute(s, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0], "roughness": 0.2})).unwrap(), "layer");
    let effect = id(execute(s, "effect.add.fill", &json!({"target": "content", "color": [0.5, 0.5, 0.5], "roughness": 0.8})).unwrap(), "effect");
    (layer, effect)
}

/// What inspect shows of content effect `effect`'s per-channel blending, in `layer`'s row
fn channel_blends(s: &mut Session, layer: u64, effect: u64) -> Value {
    let inspect = execute(s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|row| row["id"] == layer).unwrap().clone();
    let shown = row["contentEffects"].as_array().unwrap().iter().find(|shown| shown["id"] == effect).unwrap().clone();
    shown["channelBlends"].clone()
}

/// Setting one channel's blend mode or opacity leaves the others' as they are, and inspect shows it
#[test]
fn a_content_effect_blends_each_channel_on_its_own() {
    let mut s = session();
    let (layer, effect) = layer_with_content_fill(&mut s);
    assert_close(center(&mut s, Channel::BaseColor), [0.5; 3], "normal at 100%");
    assert_close(center(&mut s, Channel::Roughness), [0.8; 3], "normal at 100%");

    execute(&mut s, "effect.set", &json!({"effect": effect, "channel": "baseColor", "opacity": 0.5})).unwrap();
    assert_close(center(&mut s, Channel::BaseColor), [0.75, 0.25, 0.25], "base color at half opacity");
    assert_close(center(&mut s, Channel::Roughness), [0.8; 3], "roughness keeps its full opacity");

    execute(&mut s, "effect.set", &json!({"effect": effect, "channel": "roughness", "blend": "Multiply"})).unwrap();
    assert_close(center(&mut s, Channel::Roughness), [0.16; 3], "roughness multiplies");
    assert_close(center(&mut s, Channel::BaseColor), [0.75, 0.25, 0.25], "base color stays normal");
    let blends = channel_blends(&mut s, layer, effect);
    assert_eq!(blends["baseColor"]["opacity"], json!(0.5), "{blends}");
    assert_eq!(blends["roughness"]["blend"], json!("Multiply"), "{blends}");

    // Without a channel the edit reaches every channel, and channels left alike stop blending apart
    execute(&mut s, "effect.set", &json!({"effect": effect, "opacity": 1.0, "blend": "Normal"})).unwrap();
    assert_close(center(&mut s, Channel::Roughness), [0.8; 3], "every channel back to normal at 100%");
    assert_close(center(&mut s, Channel::BaseColor), [0.5; 3], "every channel back to normal at 100%");

    assert!(channel_blends(&mut s, layer, effect).is_null(), "no channel blends apart any more");

    // Saved per channel, loaded as saved; an effect saved before has none
    execute(&mut s, "effect.set", &json!({"effect": effect, "channel": "roughness", "blend": "Multiply"})).unwrap();
    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();

    assert_eq!(back.doc.project, s.doc.project);

    let effect = s.doc.project.effect(LayerId(effect)).unwrap().1;

    assert_eq!(effect.blend_of(Some(Channel::BaseColor)), effect.blend_of(None), "unset channels share the effect's blending");
}

/// The three "on every channel" items take the chosen channel's blend mode, opacity or both, each in one undo step
#[test]
fn one_channels_blending_goes_to_every_channel_in_one_undo_step() {
    let mut s = session();
    let (layer, effect) = layer_with_content_fill(&mut s);
    execute(&mut s, "effect.set", &json!({"effect": effect, "channel": "baseColor", "opacity": 0.5})).unwrap();
    execute(&mut s, "effect.set", &json!({"effect": effect, "channel": "roughness", "blend": "Multiply"})).unwrap();
    execute(&mut s, "layer.select", &json!({"layer": layer, "target": effect})).unwrap();

    execute(&mut s, "effect.blending.opacityToEveryChannel", &json!({"channel": "baseColor"})).unwrap();
    assert_close(center(&mut s, Channel::Roughness), [0.18; 3], "roughness multiplies at half opacity");
    assert_close(center(&mut s, Channel::BaseColor), [0.75, 0.25, 0.25], "base color unchanged");
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_close(center(&mut s, Channel::Roughness), [0.16; 3], "one undo puts the opacity back");

    execute(&mut s, "effect.blending.modeToEveryChannel", &json!({"channel": "roughness"})).unwrap();
    assert_close(center(&mut s, Channel::BaseColor), [0.75, 0.0, 0.0], "base color multiplies at its own half opacity");

    execute(&mut s, "effect.blending.bothToEveryChannel", &json!({"channel": "roughness"})).unwrap();
    assert_close(center(&mut s, Channel::BaseColor), [0.5, 0.0, 0.0], "base color multiplies at full opacity");

    assert!(channel_blends(&mut s, layer, effect).is_null(), "every channel blends alike");

    // Nothing left to change: no empty undo step
    let steps = s.history_labels().len();
    execute(&mut s, "effect.blending.bothToEveryChannel", &json!({"channel": "roughness"})).unwrap();

    assert_eq!(s.history_labels().len(), steps, "every channel already blends alike");
}

/// Copy and paste carry every channel's blending to another effect; masks, levels and anchors take what applies to them
#[test]
fn channel_blending_copies_and_pastes_between_effects() {
    let mut s = session();
    let (layer, effect) = layer_with_content_fill(&mut s);
    execute(&mut s, "effect.set", &json!({"effect": effect, "channel": "roughness", "blend": "Multiply", "opacity": 0.5})).unwrap();

    assert!(execute(&mut s, "effect.blending.paste", &json!({"effect": effect})).is_err(), "nothing copied yet");

    execute(&mut s, "effect.blending.copy", &json!({"effect": effect})).unwrap();

    let other = id(execute(&mut s, "effect.add.fill", &json!({"layer": layer, "target": "content", "roughness": 0.8})).unwrap(), "effect");
    execute(&mut s, "effect.blending.paste", &json!({"effect": other})).unwrap();
    let pasted = s.doc.project.effect(LayerId(other)).unwrap().1;

    assert_eq!(pasted.blend_of(Some(Channel::Roughness)), s.doc.project.effect(LayerId(effect)).unwrap().1.blend_of(Some(Channel::Roughness)));
    assert_eq!(pasted.blend_of(Some(Channel::BaseColor)), pasted.blend_of(None));

    let steps = s.history_labels().len();
    execute(&mut s, "effect.blending.paste", &json!({"effect": other})).unwrap();

    assert_eq!(s.history_labels().len(), steps, "pasting the blending it already has adds no undo step");

    // A mask effect has one channel: it takes the shared blending and refuses a channel of its own
    execute(&mut s, "mask.add.white", &json!({"layer": layer})).unwrap();
    let masked = id(execute(&mut s, "effect.add.fill", &json!({"layer": layer})).unwrap(), "effect");
    execute(&mut s, "effect.blending.paste", &json!({"effect": masked})).unwrap();

    assert!(s.doc.project.effect(LayerId(masked)).unwrap().1.channel_blends.is_empty());
    assert!(execute(&mut s, "effect.set", &json!({"effect": masked, "channel": "roughness", "opacity": 0.5})).is_err());
    assert!(execute(&mut s, "effect.blending.modeToEveryChannel", &json!({"effect": masked})).is_err());

    // Levels and anchor points have no blending
    let levels = id(execute(&mut s, "effect.add.levels", &json!({"layer": layer, "target": "content"})).unwrap(), "effect");

    assert!(execute(&mut s, "effect.blending.copy", &json!({"effect": levels})).is_err());

    execute(&mut s, "layer.select", &json!({"layer": layer, "target": levels})).unwrap();

    assert!((texturecraft_engine::commands::find("effect.blending.paste").unwrap().enabled)(&s).is_err(), "greyed for levels");
}
