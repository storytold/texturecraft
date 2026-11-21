//! Stamping along 2D view strokes: long segments, the carry between samples and per-stamp seeds

use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// Texture size: 1e-4 UV is 0.0256 texels
const RESOLUTION: u32 = 256;

/// A sphere project painting red with `brush` settings (the UI's 0–100 scales)
fn red_brush_session(brush: serde_json::Value) -> Session {
    let mut session = Session::new_project("UV strokes", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, RESOLUTION).unwrap();
    execute(&mut session, "tool.paint", &serde_json::Value::Null).unwrap();
    execute(&mut session, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut session, "tool.brush", &brush).unwrap();
    session
}

/// The composite base color at texel (`x`, `y`)
fn color_at(session: &Session, x: u32, y: u32) -> [f32; 3] {
    session.caches[0].composite.value(Channel::BaseColor, x, y)
}

/// Whether the composite base color at (`u`, `v`) is the painted red
fn red_at(session: &Session, u: f32, v: f32) -> bool {
    let color = color_at(session, (u * RESOLUTION as f32) as u32, (v * RESOLUTION as f32) as u32);
    color[0] > 0.85 && color[1] < 0.15
}

#[test]
fn a_segment_longer_than_one_stamp_batch_paints_to_its_end() {
    // 1e-4 UV between stamps: 0.9 UV is 9000 stamps, past the 4096 of one batch
    let mut session = red_brush_session(json!({"size": 1.0, "spacing": 1.0, "hardness": 90}));

    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.05, 0.5], [0.95, 0.5]]})).unwrap();

    for u in [0.1, 0.5, 0.9] {
        assert!(red_at(&session, u, 0.5), "painted at u = {u}");
    }
}

#[test]
fn a_segment_cut_at_the_stamp_limit_does_not_stamp_the_next_one_backwards() {
    // 1e-4 UV between stamps: the 20.5 UV first segment needs 205k stamps and is cut off before reaching the texture
    let mut session = red_brush_session(json!({"size": 1.0, "spacing": 1.0, "hardness": 90}));

    execute(&mut session, "tool.strokeUv", &json!({"points": [[-20.0, 0.5], [0.5, 0.5], [0.6, 0.5]]})).unwrap();

    assert!(red_at(&session, 0.55, 0.5), "the last segment is painted");
    assert!(!red_at(&session, 0.3, 0.5), "nothing is stamped behind the last segment's start");
}

#[test]
fn each_sample_of_a_stroke_stamps_new_noise() {
    // A 32 texel radius at full spacing: one stamp per sample, each centred on a texel at u = 0.25 and 0.5
    let mut session = red_brush_session(json!({"size": 25.0, "spacing": 100.0, "alpha": "cloud"}));

    execute(&mut session, "tool.strokeUv", &json!({"points": [[0.25, 0.5], [0.5, 0.5]]})).unwrap();

    let patch = |centre_x: u32| -> Vec<[f32; 3]> {
        (centre_x - 16..centre_x + 16).flat_map(|x| (112..144).map(move |y| (x, y))).map(|(x, y)| color_at(&session, x, y)).collect()
    };
    let first_stamp = patch(64);
    assert!(first_stamp.iter().any(|color| color[0] > 0.5 && color[1] < 0.5), "the first stamp painted");
    assert_ne!(first_stamp, patch(128), "the second sample's stamp has its own noise");
}

#[test]
fn a_hostile_segment_stays_bounded() {
    // The finest brush the settings allow: 2e-5 UV between stamps
    let mut session = red_brush_session(json!({"size": 0.2, "spacing": 1.0}));

    let result = execute(&mut session, "tool.strokeUv", &json!({"points": [[0.0, 0.0], [1e9, 1e9]]}));

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn a_light_pressure_diagonal_reaches_the_far_corner() {
    let mut session = red_brush_session(json!({"size": 0.2, "minSize": 0.0, "spacing": 1.0, "hardness": 90}));

    // Half pressure reaches the minimum stamp step: a diagonal needs about 142k stamps
    session.begin_stroke_uv([0.0, 0.0].into(), 0.5).unwrap();
    session.continue_stroke_uv([1.0, 1.0].into(), 0.5);
    session.end_stroke().unwrap();
    session.update();

    assert!(red_at(&session, 0.5, 0.5), "the diagonal paints through its midpoint");
    assert!(red_at(&session, 0.97, 0.97), "the diagonal reaches the far corner");
}

#[test]
fn decreasing_pressure_does_not_stamp_behind_a_turn() {
    let mut session = red_brush_session(json!({"size": 10.0, "minSize": 0.0, "spacing": 100.0, "hardness": 90}));

    session.begin_stroke_uv([0.25, 0.5].into(), 1.0).unwrap();
    session.continue_stroke_uv([0.33, 0.5].into(), 1.0);
    session.continue_stroke_uv([0.33, 0.6].into(), 0.1);
    session.end_stroke().unwrap();
    session.update();

    assert!(red_at(&session, 0.33, 0.55), "the new segment is painted");
    assert!(!red_at(&session, 0.33, 0.45), "a smaller step must not stamp behind the turn");
}

/// A black mask on a white fill, with a paint effect below its blur.
fn blurred_mask_session() -> Session {
    let mut s = Session::new_project("mask cache", primitives::plane(1.0, 1), "builtin:plane", None, RESOLUTION).unwrap();
    let layer = execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0]})).unwrap()["layer"].clone();
    execute(&mut s, "mask.add.black", &json!({})).unwrap();
    let paint = execute(&mut s, "effect.add.paint", &json!({"target": "mask"})).unwrap()["effect"].clone();
    execute(&mut s, "effect.add.filter", &json!({"target": "mask", "filter": "blur", "radius": 32})).unwrap();
    execute(&mut s, "layer.select", &json!({"layer": layer, "target": paint})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 2.0, "hardness": 100})).unwrap();
    s.update();
    s
}

#[test]
fn swapping_mask_gray_refreshes_the_last_displayed_dab_and_keeps_its_priority() {
    let mut session = Session::new_project("Mask stroke", primitives::plane(1.0, 1), "builtin:plane", None, RESOLUTION).unwrap();
    execute(&mut session, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0]})).unwrap();
    execute(&mut session, "mask.add.white", &json!({})).unwrap();
    session.tool.mask_value = 1.0;
    session.update();
    session.take_uploads(0);

    session.begin_stroke_uv([0.5, 0.5].into(), 1.0).unwrap();
    let (id, tiles) = session.pending_display_stroke(0).unwrap();
    let tile = session.tile_demand(None).brush.unwrap();
    assert!(tiles.contains(&tile));
    session.update();
    session.take_uploads(0);
    session.clear_display_stroke_tiles();
    assert!(session.pending_display_stroke(0).unwrap().1.is_empty());

    execute(&mut session, "tool.swapMaskValue", &json!({})).unwrap();
    assert!(session.pending_display_stroke(0).unwrap().1.contains(&tile), "a new gray refreshes existing coverage");
    let revision = session.caches[0].material_revision;
    session.end_stroke().unwrap();

    assert!(session.finished_display_stroke(0, id, revision).unwrap().contains(&tile), "the commit refreshes the last displayed gray");
    assert!(session.caches[0].settle.contains(&tile));
    assert!(session.caches[0].dirty.is_empty() && session.caches[0].upload.is_empty(), "only the settle queue holds the released dab");
    assert_eq!(session.tile_demand(None).brush, Some(tile));
}

#[test]
fn filtered_stroke_refreshes_the_neighbour_on_preview_cancel_commit_and_history() {
    use texturecraft_engine::compose::Composite;
    fn compare_full(s: &mut Session, phase: &str) -> Composite {
        s.update();
        let incremental = s.caches[0].composite.clone();
        let cache = &mut s.caches[0];
        let warm = std::mem::take(&mut cache.below);
        cache.below.budget = 0;
        cache.invalidate_all();
        s.update();
        s.caches[0].below = warm;
        assert!(s.caches[0].composite == incremental, "{phase}: incremental output differs from a full evaluation");
        incremental
    }

    let mut s = blurred_mask_session();
    s.begin_stroke_uv([0.8, 0.8].into(), 1.0).unwrap();
    s.end_stroke().unwrap();
    s.update();
    let baseline = s.caches[0].composite.clone();
    let uv = [60.5 / RESOLUTION as f32, 32.5 / RESOLUTION as f32].into();
    s.begin_stroke_uv(uv, 1.0).unwrap();
    assert_eq!(s.stroke_tiles(), 1, "the brush itself stays inside one tile");
    let live = compare_full(&mut s, "live");
    assert_ne!(live.value(Channel::BaseColor, 64, 32), baseline.value(Channel::BaseColor, 64, 32));
    s.cancel_stroke();
    assert!(compare_full(&mut s, "cancel") == baseline);
    s.begin_stroke_uv(uv, 1.0).unwrap();
    s.end_stroke().unwrap();
    let committed = compare_full(&mut s, "commit");
    s.undo().unwrap();
    assert!(!s.caches[0].dirty.contains(&(3, 3)), "undo leaves distant tiles alone");
    assert!(compare_full(&mut s, "undo") == baseline);
    s.redo().unwrap();
    assert!(!s.caches[0].dirty.contains(&(3, 3)), "redo leaves distant tiles alone");
    assert!(compare_full(&mut s, "redo") == committed);
}
