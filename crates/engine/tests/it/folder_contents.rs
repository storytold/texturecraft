//! A folder's Scale and Rotation over the fills inside it (stored on the folder, reset drops it) and Randomize seeds as one undo step

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, ContentsOffset, EffectKind, Filter, LayerId, LayerKind, Projection, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// The test project's texture size
const RESOLUTION: u32 = 128;

/// Texels between two base colour samples, across and down
const SAMPLE_STRIDE: usize = 9;

fn id(v: Value, key: &str) -> LayerId {
    LayerId(v[key].as_u64().unwrap())
}

/// A folder holding a Clouds fill (base color and roughness on one seed) with a warp filter in its mask; returns (session, folder, fill)
fn folder_with_fill() -> (Session, LayerId, LayerId) {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 16, 8), "builtin:sphere", None, RESOLUTION).unwrap();
    let fill = id(execute(&mut s, "layer.add.fill", &Value::Null).unwrap(), "layer");
    for channel in ["baseColor", "roughness"] {
        execute(&mut s, "layer.fill.set", &json!({"layer": fill.0, "channel": channel, "procedural": "Clouds", "seed": 7})).unwrap();
    }
    execute(&mut s, "layer.fill.projection", &json!({"layer": fill.0, "projection": "uv", "scale": [2.0, 2.0], "rotation": 10.0})).unwrap();
    execute(&mut s, "mask.add.white", &json!({"layer": fill.0})).unwrap();
    execute(&mut s, "effect.add.filter", &json!({"layer": fill.0, "target": "mask", "filter": "warp"})).unwrap();
    let folder = id(execute(&mut s, "layer.group", &json!({"layer": fill.0})).unwrap(), "layer");
    s.update();
    (s, folder, fill)
}

fn projection(s: &Session, fill: LayerId) -> Projection {
    match &s.doc.project.layer(fill).unwrap().kind {
        LayerKind::Fill(f) => f.projection.clone(),
        other => panic!("not a fill: {other:?}"),
    }
}

fn base_colors(s: &Session) -> Vec<[f32; 3]> {
    let composite = &s.caches[s.ts()].composite;
    let samples = || (0..RESOLUTION).step_by(SAMPLE_STRIDE);
    samples().flat_map(|y| samples().map(move |x| (x, y))).map(|(x, y)| composite.value(Channel::BaseColor, x, y)).collect()
}

#[test]
fn a_folders_scale_and_rotation_lay_on_its_fills_without_changing_them_and_reset_drops_them() {
    let (mut s, folder, fill) = folder_with_fill();
    let own = projection(&s, fill);
    let before = base_colors(&s);
    let steps = s.history_labels().len();

    let set = execute(&mut s, "folder.contents.set", &json!({"layer": folder.0, "scale": 1.5, "rotation": 30.0})).unwrap();

    assert_eq!(set, json!({"scale": 1.5, "rotation": 30.0}));
    assert_eq!(s.history_labels().len(), steps + 1, "one undo step");
    assert_eq!(s.doc.project.contents_around(fill), ContentsOffset::new(1.5, 30.0));
    assert_eq!(projection(&s, fill), own, "the fill keeps its own values");

    s.update();
    let turned = base_colors(&s);
    assert_ne!(turned, before, "the fill renders scaled and turned");

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();

    assert!(inspect.contains(r#""contents":{"rotation":30.0,"scale":1.5}"#), "{inspect}");

    // A drag continues its step; one value given keeps the other
    execute(&mut s, "folder.contents.set", &json!({"layer": folder.0, "rotation": 400.0, "merge": true})).unwrap();

    assert_eq!(s.history_labels().len(), steps + 1);
    assert_eq!(s.doc.project.contents_offset(folder), ContentsOffset::new(1.5, 40.0));

    // The values it already has add no step
    let unchanged = execute(&mut s, "folder.contents.set", &json!({"layer": folder.0, "scale": 1.5, "rotation": 40.0})).unwrap();

    assert_eq!(unchanged, json!({"scale": 1.5, "rotation": 40.0}));
    assert_eq!(s.history_labels().len(), steps + 1, "no step for the same values");

    execute(&mut s, "folder.contents.reset", &json!({"layer": folder.0})).unwrap();

    assert!(s.doc.project.contents_offset(folder).is_none());

    s.update();

    assert_eq!(base_colors(&s), before, "reset gives every fill its own back");

    let steps = s.history_labels().len();
    execute(&mut s, "folder.contents.reset", &json!({"layer": folder.0})).unwrap();

    assert_eq!(s.history_labels().len(), steps, "nothing to drop, no step");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(s.doc.project.contents_offset(folder), ContentsOffset::new(1.5, 40.0));

    // Only folders take an offset, and it needs a value
    assert!(execute(&mut s, "folder.contents.set", &json!({"layer": fill.0, "scale": 2.0})).is_err());
    assert!(execute(&mut s, "folder.contents.set", &json!({"layer": folder.0})).is_err());
    assert!(execute(&mut s, "folder.randomizeSeeds", &json!({"layer": fill.0})).is_err());
}

#[test]
fn randomize_seeds_gives_every_pattern_inside_a_new_seed_in_one_undo_step() {
    let (mut s, folder, fill) = folder_with_fill();

    // The folder inside another: its seeds are inside that one too
    let outer = id(execute(&mut s, "layer.group", &json!({"layer": folder.0})).unwrap(), "layer");
    let before = fill_seeds(&s, fill);
    let steps = s.history_labels().len();

    let out = execute(&mut s, "folder.randomizeSeeds", &json!({"layer": outer.0})).unwrap();

    assert_eq!(out["seeds"], 3);
    assert_eq!(s.history_labels().len(), steps + 1, "one undo step");

    let after = fill_seeds(&s, fill);
    assert!(after.0 != before.0 && after.2 != before.2, "{before:?} -> {after:?}");
    assert_eq!(after.0, after.1, "channels on one seed stay on one seed");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(fill_seeds(&s, fill), before, "one undo takes every seed back");

    // A folder with nothing seeded inside adds no step
    let empty = id(execute(&mut s, "layer.add.group", &Value::Null).unwrap(), "layer");
    let steps = s.history_labels().len();
    assert_eq!(execute(&mut s, "folder.randomizeSeeds", &json!({"layer": empty.0})).unwrap()["seeds"], 0);
    assert_eq!(s.history_labels().len(), steps);
}

#[test]
fn reset_restores_a_procedural_filter_input_by_name_after_reopening() {
    let (mut session, folder, fill) = folder_with_fill();
    let filter = id(execute(&mut session, "effect.add.filter", &json!({"layer": fill.0, "target": "content", "filter": "colorMatch"})).unwrap(), "effect");
    for (slot, seed) in [("input", 23), ("imageInput", 41)] {
        execute(&mut session, "layer.fill.set", &json!({"effect": filter.0, "input": slot, "procedural": "Clouds", "seed": seed})).unwrap();
    }
    let seed = |session: &Session, slot: &str| match &session.doc.project.effect(filter).unwrap().1.inputs[slot] {
        Source::Procedural { pattern, .. } => pattern.seed,
        other => panic!("expected procedural input, got {other:?}"),
    };

    execute(&mut session, "folder.randomizeSeeds", &json!({"layer": folder.0})).unwrap();
    assert_ne!(seed(&session, "imageInput"), 41);
    execute(&mut session, "layer.fill.set", &json!({"effect": filter.0, "input": "input", "clear": true})).unwrap();
    session.doc.project = serde_json::from_slice(&serde_json::to_vec(&session.doc.project).unwrap()).unwrap();
    execute(&mut session, "folder.contents.reset", &json!({"layer": folder.0})).unwrap();

    assert_eq!(seed(&session, "imageInput"), 41);
}

/// The seeds of fill layer `id`'s base color and roughness patterns and of its mask's warp filter
fn fill_seeds(s: &Session, id: LayerId) -> (u32, u32, u32) {
    let layer = s.doc.project.layer(id).unwrap();
    let LayerKind::Fill(fill) = &layer.kind else { panic!("not a fill") };
    let pattern = |channel| match fill.sources.get(&channel) {
        Some(Source::Procedural { pattern, .. }) => pattern.seed,
        other => panic!("not a pattern: {other:?}"),
    };
    let warp = layer.mask.as_ref().unwrap().effects.iter().find_map(|effect| match effect.kind {
        EffectKind::Filter(Filter::SourceWarp(warp)) => Some(warp.seed),
        _ => None,
    });
    (pattern(Channel::BaseColor), pattern(Channel::Roughness), warp.unwrap())
}

#[test]
fn the_contents_reset_puts_back_the_seeds_randomize_seeds_changed_in_one_undo_step() {
    let (mut s, folder, fill) = folder_with_fill();
    let before = fill_seeds(&s, fill);
    let pictured = base_colors(&s);
    assert!(!texturecraft_engine::folder_cmds::contents_changed(&s.doc.project, folder), "a new folder's contents are at their defaults");

    // Randomized twice, with a Scale laid on top: the reset still goes back to the seeds from before the first
    execute(&mut s, "folder.randomizeSeeds", &json!({"layer": folder.0})).unwrap();
    execute(&mut s, "folder.randomizeSeeds", &json!({"layer": folder.0})).unwrap();
    execute(&mut s, "folder.contents.set", &json!({"layer": folder.0, "scale": 2.0})).unwrap();
    let randomized = fill_seeds(&s, fill);
    assert!(randomized.0 != before.0 && randomized.2 != before.2, "{before:?} -> {randomized:?}");
    assert!(texturecraft_engine::folder_cmds::contents_changed(&s.doc.project, folder), "the reset is lit");

    // A saved positional record from before stable seed targets still resets the original slot
    let mut saved = serde_json::to_value(&s.doc.project).unwrap();
    let records = saved["layers"][folder.0.to_string()]["kind"]["Group"]["randomized"].as_array_mut().unwrap();
    assert!(!records.is_empty());
    for record in records {
        record.as_object_mut().unwrap().remove("target");
    }
    s.doc.project = serde_json::from_value(saved).unwrap();

    let steps = s.history_labels().len();
    execute(&mut s, "folder.contents.reset", &json!({"layer": folder.0})).unwrap();

    assert_eq!(s.history_labels().len(), steps + 1, "one undo step");
    assert_eq!(fill_seeds(&s, fill), before, "every seed is back");
    assert!(s.doc.project.contents_offset(folder).is_none(), "the Scale is dropped too");
    assert!(!texturecraft_engine::folder_cmds::contents_changed(&s.doc.project, folder), "nothing left to reset");

    s.update();

    assert_eq!(base_colors(&s), pictured, "the fill looks as it did before Randomize seeds");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(fill_seeds(&s, fill), randomized, "one undo brings the random seeds and the Scale back");
    assert_eq!(s.doc.project.contents_offset(folder).scale, 2.0);

    // A seed the artist set by hand after randomizing keeps their pick; the others go back
    execute(&mut s, "layer.fill.set", &json!({"layer": fill.0, "channel": "roughness", "procedural": "Clouds", "seed": 42})).unwrap();
    execute(&mut s, "folder.contents.reset", &json!({"layer": folder.0})).unwrap();
    let after = fill_seeds(&s, fill);
    assert_eq!((after.0, after.1, after.2), (before.0, 42, before.2));
}

/// A project saved before folders kept their randomized seeds opens with none and saves the folder as before
#[test]
fn a_folder_saved_without_randomized_seeds_opens_and_saves_unchanged() {
    let (s, folder, _) = folder_with_fill();
    let saved = serde_json::to_value(&s.doc.project.layers[&folder]).unwrap();

    assert!(saved.to_string().contains("\"children\"") && !saved.to_string().contains("randomized"), "nothing new is written: {saved}");

    let read: texturecraft_engine::core::Layer = serde_json::from_value(saved.clone()).unwrap();

    assert!(matches!(&read.kind, LayerKind::Group { randomized, .. } if randomized.is_empty()));
    assert_eq!(serde_json::to_value(&read).unwrap(), saved);
}

#[test]
fn reset_restores_original_effects_after_a_randomized_effect_is_duplicated() {
    let (mut session, folder, layer) = folder_with_fill();
    let original = session.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects.last().unwrap().id;
    let duplicate = |session: &mut Session| id(execute(session, "effect.duplicate", &json!({"effect": original.0})).unwrap(), "effect");
    let second = duplicate(&mut session);
    let seed = |session: &Session, effect| match session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Filter(Filter::SourceWarp(warp)) => warp.seed,
        _ => panic!("expected a library Warp filter"),
    };
    let before = seed(&session, original);
    execute(&mut session, "folder.randomizeSeeds", &json!({"layer": folder.0})).unwrap();
    let copied = duplicate(&mut session);
    let randomized = seed(&session, copied);

    execute(&mut session, "folder.contents.reset", &json!({"layer": folder.0})).unwrap();

    assert_ne!(randomized, before);
    assert_eq!(seed(&session, copied), randomized, "the copy was created after randomization");
    assert_eq!((seed(&session, original), seed(&session, second)), (before, before));
}

#[test]
fn contents_controls_follow_an_instanced_folder_without_randomizing_its_source() {
    let (mut session, source, _) = folder_with_fill();
    let project = &mut session.doc.project;
    let folder = project.alloc_id();
    let instance = project.alloc_id();
    let new_layer = |id, kind| texturecraft_engine::core::Layer::new(id, "Layer", kind, &Channel::DEFAULT_SET);
    project.insert_layer(0, new_layer(folder, LayerKind::folder()), None, false).unwrap();
    project.insert_layer(0, new_layer(instance, LayerKind::Instance { source }), Some(folder), true).unwrap();

    assert!(texturecraft_engine::folder_cmds::has_contents(&session.doc.project, folder));
    assert_eq!(execute(&mut session, "folder.randomizeSeeds", &json!({"layer": folder.0})).unwrap()["seeds"], 0);
}

#[test]
fn a_copied_folder_resets_only_its_own_randomized_seeds() {
    let (mut session, folder, fill) = folder_with_fill();
    let original = fill_seeds(&session, fill);
    execute(&mut session, "folder.randomizeSeeds", &json!({"layer": folder.0})).unwrap();
    let changed = fill_seeds(&session, fill);
    let copy = id(execute(&mut session, "layer.duplicate", &json!({"layer": folder.0})).unwrap(), "layer");
    let copied_fill = session.doc.project.layers[&copy].children()[0];

    execute(&mut session, "folder.contents.reset", &json!({"layer": copy.0})).unwrap();

    assert_ne!(original, changed);
    assert_eq!(fill_seeds(&session, copied_fill), original);
    assert_eq!(fill_seeds(&session, fill), changed);
}
