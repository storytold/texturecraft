//! Issue #175: the "Pattern scale" slider of a fill's procedural channel applies what it was
//! dragged, clicked or typed to.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{Channel, LayerKind, Source};
use texturecraft_ui_egui::TextureCraftApp;

/// Pattern scale a new procedural source starts at (`Procedural::new`)
const DEFAULT_SCALE: f32 = 4.0;

/// The sample session with a new fill layer whose roughness is a grunge pattern, selected in
/// Properties.
fn harness() -> (Harness<'static, TextureCraftApp>, u64) {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let added = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    let layer = added["layer"].as_u64().unwrap();
    texturecraft_engine::execute(&mut session, "layer.fill.set", &json!({"layer": layer, "channel": "roughness", "procedural": "grunge"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 2400.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    (h, layer)
}

/// The stored pattern scale of the fill's roughness channel.
fn scale(h: &Harness<'_, TextureCraftApp>, layer: u64) -> f32 {
    let l = h.state().session.doc.project.layers.get(&texturecraft_engine::core::LayerId(layer)).unwrap();
    let LayerKind::Fill(f) = &l.kind else { panic!("fill") };
    match f.source(Channel::Roughness) {
        Source::Procedural { pattern, .. } => pattern.scale,
        _ => panic!("procedural"),
    }
}

/// The track under the slider labelled `label` (the row below the label).
fn track(h: &Harness<'_, TextureCraftApp>, label: &str) -> egui::Rect {
    let l = h.get_by_label(label).rect();
    egui::Rect::from_min_max(egui::pos2(l.left() + 6.0, l.bottom() + 6.0), egui::pos2(l.left() + 200.0, l.bottom() + 14.0))
}

#[test]
fn dragging_the_pattern_scale_applies_live() {
    let (mut h, layer) = harness();
    assert_eq!(scale(&h, layer), DEFAULT_SCALE);
    let t = track(&h, "Pattern scale");
    let at = |x: f32| egui::pos2(t.left() + x, t.center().y);
    h.drag_at(at(10.0));
    h.run_steps(2);
    let mut last = 0.0;
    for x in [60.0, 160.0] {
        h.hover_at(at(x));
        h.run_steps(2);
        assert!(scale(&h, layer) > last && scale(&h, layer) != DEFAULT_SCALE, "the scale follows the drag: {last} -> {}", scale(&h, layer));
        last = scale(&h, layer);
    }
    h.drop_at(at(160.0));
    h.run_steps(3);
    assert_eq!(scale(&h, layer), last);
}

/// Dragging the scale keeps the pattern's other settings, which the panel has no control for.
#[test]
fn dragging_the_pattern_scale_keeps_balance_and_invert() {
    let (mut h, layer) = harness();
    texturecraft_engine::execute(
        &mut h.state_mut().session,
        "layer.fill.set",
        &json!({"layer": layer, "channel": "roughness", "procedural": "grunge", "balance": 0.25, "invert": true}),
    )
    .unwrap();
    h.run_steps(3);
    let t = track(&h, "Pattern scale");
    h.drag_at(egui::pos2(t.left() + 100.0, t.center().y));
    h.run_steps(2);
    h.drop_at(egui::pos2(t.left() + 100.0, t.center().y));
    h.run_steps(3);
    let l = h.state().session.doc.project.layers.get(&texturecraft_engine::core::LayerId(layer)).unwrap();
    let LayerKind::Fill(f) = &l.kind else { panic!("fill") };
    let Source::Procedural { pattern, .. } = f.source(Channel::Roughness) else { panic!("procedural") };
    assert!(pattern.scale != DEFAULT_SCALE, "the drag applied");
    assert_eq!((pattern.balance, pattern.invert), (0.25, true), "balance and invert survive the drag");
}

/// The Seed slider is an f32; a Scale drag must not round a seed above 2^24 through it.
#[test]
fn dragging_the_pattern_scale_keeps_a_large_seed() {
    let (mut h, layer) = harness();
    texturecraft_engine::execute(
        &mut h.state_mut().session,
        "layer.fill.set",
        &json!({"layer": layer, "channel": "roughness", "procedural": "grunge", "seed": 123_456_789u64}),
    )
    .unwrap();
    h.run_steps(3);
    let t = track(&h, "Pattern scale");
    h.drag_at(egui::pos2(t.left() + 100.0, t.center().y));
    h.run_steps(2);
    h.drop_at(egui::pos2(t.left() + 100.0, t.center().y));
    h.run_steps(3);
    let l = h.state().session.doc.project.layers.get(&texturecraft_engine::core::LayerId(layer)).unwrap();
    let LayerKind::Fill(f) = &l.kind else { panic!("fill") };
    let Source::Procedural { pattern, .. } = f.source(Channel::Roughness) else { panic!("procedural") };
    assert!(pattern.scale != DEFAULT_SCALE, "the drag applied");
    assert_eq!(u64::from(pattern.seed), 123_456_789, "the seed survives the drag");
}
