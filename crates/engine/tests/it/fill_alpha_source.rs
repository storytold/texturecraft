//! A library alpha as a fill channel's source through `layer.fill.set {alpha}`: its picture becomes one image resource the channel reads

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId, LayerKind, Source};
use texturecraft_engine::fill_alpha::{library_alpha_of, resource_name};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::paint::alphas;
use texturecraft_engine::{Session, execute};

fn fill() -> (Session, LayerId) {
    let mut session = Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
    let layer = LayerId(execute(&mut session, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    (session, layer)
}

fn source(session: &Session, layer: LayerId, channel: Channel) -> Source {
    match &session.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(fill) => fill.source(channel),
        other => panic!("not a fill: {other:?}"),
    }
}

#[test]
fn an_alpha_becomes_the_channels_image_once_and_its_set_is_one_undo_step() {
    let (mut session, layer) = fill();
    let scratches = alphas::find("scratches").unwrap();
    let name = resource_name(scratches);
    let set = |session: &mut Session, channel: &str, alpha: &str| {
        execute(session, "layer.fill.set", &json!({"layer": layer.0, "channel": channel, "alpha": alpha})).unwrap();
    };
    set(&mut session, "roughness", "Scratches");

    assert_eq!(source(&session, layer, Channel::Roughness), Source::image(name.clone()));
    assert!(session.doc.resources.contains_key(&name), "the alpha's picture is a project resource, so the project saves it");

    let image = session.images.get(&name).expect("the channel's image is decoded");
    assert_eq!((image.width, image.height), (alphas::BASE as u32, alphas::BASE as u32));
    assert_eq!(library_alpha_of(&name).map(|alpha| alpha.id.as_str()), Some("scratches"), "the slot can name the alpha it came from");
    assert!(library_alpha_of("scratches.png").is_none(), "an imported image is not a library alpha");

    // One undo takes the source and the resource back together
    execute(&mut session, "edit.undo", &Value::Null).unwrap();

    assert_eq!(source(&session, layer, Channel::Roughness), Source::uniform_of(Channel::Roughness));
    assert!(!session.doc.resources.contains_key(&name), "undo takes the resource back with the source");

    // The same alpha on a second channel reads the same resource
    set(&mut session, "roughness", "Scratches");
    let count = session.doc.resources.len();
    set(&mut session, "metallic", "scratches");

    assert_eq!(source(&session, layer, Channel::Metallic), Source::image(name.clone()));
    assert_eq!(session.doc.resources.len(), count);
}

#[test]
fn an_unknown_alpha_changes_nothing() {
    let (mut session, layer) = fill();
    let before = session.doc.resources.len();
    assert!(execute(&mut session, "layer.fill.set", &json!({"layer": layer.0, "channel": "roughness", "alpha": "no such alpha"})).is_err());

    assert_eq!(source(&session, layer, Channel::Roughness), Source::uniform_of(Channel::Roughness));
    assert_eq!(session.doc.resources.len(), before);

    // No undo step either: the next undo takes back the added layer
    execute(&mut session, "edit.undo", &Value::Null).unwrap();

    assert!(session.doc.project.layer(layer).is_err(), "the refused set added no undo step");
}
