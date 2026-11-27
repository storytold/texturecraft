//! The Baking Log's record of the latest bake, map by map

use std::collections::BTreeSet;

use serde_json::json;
use texturecraft_engine::bake::{BakeHighState, BakeLog, BakeMapState, NamePair};
use texturecraft_engine::compose::BakedMaps;
use texturecraft_engine::core::{BakedMap, MeshMatchMode};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute, sample_session};

#[test]
fn bake_run_lists_every_checked_map_of_the_set_as_baked() {
    let mesh = primitives::builtin("plane").unwrap();
    let mut session = Session::new_project("t", mesh, "builtin:plane", None, 256).unwrap();

    execute(&mut session, "bake.run", &json!({ "resolution": 64, "maps": ["opacity", "position"], "dilation": 0 })).unwrap();

    let set_name = &session.doc.project.texture_sets[0].name;
    let rows = &session.bake_log.maps;
    let maps: BTreeSet<BakedMap> = rows.iter().map(|row| row.map).collect();

    assert_eq!(maps, BTreeSet::from([BakedMap::Position, BakedMap::Opacity]));
    assert!(rows.iter().all(|row| row.state == BakeMapState::Baked && &row.set_name == set_name), "{rows:?}");
    assert!(!session.bake_log.is_running());
}

#[test]
fn a_canceled_bake_keeps_the_finished_sets_and_cancels_the_rest() {
    let mut session = sample_session(256, 64).unwrap();
    for set in &mut session.doc.project.texture_sets {
        set.bake.maps.select([BakedMap::Normal, BakedMap::Curvature]);
    }
    let log = &mut session.bake_log;
    log.begin(&session.doc.project, &session.doc.bake_meshes.high, &[0, 1]);

    log.set_started(0);
    log.map_baked(0, BakedMap::Normal);
    log.map_baked(0, BakedMap::Curvature);
    log.set_finished(0);
    log.set_started(1);
    log.map_baked(1, BakedMap::Normal);

    let baking: Vec<_> = log.maps.iter().filter(|row| row.state == BakeMapState::Baking).map(|row| (row.texture_set, row.map)).collect();
    assert_eq!(baking, [(1, BakedMap::Curvature)], "the next map of the running set is the one in progress");

    log.cancel(Some(1));

    let state_of = |set: usize| log.maps.iter().filter(move |row| row.texture_set == set).map(|row| row.state.clone()).collect::<Vec<_>>();
    assert!(state_of(0).iter().all(|state| *state == BakeMapState::Baked), "{:?}", log.maps);
    assert!(state_of(1).iter().all(|state| *state == BakeMapState::Canceled), "maps of the set not kept are canceled: {:?}", log.maps);
    assert!(!log.is_running());
}

#[test]
fn bake_log_reports_each_map_with_its_state_and_a_failures_reason() {
    let mesh = primitives::builtin("plane").unwrap();
    let mut session = Session::new_project("t", mesh, "builtin:plane", None, 256).unwrap();
    execute(&mut session, "bake.run", &json!({ "resolution": 64, "maps": ["opacity", "position"], "dilation": 0 })).unwrap();

    let log = execute(&mut session, "bake.log", &json!({})).unwrap();

    assert_eq!(log["running"], false);
    let maps: BTreeSet<&str> = log["maps"].as_array().unwrap().iter().map(|row| row["map"].as_str().unwrap()).collect();
    assert_eq!(maps, BTreeSet::from(["opacity", "position"]));
    assert!(log["maps"].as_array().unwrap().iter().all(|row| row["state"] == "baked" && row["textureSet"] == 0 && row.get("reason").is_none()), "{log}");

    session.bake_log.set_failed(0, "The high poly file could not be read.");
    let log = execute(&mut session, "bake.log", &json!({})).unwrap();

    assert!(log["maps"].as_array().unwrap().iter().all(|row| row["state"] == "failed" && row["reason"] == "The high poly file could not be read."), "{log}");
}

#[test]
fn a_bake_lists_the_high_polys_each_set_loads_and_skips_a_set_baking_from_its_low_poly() {
    let mut session = sample_session(256, 64).unwrap();
    session.add_high_poly_bytes("ball_high.obj", b"o Ball_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    session.doc.project.texture_sets[1].bake.high_poly.use_low_as_high = true;

    execute(&mut session, "bake.run", &json!({ "ts": [0, 1], "resolution": 64, "maps": ["opacity"], "dilation": 0 })).unwrap();

    let highs: Vec<_> = session.bake_log.highs.iter().map(|row| (row.texture_set, row.file.as_str(), row.state.clone())).collect();
    assert_eq!(highs, [(0, "ball_high.obj", BakeHighState::Loaded)]);
    let log = execute(&mut session, "bake.log", &json!({})).unwrap();
    assert_eq!(log["highPolys"][0]["state"], "loaded", "{log}");
}

#[test]
fn a_set_that_fails_before_its_high_polys_are_ready_marks_them_failed_and_a_cancel_the_rest_canceled() {
    let mut session = sample_session(256, 64).unwrap();
    session.add_high_poly_bytes("ball_high.obj", b"o Ball_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    let log = &mut session.bake_log;
    log.begin(&session.doc.project, &session.doc.bake_meshes.high, &[0, 1]);
    let state = |log: &BakeLog, set: usize| log.highs.iter().find(|high| high.texture_set == set).map(|high| high.state.clone());

    log.set_started(0);

    assert_eq!(state(log, 0), Some(BakeHighState::Loading));
    assert!(log.is_loading_high_polys());

    log.set_failed(0, "The cage file could not be read.");
    log.set_started(1);
    log.cancel(Some(1));

    assert_eq!(state(log, 0), Some(BakeHighState::Failed("The cage file could not be read.".into())));
    assert_eq!(state(log, 1), Some(BakeHighState::Canceled));
    assert!(!log.is_loading_high_polys());
}

#[test]
fn a_bake_matching_by_name_lists_its_pairs_and_only_the_high_parts_no_set_pairs_with() {
    let obj = "o Box_low\nusemtl Body\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n\
               o Lid_low\nusemtl Lid\nv 2 0 0\nv 3 0 0\nv 2 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 4/4 5/5 6/6\n";
    let mut session = Session::from_mesh_bytes("sets.obj", obj.as_bytes(), 128, "sets").unwrap();
    session
        .add_high_poly_bytes("high.obj", b"o Box_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no Extra_high\nv 2 0 0\nv 3 0 0\nv 2 1 0\nf 4 5 6\n".to_vec())
        .unwrap();
    session.begin_bake_log(&[0, 1]);

    assert_eq!(session.bake_log.matching, None, "matching \"Always\" lists no pairs");

    for set in &mut session.doc.project.texture_sets {
        set.bake.high_poly.match_mode = MeshMatchMode::ByMeshName;
    }
    session.begin_bake_log(&[0, 1]);

    let pair = |low: Option<&str>, high: Option<&str>| NamePair { low: low.map(Into::into), high: high.map(Into::into) };
    let matching = session.bake_log.matching.clone().unwrap();
    assert_eq!(
        matching.pairs,
        [pair(Some("Box_low"), Some("Box_high")), pair(Some("Lid_low"), None), pair(None, Some("Extra_high"))],
        "Box_high is paired by the other set"
    );
    assert_eq!(matching.issue_count(), 2);
    let log = execute(&mut session, "bake.log", &json!({})).unwrap();
    assert_eq!(log["matching"]["pairs"][1], json!({ "low": "Lid_low", "high": null }), "{log}");
}

#[test]
fn a_high_part_counts_as_unused_only_when_no_set_of_the_bake_matches_always() {
    let obj = "o Box_low\nusemtl Body\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n\
               o Lid_low\nusemtl Lid\nv 2 0 0\nv 3 0 0\nv 2 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 4/4 5/5 6/6\n";
    let mut session = Session::from_mesh_bytes("sets.obj", obj.as_bytes(), 128, "sets").unwrap();
    session
        .add_high_poly_bytes("high.obj", b"o Box_high\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no Extra_high\nv 2 0 0\nv 3 0 0\nv 2 1 0\nf 4 5 6\n".to_vec())
        .unwrap();
    session.doc.project.texture_sets[0].bake.high_poly.match_mode = MeshMatchMode::ByMeshName;

    session.begin_bake_log(&[0, 1]);

    let matching = session.bake_log.matching.clone().unwrap();
    assert!(matching.pairs.iter().all(|pair| pair.low.is_some()), "the set matching \"Always\" bakes from Extra_high: {:?}", matching.pairs);
    assert_eq!(matching.issue_count(), 0);
}

#[test]
fn a_canceled_bake_at_a_new_output_size_keeps_none_of_its_maps_so_the_sets_others_stay() {
    let mut session = sample_session(256, 64).unwrap();
    let before = session.baked(0).cloned().unwrap();
    let resolution = before.resolution * 2;
    let data = std::sync::Arc::new(vec![0.5; (resolution * resolution) as usize * BakedMap::Normal.components()]);
    let finished = BakedMaps { resolution, maps: [(BakedMap::Normal, data)].into() };

    let kept = session.keep_canceled_bake(0, finished, &Default::default(), 0).unwrap();

    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(session.baked(0).map(|baked| baked.maps.keys().copied().collect::<Vec<_>>()), Some(before.maps.keys().copied().collect()));
    assert_eq!(session.baked(0).map(|baked| baked.resolution), Some(before.resolution));
}

#[test]
fn a_canceled_bake_keeps_the_out_of_date_mark_of_a_map_it_did_not_finish() {
    let mut session = sample_session(256, 64).unwrap();
    execute(&mut session, "bake.run", &json!({ "resolution": 64, "maps": ["id", "position"], "dilation": 0 })).unwrap();
    execute(&mut session, "bake.settings", &json!({ "positionMode": "boundingBox" })).unwrap();
    let task = session.bake_task_for(0, &[BakedMap::Id, BakedMap::Position]).unwrap();
    let stored = session.baked(0).cloned().unwrap();
    let finished = BakedMaps { resolution: stored.resolution, maps: [(BakedMap::Id, stored.maps[&BakedMap::Id].clone())].into() };

    let kept = session.keep_canceled_bake(0, finished, &task.sources, task.painted_stroke).unwrap();

    assert_eq!(kept, [BakedMap::Id]);
    assert!(!session.bake_changes(0, BakedMap::Position).is_empty(), "Position still has its old pixels, so it stays out of date");
}

#[test]
fn a_canceled_bakes_log_line_names_the_set_as_the_texture_set_list_does() {
    let mut session = sample_session(256, 64).unwrap();
    let stored = session.baked(0).cloned().unwrap();
    let (map, data) = stored.maps.iter().next().map(|(map, data)| (*map, data.clone())).unwrap();
    let finished = BakedMaps { resolution: stored.resolution, maps: [(map, data)].into() };

    session.keep_canceled_bake(0, finished, &Default::default(), 0).unwrap();

    let name = session.doc.project.texture_sets[0].name.clone();
    let line = session.log.iter().find(|line| line.contains("canceled bake")).map(|line| line.message.clone()).unwrap();
    assert!(line.contains(&format!("texture set {name} ")), "the first set is not numbered 0: {line}");
}
