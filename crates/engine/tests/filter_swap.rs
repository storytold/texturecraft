//! Swapping a filter effect's type in place through `effect.set {filter}`

use serde_json::{Value, json};
use texturecraft_engine::core::{BlendMode, Effect, EffectKind, Filter, LayerId};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, StackTarget, execute};

/// A sphere project with its default layer selected
fn session() -> Session {
    Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 256).unwrap()
}

/// Runs effect-adding `command` and returns the new effect, which it selects
fn add(s: &mut Session, command: &str, params: Value) -> LayerId {
    execute(s, command, &params).unwrap();
    let StackTarget::Effect(added) = s.target else { panic!("adding an effect selects it: {:?}", s.target) };

    added
}

/// A copy of effect `id`
fn effect(s: &Session, id: LayerId) -> Effect {
    s.doc.project.effect(id).unwrap().1.clone()
}

/// Ids of the selected layer's content effects, bottom first
fn content_order(s: &Session) -> Vec<LayerId> {
    let layer = s.selected_layer().unwrap();
    s.doc.project.layer(layer).unwrap().content_effects.iter().map(|effect| effect.id).collect()
}

#[test]
fn swapping_a_filter_keeps_its_place_blend_opacity_and_visibility() {
    let mut s = session();
    let below = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let swapped = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let above = add(&mut s, "effect.add.filter", json!({"filter": "grayscale", "target": "content"}));
    execute(&mut s, "effect.set", &json!({"effect": swapped.0, "blend": "Multiply", "opacity": 0.4, "visible": false})).unwrap();

    execute(&mut s, "effect.set", &json!({"effect": swapped.0, "filter": "blur", "radius": 5.0})).unwrap();

    let blur = effect(&s, swapped);
    assert!(matches!(blur.kind, EffectKind::Filter(Filter::Blur { radius }) if radius == 5.0), "{:?}", blur.kind);
    assert_eq!(blur.name, "Blur", "a default name follows the new filter");
    assert_eq!((blur.blend, blur.opacity, blur.visible), (BlendMode::Multiply, 0.4, false));
    assert_eq!(content_order(&s), vec![below, swapped, above]);
}

#[test]
fn swapping_a_filter_keeps_a_name_the_user_chose() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content", "name": "Flip"}));

    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "threshold"})).unwrap();

    assert_eq!(effect(&s, id).name, "Flip");
}

#[test]
fn one_undo_restores_the_old_filter() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "threshold", "target": "content", "level": 0.3}));

    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "sharpen"})).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    let restored = effect(&s, id);
    assert!(matches!(restored.kind, EffectKind::Filter(Filter::Threshold { level }) if level == 0.3), "{:?}", restored.kind);
    assert_eq!(restored.name, "Threshold");
}

#[test]
fn a_mask_refuses_a_content_only_filter() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));

    let err = execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "grayscale"})).unwrap_err();

    assert!(err.to_string().contains("not on masks"), "{err}");
    assert!(matches!(effect(&s, id).kind, EffectKind::Filter(Filter::Invert)));
    execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "blur"})).expect("a mask takes a filter that works on masks");
}

#[test]
fn only_filter_effects_swap_and_only_to_filters() {
    let mut s = session();
    let levels = add(&mut s, "effect.add.levels", json!({"target": "content"}));
    let filter = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));

    assert!(execute(&mut s, "effect.set", &json!({"effect": levels.0, "filter": "blur"})).is_err(), "levels is not a filter effect");
    assert!(execute(&mut s, "effect.set", &json!({"effect": filter.0, "filter": "matFinishGrind"})).is_err(), "a material finish is a fill");
    assert!(execute(&mut s, "effect.set", &json!({"effect": filter.0, "filter": "nope"})).is_err());
    assert!(matches!(effect(&s, filter).kind, EffectKind::Filter(Filter::Invert)));
}
