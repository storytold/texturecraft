//! Swapping a filter effect's type in place through `effect.set {filter}`

use serde_json::{Value, json};
use texturecraft_engine::core::{BlendMode, Effect, EffectKind, Filter, LayerId, SourceWarp, WarpMode, WarpSourceMode};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, StackTarget, execute};

use crate::filter_support::set;

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
    set(&mut s, swapped, json!({"blend": "Multiply", "opacity": 0.4, "visible": false}));

    set(&mut s, swapped, json!({"filter": "blur", "radius": 5.0}));

    let blur = effect(&s, swapped);
    assert!(matches!(blur.kind, EffectKind::Filter(Filter::Blur { radius, .. }) if radius == 5.0), "{:?}", blur.kind);
    assert_eq!(blur.name, "Blur", "a default name follows the new filter");
    assert_eq!((blur.blend, blur.opacity, blur.visible), (BlendMode::Multiply, 0.4, false));
    assert_eq!(content_order(&s), vec![below, swapped, above]);
}

#[test]
fn swapping_a_filter_keeps_a_name_the_user_chose() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content", "name": "Flip"}));

    set(&mut s, id, json!({"filter": "threshold"}));

    assert_eq!(effect(&s, id).name, "Flip");
}

#[test]
fn canonical_warp_and_blur_slope_can_be_added_and_swapped() {
    let mut session = session();
    let listed = execute(&mut session, "filter.list", &Value::Null).unwrap();
    let entries = listed.as_array().unwrap();
    let warp = add(&mut session, "effect.add.filter", json!({"filter": "warp", "target": "content", "warpMode": "directional", "angle": 45.0}));

    assert!(
        matches!(effect(&session, warp).kind, EffectKind::Filter(Filter::SourceWarp(settings)) if settings.mode == WarpMode::Directional && settings.angle == 45.0)
    );

    for (id, label) in [("warp", "Warp"), ("blurSlope", "Blur Slope")] {
        let entry = entries.iter().find(|entry| entry["id"] == id).unwrap_or_else(|| panic!("missing {id}"));
        let added = add(&mut session, "effect.add.filter", json!({"filter": id, "target": "mask"}));

        assert_eq!(entry["label"], label);
        assert_eq!(effect(&session, added).name, label);
        assert_eq!(entry["defaults"], serde_json::to_value(effect(&session, added).kind).unwrap());
    }

    set(&mut session, warp, json!({"filter": "Blur Slope", "quality": 0.5, "sourceMode": "customNoise"}));
    let blurred = effect(&session, warp);

    assert_eq!(blurred.name, "Blur Slope");
    assert!(
        matches!(blurred.kind, EffectKind::Filter(Filter::SourceSlopeBlur(settings)) if settings.quality == 0.5 && settings.source.mode == WarpSourceMode::CustomNoise)
    );

    set(&mut session, warp, json!({"filter": "warp", "warpMode": "multiDirectional", "directions": 2}));
    let warped = effect(&session, warp);

    assert_eq!(warped.name, "Warp");
    assert!(matches!(warped.kind, EffectKind::Filter(Filter::SourceWarp(settings)) if settings.mode == WarpMode::MultiDirectional && settings.directions == 2));
}

#[test]
fn replaced_filter_ids_are_rejected_without_editing_the_stack() {
    let mut session = session();
    let effect_id = add(&mut session, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let entries = execute(&mut session, "filter.list", &Value::Null).unwrap();
    let before = effect(&session, effect_id);
    let order = content_order(&session);
    let history = session.history_labels();

    for retired in ["slopeBlur", "Slope Blur", "warpDirectional", "Warp Directional", "Directional Warp"] {
        let add_error = execute(&mut session, "effect.add.filter", &json!({"filter": retired, "target": "content"})).unwrap_err();
        let swap_error = execute(&mut session, "effect.set", &json!({"effect": effect_id.0, "filter": retired, "warpMode": "directional"})).unwrap_err();

        assert!(add_error.to_string().contains("unknown filter"), "{add_error}");
        assert!(swap_error.to_string().contains("unknown filter"), "{swap_error}");
        assert!(!entries.as_array().unwrap().iter().any(|entry| entry["id"] == retired || entry["label"] == retired));
    }

    assert_eq!(effect(&session, effect_id), before);
    assert_eq!(content_order(&session), order);
    assert_eq!(session.history_labels(), history);
}

#[test]
fn setting_directional_mode_on_a_modern_warp_preserves_its_seed() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "warp", "target": "content", "seed": 7}));

    set(&mut s, id, json!({"filter": "warp", "warpMode": "directional"}));

    let edited = effect(&s, id).kind;
    assert!(matches!(edited, EffectKind::Filter(Filter::SourceWarp(warp)) if warp.mode == WarpMode::Directional && warp.seed == 7), "{edited:?}");
}

/// A content warp effect saved before the warp modes, as `filter`
fn saved_warp(s: &mut Session, filter: Filter) -> LayerId {
    let id = add(s, "effect.add.filter", json!({"filter": "warp", "target": "content"}));
    s.doc.project.effect_mut(id).unwrap().kind = EffectKind::Filter(filter);
    id
}

#[test]
fn naming_canonical_warp_replaces_a_saved_warp_even_with_the_same_label() {
    let mut session = session();
    let saved = Filter::Warp { intensity: 8.0, scale: 4.0, seed: 7, divider: 1 };
    let id = saved_warp(&mut session, saved.clone());

    set(&mut session, id, json!({"filter": "warp"}));

    assert_eq!(effect(&session, id).kind, EffectKind::Filter(Filter::SourceWarp(SourceWarp::LIBRARY)));
    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(effect(&session, id).kind, EffectKind::Filter(saved));
}

#[test]
fn editing_settings_without_replacing_a_saved_warp_keeps_its_variant() {
    let mut s = session();
    let warp = saved_warp(&mut s, Filter::Warp { intensity: 8.0, scale: 4.0, seed: 1, divider: 1 });
    let directional = saved_warp(&mut s, Filter::DirectionalWarp { intensity: 8.0, angle: 0.0, scale: 4.0, seed: 1, divider: 1 });

    set(&mut s, warp, json!({"seed": 3}));
    set(&mut s, directional, json!({"angle": 45.0}));

    assert_eq!(effect(&s, warp).kind, EffectKind::Filter(Filter::Warp { intensity: 8.0, scale: 4.0, seed: 3, divider: 1 }));
    assert_eq!(effect(&s, directional).kind, EffectKind::Filter(Filter::DirectionalWarp { intensity: 8.0, angle: 45.0, scale: 4.0, seed: 1, divider: 1 }));
}

#[test]
fn swapping_to_a_gradient_given_colors_ramps_evenly_through_them() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));

    set(&mut s, id, json!({"filter": "gradientMap", "colors": [[0, 0, 0], [1, 1, 1]]}));

    let EffectKind::Filter(gradient) = effect(&s, id).kind else { panic!("still a filter") };
    let halfway = gradient.ramp_at(0.5).unwrap();

    assert!((halfway[0] - 0.5).abs() < 1e-3, "black to white is mid gray halfway: {halfway:?}");
}

/// A swap to a gradient given five colors and their places ramps through all five, not the library's three
#[test]
fn swapping_to_a_gradient_given_colors_and_places_uses_every_color() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let colors = [[0.0; 3], [0.25; 3], [0.5; 3], [0.75; 3], [1.0; 3]];

    set(&mut s, id, json!({"filter": "gradientMap", "colors": colors, "positions": [0.0, 0.1, 0.2, 0.3, 1.0]}));

    let EffectKind::Filter(gradient) = effect(&s, id).kind else { panic!("still a filter") };
    assert_eq!(gradient.ramp_colors(), colors);
    assert_eq!(gradient.ramp_at(0.3), Some(colors[3]));
}

#[test]
fn one_undo_restores_the_old_filter() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "threshold", "target": "content", "level": 0.3}));

    set(&mut s, id, json!({"filter": "sharpen"}));
    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    let restored = effect(&s, id);
    assert!(matches!(restored.kind, EffectKind::Filter(Filter::Threshold { level, .. }) if level == 0.3), "{:?}", restored.kind);
    assert_eq!(restored.name, "Threshold");
}

#[test]
fn a_mask_refuses_a_content_only_filter() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));

    let err = execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "normalInvert"})).unwrap_err();

    assert!(err.to_string().contains("not on masks"), "{err}");
    assert!(matches!(effect(&s, id).kind, EffectKind::Filter(Filter::Invert)));
}

#[test]
fn a_mask_filter_swaps_in_place_on_the_mask() {
    let mut s = session();
    let below = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    let swapped = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    let above = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    set(&mut s, swapped, json!({"blend": "Multiply", "opacity": 0.4}));

    set(&mut s, swapped, json!({"filter": "blur"}));

    let blur = effect(&s, swapped);
    assert!(matches!(blur.kind, EffectKind::Filter(Filter::Blur { .. })), "{:?}", blur.kind);
    assert_eq!((blur.name.as_str(), blur.blend, blur.opacity), ("Blur", BlendMode::Multiply, 0.4));

    let layer = s.selected_layer().unwrap();
    let mask_order: Vec<LayerId> = s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects.iter().map(|effect| effect.id).collect();

    assert_eq!(mask_order, vec![below, swapped, above]);
}

#[test]
fn naming_the_current_filter_edits_it_in_place() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "blur", "target": "content", "radius": 5.0}));

    set(&mut s, id, json!({"filter": "blur", "radius": 9.0}));
    set(&mut s, id, json!({"filter": "blur"}));

    let blur = effect(&s, id);
    assert!(matches!(blur.kind, EffectKind::Filter(Filter::Blur { radius, .. }) if radius == 9.0), "the radius is kept, not reset: {:?}", blur.kind);
}

#[test]
fn a_name_given_with_the_swap_wins() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));

    set(&mut s, id, json!({"filter": "threshold", "name": "Edges"}));

    let swapped = effect(&s, id);
    assert!(matches!(swapped.kind, EffectKind::Filter(Filter::Threshold { .. })), "{:?}", swapped.kind);
    assert_eq!(swapped.name, "Edges");
}

#[test]
fn a_refused_swap_changes_nothing_and_adds_no_undo_step() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));
    set(&mut s, id, json!({"blend": "Multiply", "opacity": 0.4, "name": "Flip"}));
    let before = effect(&s, id);

    for refused in [
        json!({"effect": id.0, "filter": "normalInvert"}),
        json!({"effect": id.0, "filter": "nope"}),
        json!({"effect": id.0, "filter": "levels"}),
        json!({"effect": id.0, "filter": "colorBalance", "shadows": "not a color"}),
    ] {
        assert!(execute(&mut s, "effect.set", &refused).is_err(), "{refused}");
    }

    assert_eq!(effect(&s, id), before);

    // The last step is still the blend, opacity and name edit
    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(effect(&s, id).name, "Invert");
}

#[test]
fn only_filter_effects_swap_and_only_to_filters() {
    let mut s = session();
    let levels = add(&mut s, "effect.add.levels", json!({"target": "content"}));
    let filter = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));

    assert!(execute(&mut s, "effect.set", &json!({"effect": levels.0, "filter": "blur"})).is_err(), "levels is not a filter effect");
    assert!(execute(&mut s, "effect.set", &json!({"effect": levels.0, "filter": "nope"})).is_err(), "an unknown filter on levels");
    assert!(execute(&mut s, "effect.set", &json!({"effect": filter.0, "filter": "matFinishGrind"})).is_err(), "a material finish is a fill");

    assert!(execute(&mut s, "effect.set", &json!({"effect": filter.0, "filter": "nope"})).is_err());
    assert!(matches!(effect(&s, filter).kind, EffectKind::Filter(Filter::Invert)));
}

#[test]
fn invalid_swap_parameters_report_effect_set_and_preserve_history() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "content"}));
    let before = effect(&s, id);
    let history = s.history_labels();

    let error = execute(&mut s, "effect.set", &json!({"effect": id.0, "filter": "colorBalance", "shadows": "invalid", "name": "Changed"})).unwrap_err();

    assert!(matches!(error, texturecraft_engine::EngineError::BadParams { ref cmd, .. } if cmd == "effect.set"), "{error}");
    assert_eq!(effect(&s, id), before);
    assert_eq!(s.history_labels(), history);
}

#[test]
fn a_mask_takes_grayscale_conversion() {
    let mut s = session();
    let id = add(&mut s, "effect.add.filter", json!({"filter": "invert", "target": "mask"}));

    set(&mut s, id, json!({"filter": "grayscale"}));

    let swapped = effect(&s, id);
    assert!(matches!(swapped.kind, EffectKind::Filter(Filter::GrayscaleConversion(_))), "{:?}", swapped.kind);
    assert_eq!(swapped.name, "Grayscale Conversion");
}

#[test]
fn filters_carry_the_workflows_names_and_former_names_still_follow_a_swap() {
    let mut s = session();
    let slope = add(&mut s, "effect.add.filter", json!({"filter": "blurSlope", "target": "content"}));
    assert_eq!(effect(&s, slope).name, "Blur Slope");

    let names: Vec<Value> = execute(&mut s, "filter.list", &Value::Null).unwrap().as_array().unwrap().iter().map(|def| def["label"].clone()).collect();
    for name in ["Contrast Luminosity", "Blur Slope", "Highpass", "Grayscale Conversion"] {
        assert!(names.contains(&json!(name)), "{name} in {names:?}");
    }

    // A saved default name follows a swap even when the former name is no longer a command selector
    let old = add(&mut s, "effect.add.filter", json!({"filter": "blurSlope", "target": "content", "name": "Slope Blur"}));
    set(&mut s, old, json!({"filter": "blur"}));

    assert_eq!(effect(&s, old).name, "Blur");
}
