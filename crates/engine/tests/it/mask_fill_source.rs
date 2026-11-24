//! A mask fill effect edited through the `layer.fill.*` commands like any fill: its one grayscale source, its projection and its symmetry

use serde_json::{Value, json};
use texturecraft_engine::core::{EffectKind, LayerId, OutputResolution, Procedural, ProceduralKind, Projection, Source};
use texturecraft_engine::fill_alpha::resource_name;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::paint::alphas;
use texturecraft_engine::{Session, execute};

/// A paint layer whose mask holds one fill effect, and that effect
fn mask_fill() -> (Session, LayerId, LayerId) {
    mask_fill_on(128)
}

/// [`mask_fill`] on a texture set `size` texels across
fn mask_fill_on(size: u32) -> (Session, LayerId, LayerId) {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, size).unwrap();
    let layer = LayerId(execute(&mut s, "layer.add.paint", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    let effect = LayerId(execute(&mut s, "effect.add.fill", &json!({"layer": layer.0, "target": "mask"})).unwrap()["effect"].as_u64().unwrap());
    (s, layer, effect)
}

fn source(s: &Session, effect: LayerId) -> Source {
    match &s.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Fill { source, .. } => source.clone(),
        other => panic!("not a mask fill: {other:?}"),
    }
}

fn projection(s: &Session, effect: LayerId) -> Projection {
    match &s.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Fill { projection, .. } => projection.clone(),
        other => panic!("not a mask fill: {other:?}"),
    }
}

/// Mask values along the texture's diagonal
fn mask_line(s: &Session, layer: LayerId) -> Vec<f32> {
    let texels: Vec<(u32, u32)> = (0..16).map(|i| (i * 8 + 3, i * 8 + 5)).collect();
    s.mask_at(layer, &texels).unwrap()
}

#[test]
fn a_mask_fill_takes_a_procedural_an_image_an_alpha_or_a_value_as_its_one_grayscale_source() {
    let (mut s, layer, effect) = mask_fill();
    let kind = ProceduralKind::ALL[0];

    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "procedural": kind.label(), "scale": 4.0})).unwrap();
    let Source::Procedural { pattern, low, high, .. } = source(&s, effect) else { panic!("not a procedural: {:?}", source(&s, effect)) };
    assert_eq!((pattern.kind, pattern.scale, low, high), (kind, 4.0, [0.0; 3], [1.0; 3]));

    let line = mask_line(&s, layer);
    assert!(line.iter().any(|value| (value - line[0]).abs() > 1e-3), "the mask follows the pattern: {line:?}");

    // A parameter alone keeps the rest of the pattern
    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "contrast": 1.5})).unwrap();
    let Source::Procedural { pattern, .. } = source(&s, effect) else { panic!("not a procedural") };
    assert_eq!((pattern.scale, pattern.contrast), (4.0, 1.5));

    let scratches = alphas::find("scratches").unwrap();
    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "alpha": "scratches"})).unwrap();

    assert_eq!(source(&s, effect), Source::image(resource_name(scratches)));
    assert!(s.doc.resources.contains_key(&resource_name(scratches)));

    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "value": 0.25})).unwrap();

    assert_eq!(source(&s, effect), Source::Uniform([0.25; 3]));
    assert!(mask_line(&s, layer).iter().all(|value| (value - 0.25).abs() < 1e-3));

    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "value": 7.0})).unwrap();

    assert_eq!(source(&s, effect), Source::Uniform([1.0; 3]), "a mask value stays in 0..1");

    let inspected = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();

    assert!(inspected.contains("projectionSettings"), "inspect shows the mask fill's projection");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(source(&s, effect), Source::image(resource_name(scratches)), "each pick is one undo step");
}

#[test]
fn a_mask_fill_has_no_channels_or_material() {
    let (mut s, _, effect) = mask_fill();
    let before = source(&s, effect);
    for refused in [
        json!({"effect": effect.0, "channel": "height", "value": 0.5}),
        json!({"effect": effect.0, "enabled": false}),
        json!({"effect": effect.0}),
        json!({"effect": effect.0, "procedural": "no such pattern"}),
        json!({"effect": effect.0, "image": "missing.png"}),
    ] {
        assert!(execute(&mut s, "layer.fill.set", &refused).is_err(), "{refused}");
    }

    assert!(execute(&mut s, "layer.fill.material", &json!({"effect": effect.0, "material": null})).is_err());
    assert_eq!(source(&s, effect), before);
}

#[test]
fn a_mask_fill_is_projected_and_reset_like_any_fill() {
    let (mut s, _, effect) = mask_fill();
    execute(&mut s, "layer.fill.projection", &json!({"effect": effect.0, "projection": "planar", "scale": 2.0, "symmetry": true})).unwrap();

    assert!(matches!(projection(&s, effect), Projection::Planar { scale, .. } if scale == 2.0));

    let inspected = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();

    assert!(inspected.contains("projectionSettings"), "inspect shows the mask fill's projection");

    execute(&mut s, "layer.fill.resetProjection", &json!({"effect": effect.0})).unwrap();

    assert!(matches!(projection(&s, effect), Projection::Planar { scale, .. } if scale == 1.0), "the reset keeps the projection, at its defaults");
    match &s.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Fill { sampling, .. } => assert!(sampling.symmetry.on, "the reset keeps the Symmetry section"),
        other => panic!("not a mask fill: {other:?}"),
    }

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert!(matches!(projection(&s, effect), Projection::Planar { scale, .. } if scale == 2.0), "the reset is one undo step");
}

fn resolution(s: &Session, effect: LayerId) -> OutputResolution {
    match source(s, effect) {
        Source::Procedural { pattern, .. } => pattern.resolution,
        other => panic!("not a procedural: {other:?}"),
    }
}

#[test]
fn a_procedural_source_is_made_at_its_resolution() {
    let (mut s, layer, effect) = mask_fill_on(512);
    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "procedural": "Clouds", "scale": 8.0})).unwrap();
    let every_texel = mask_line(&s, layer);
    assert!(resolution(&s, effect).is_default());

    // A quarter of the texture set's size: the pattern is made at 128 × 128 and smoothed back up
    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "procedural": "Clouds", "resolutionFactor": -2})).unwrap();

    assert_eq!(resolution(&s, effect).factor, -2);

    let coarse = mask_line(&s, layer);
    assert!(coarse.iter().zip(&every_texel).any(|(sampled, exact)| (sampled - exact).abs() > 1e-3), "{coarse:?} vs {every_texel:?}");
    assert!(execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string().contains("\"resolution\""), "inspect shows it");

    // Another parameter keeps it; a bad size is refused and changes nothing
    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "procedural": "Clouds", "contrast": 1.0})).unwrap();

    assert_eq!(resolution(&s, effect).factor, -2);
    assert!(execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "procedural": "Clouds", "resolution": "big"})).is_err());
    assert_eq!(resolution(&s, effect).factor, -2);

    // The default size again: every texel, as before
    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "procedural": "Clouds", "resolution": "auto", "resolutionFactor": 0})).unwrap();

    assert!(resolution(&s, effect).is_default());
    assert_eq!(mask_line(&s, layer), every_texel);
}

#[test]
fn a_pattern_saved_before_resolutions_existed_reads_and_saves_unchanged() {
    let old = r#"{"kind":"Clouds","scale":4.0,"seed":1,"contrast":1.0,"balance":0.0,"invert":false}"#;
    let pattern: Procedural = serde_json::from_str(old).unwrap();

    assert_eq!(pattern, Procedural::new(ProceduralKind::Clouds));
    assert_eq!(serde_json::to_string(&pattern).unwrap(), old, "no resolution, no new field in the file");
}

#[test]
fn setting_the_source_a_mask_fill_already_reads_adds_no_undo_step() {
    let (mut s, _, effect) = mask_fill();
    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "value": 0.25})).unwrap();
    let steps = s.history_labels().len();

    execute(&mut s, "layer.fill.set", &json!({"effect": effect.0, "value": 0.25})).unwrap();

    assert_eq!(s.history_labels().len(), steps);
    assert_eq!(source(&s, effect), Source::Uniform([0.25; 3]));
}
