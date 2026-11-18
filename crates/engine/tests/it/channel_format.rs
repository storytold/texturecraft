//! Texture Set Settings: a channel's storage format (`textureSet.channel.format`).

use serde_json::json;
use texturecraft_engine::core::{Channel, ChannelFormat, LayerId};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 128).unwrap();
    s.update();
    s
}

fn format(s: &Session, c: Channel) -> ChannelFormat {
    s.doc.project.texture_sets[0].channels.iter().find(|(x, _)| *x == c).unwrap().1
}

/// A paint layer whose roughness holds one texel of 0x0123 (coverage 0xffff).
fn painted(s: &mut Session) -> LayerId {
    let id = LayerId(execute(s, "layer.add.paint", &json!({})).unwrap()["layer"].as_u64().unwrap());
    s.doc.pixels.layer_mut(id, Channel::Roughness, (128, 128)).set(3, 4, &[0x0123, 0xffff]);
    id
}

fn texel(s: &Session, id: LayerId) -> [u16; 4] {
    s.doc.pixels.layer(id, Channel::Roughness).unwrap().get(3, 4)
}

#[test]
fn the_format_changes_and_one_undo_restores_it() {
    let mut s = session();
    assert_eq!(format(&s, Channel::BaseColor), ChannelFormat::Srgb8);
    let r = execute(&mut s, "textureSet.channel.format", &json!({"channel": "baseColor", "format": "RGB16F"})).unwrap();
    assert_eq!(r["format"], "RGB16F");
    assert_eq!(format(&s, Channel::BaseColor), ChannelFormat::Rgb16F);

    execute(&mut s, "edit.undo", &json!({})).unwrap();
    assert_eq!(format(&s, Channel::BaseColor), ChannelFormat::Srgb8);
}

#[test]
fn eight_bit_rounds_the_layer_data_and_undo_brings_it_back() {
    let mut s = session();
    let id = painted(&mut s);
    execute(&mut s, "textureSet.channel.format", &json!({"channel": "roughness", "format": "L16"})).unwrap();
    assert_eq!(texel(&s, id)[0], 0x0123, "16 bit keeps every value");

    execute(&mut s, "textureSet.channel.format", &json!({"channel": "roughness", "format": "L8"})).unwrap();
    // 0x0123 / 65535 * 255 = 1.12 -> step 1 -> 257
    assert_eq!(texel(&s, id), [257, 0xffff, 0, 0], "rounded to 256 steps, coverage untouched");

    execute(&mut s, "edit.undo", &json!({})).unwrap();
    assert_eq!(format(&s, Channel::Roughness), ChannelFormat::L16);
    assert_eq!(texel(&s, id)[0], 0x0123, "undo restores the unrounded data");
}

#[test]
fn only_the_asked_channel_is_converted() {
    let mut s = session();
    let id = painted(&mut s);
    s.doc.pixels.layer_mut(id, Channel::Metallic, (128, 128)).set(3, 4, &[0x0123, 0xffff]);
    execute(&mut s, "textureSet.channel.format", &json!({"channel": "metallic", "format": "L8"})).unwrap();
    assert_eq!(texel(&s, id)[0], 0x0123, "roughness is a different channel");
}

#[test]
fn wrong_family_unknown_format_and_missing_channel_are_errors_that_change_nothing() {
    let mut s = session();
    let steps = s.history_labels().len();
    for p in [
        json!({"channel": "baseColor", "format": "L8"}),
        json!({"channel": "roughness", "format": "sRGB8"}),
        json!({"channel": "roughness", "format": "L12"}),
        json!({"channel": "roughness"}),
        json!({"channel": "coatWeight", "format": "L8"}),
    ] {
        assert!(execute(&mut s, "textureSet.channel.format", &p).is_err(), "{p}");
    }
    assert_eq!(format(&s, Channel::Roughness), ChannelFormat::L8);
    assert_eq!(s.history_labels().len(), steps);
}

#[test]
fn the_same_format_is_not_an_undo_step() {
    let mut s = session();
    let steps = s.history_labels().len();
    execute(&mut s, "textureSet.channel.format", &json!({"channel": "roughness", "format": "l8"})).unwrap();
    assert_eq!(s.history_labels().len(), steps);
}
