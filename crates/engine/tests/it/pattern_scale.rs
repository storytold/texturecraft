//! Issue #175: the scale of a fill's procedural pattern changes the composited texture, for every
//! pattern kind, through the command the "Pattern scale" slider issues.

use serde_json::json;
use texturecraft_engine::core::{Channel, LayerKind, ProceduralKind, Source};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// Texture size: small, but large enough for a pattern at scale 2 and at scale 30 to differ.
const SIZE: u32 = 128;
/// Scales the slider moves between (its range is 0.1..=100).
const SCALE_LOW: f32 = 2.0;
const SCALE_HIGH: f32 = 30.0;

/// A project with one fill layer whose roughness is `kind` at scale `SCALE_LOW`.
fn session_with(kind: ProceduralKind) -> Session {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, SIZE).unwrap();
    s.viewport = Vec2::new(200.0, 200.0);
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"channel": "roughness", "procedural": kind.label(), "scale": SCALE_LOW})).unwrap();
    s.update();
    s
}

/// The roughness plane of the first texture set's composite.
fn plane(s: &mut Session) -> Vec<f32> {
    s.update();
    let c = &s.caches[0].composite;
    (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).map(|(x, y)| c.value(Channel::Roughness, x, y)[0]).collect()
}

/// The pattern scale stored in the (only) fill layer's roughness source.
fn stored_scale(s: &Session) -> f32 {
    let layer = s.doc.project.layers.values().find(|l| matches!(l.kind, LayerKind::Fill(_))).unwrap();
    let LayerKind::Fill(f) = &layer.kind else { unreachable!() };
    match f.source(Channel::Roughness) {
        Source::Procedural { pattern, .. } => pattern.scale,
        _ => panic!("not procedural"),
    }
}

#[test]
fn changing_the_pattern_scale_changes_the_texture_for_every_pattern() {
    for kind in ProceduralKind::ALL {
        let mut s = session_with(kind);
        let before = plane(&mut s);
        // Exactly what the slider sends while dragged (see `dock.rs`, "Pattern scale").
        execute(
            &mut s,
            "layer.fill.set",
            &json!({"channel": "roughness", "procedural": kind.label(), "scale": SCALE_HIGH, "contrast": 1.0, "seed": 1, "low": [0.0, 0.0, 0.0], "high": [1.0, 1.0, 1.0], "merge": false}),
        )
        .unwrap();
        assert_eq!(stored_scale(&s), SCALE_HIGH, "{kind:?}: the document keeps the scale");
        assert!(plane(&mut s) != before, "{kind:?}: the composite follows the scale");
    }
}
