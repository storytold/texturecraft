//! Live bake preview: maps stream into the view while a bake runs, the view mode comes back,
//! painting is paused, a cancelled bake leaves the document alone, and the streamed pixels are
//! the ones a single bake makes.

use serde_json::{Value, json};
use texturecraft_engine::bake::Progress;
use texturecraft_engine::core::{BakeResolution, BakedMap, Channel};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::{BakeTask, PreviewShow, Session, ViewMode, execute};

const MAPS: [BakedMap; 4] = [BakedMap::Id, BakedMap::AmbientOcclusion, BakedMap::Curvature, BakedMap::Height];

fn session() -> Session {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for texture_set in &mut session.doc.project.texture_sets {
        texture_set.bake = Default::default();
        texture_set.bake.common.resolution = BakeResolution::R64;
        texture_set.bake.maps.ao.parameters.rays.rays = 8;
        texture_set.bake.maps.select(MAPS);
    }
    session
}

fn task(session: &Session) -> BakeTask {
    let mut task = session.bake_task(0).unwrap();
    task.tracer = None;
    task
}

fn order(session: &Session) -> Vec<BakedMap> {
    task(session).maps()
}

#[test]
fn streamed_maps_are_the_pixels_of_one_bake() {
    let session = session();
    let task = task(&session);
    let whole = task.run(&Progress::new()).unwrap();
    let mut streamed = Vec::new();
    let combined = task.run_streaming(&Progress::new(), &mut |map| streamed.push(map)).unwrap();
    assert_eq!(streamed.iter().map(|map| map.map).collect::<Vec<_>>(), order(&session));
    assert_eq!(streamed.iter().map(|map| (map.index, map.total)).collect::<Vec<_>>(), (0..MAPS.len()).map(|index| (index, MAPS.len())).collect::<Vec<_>>());
    assert_eq!((combined.resolution, combined.height), (whole.resolution, whole.height));
    assert_eq!(combined.maps.keys().collect::<Vec<_>>(), whole.maps.keys().collect::<Vec<_>>());
    for (map, pixels) in &whole.maps {
        assert!(combined.maps[map] == *pixels, "{map:?} differs between a streamed and a single bake");
    }
    for map in &streamed {
        assert!(*map.data == whole.maps[&map.map], "{:?} handed over differs from the bake", map.map);
    }
}

#[test]
fn the_view_follows_each_map_and_the_previous_mode_comes_back() {
    let mut session = session();
    session.view = ViewMode::Channel(Channel::Roughness);
    let before = session.view;
    let baked_before = session.doc.baked.get(&0).cloned();
    let maps = order(&session);
    session.bake_preview_begin(vec![(0, maps.clone())]);
    session.bake_preview_set_current(0);
    // Nothing is baked yet: the first map shows, neutral
    assert_eq!(session.view, ViewMode::MeshMap(maps[0]));
    assert!(session.bake_preview_baked(0).is_some_and(|baked| baked.maps.is_empty()));

    let task = task(&session);
    let mut shown = Vec::new();
    let mut stages = Vec::new();
    let output = task
        .run_streaming(&Progress::new(), &mut |map| {
            session.bake_preview_map(0, map);
            shown.push(session.view);
            stages.push(session.bake_preview.as_ref().and_then(|preview| preview.stage()).map(|stage| (stage.position, stage.total)));
        })
        .unwrap();
    assert_eq!(shown, maps.iter().map(|map| ViewMode::MeshMap(*map)).collect::<Vec<_>>(), "the view follows the newest map");
    assert_eq!(stages, [Some((2, 4)), Some((3, 4)), Some((4, 4)), None], "the stage names the next map");
    assert_eq!(session.doc.baked.get(&0).cloned(), baked_before, "the document is untouched until the bake is applied");
    let undo_before = session.undo_label().map(str::to_owned);

    session.apply_bake(0, output).unwrap();
    assert_ne!(session.undo_label().map(str::to_owned), undo_before, "applying the bake is one undo step");
    session.bake_preview_end();
    assert_eq!(session.view, before, "the previous view mode is restored");
    assert!(!session.bake_preview_active());
    assert!(maps.iter().all(|map| session.doc.baked[&0].maps.contains_key(map)));
}

#[test]
fn pinning_material_and_follow() {
    let mut session = session();
    let maps = order(&session);
    session.bake_preview_begin(vec![(0, maps.clone())]);
    session.bake_preview_set_current(0);
    let task = task(&session);
    let mut count = 0;
    task.run_streaming(&Progress::new(), &mut |map| {
        session.bake_preview_map(0, map);
        count += 1;
        match count {
            // Pin the finished map: later maps do not take the view
            1 => session.bake_preview_show(PreviewShow::Pin(maps[0])).unwrap(),
            2 => {
                assert_eq!(session.view, ViewMode::MeshMap(maps[0]));
                session.bake_preview_show(PreviewShow::Material).unwrap();
            }
            3 => {
                assert_eq!(session.view, ViewMode::Material);
                session.bake_preview_show(PreviewShow::Follow).unwrap();
                assert_eq!(session.view, ViewMode::MeshMap(maps[2]));
            }
            _ => {}
        }
    })
    .unwrap();
    // A mesh map picked in the viewport pins it; another mode is what comes back at the end
    execute(&mut session, "viewport.view", &json!({ "mode": "map:height" })).unwrap();
    assert_eq!(session.bake_preview.as_ref().map(|preview| preview.show), Some(PreviewShow::Pin(BakedMap::Height)));
    execute(&mut session, "viewport.view", &json!({ "mode": "uvChecker" })).unwrap();
    assert_eq!(session.view, ViewMode::UvChecker);
    session.bake_preview_end();
    assert_eq!(session.view, ViewMode::UvChecker);
}

#[test]
fn unbaked_sets_show_neutral_and_the_control_command_reports() {
    let mut session = session();
    assert_eq!(execute(&mut session, "bake.preview", &Value::Null).unwrap()["active"], false);
    assert!(execute(&mut session, "bake.preview", &json!({ "show": "material" })).is_err(), "nothing to show when no bake runs");
    session.bake_preview_begin(vec![(0, order(&session)), (1, order(&session))]);
    session.bake_preview_set_current(0);
    assert!(session.bake_preview_baked(1).is_some_and(|baked| baked.maps.is_empty()), "a queued set has nothing yet");
    assert!(session.bake_preview_baked(7).is_none(), "a set outside the bake shows its document maps");
    let reply = execute(&mut session, "bake.preview", &json!({ "show": "map:ambientOcclusion" })).unwrap();
    assert_eq!(
        (reply["active"].clone(), reply["show"].clone(), reply["shown"].clone()),
        (json!(true), json!("map:ambientOcclusion"), json!("ambientOcclusion"))
    );
    assert!(execute(&mut session, "bake.preview", &json!({ "show": "nonsense" })).is_err());
    assert_eq!(texturecraft_engine::inspect::session(&session)["bakePreview"]["active"], true);
}

#[test]
fn painting_is_refused_while_a_bake_shows() {
    let mut session = session();
    session.bake_preview_begin(vec![(0, order(&session))]);
    let error = session.begin_stroke(Vec2::new(10.0, 10.0), Vec2::new(100.0, 100.0), 1.0).unwrap_err().to_string();
    assert_eq!(error, texturecraft_engine::bake_preview::PAINTING_PAUSED);
    assert!(session.begin_stroke_uv(Vec2::new(0.5, 0.5), 1.0).is_err());
    assert!(session.polygon_fill(Vec2::new(10.0, 10.0), Vec2::new(100.0, 100.0)).is_err());
    assert!(!session.stroke_active());
    session.bake_preview_end();
    assert!(session.painting_blocked().is_none());
}

#[test]
fn cancelling_mid_bake_leaves_the_document_and_the_shown_maps_whole() {
    let mut session = session();
    let before = session.doc.baked.get(&0).cloned();
    let maps = order(&session);
    let full = task(&session).run(&Progress::new()).unwrap();
    session.bake_preview_begin(vec![(0, maps.clone())]);
    session.bake_preview_set_current(0);
    let progress = Progress::new();
    let task = task(&session);
    let result = task.run_streaming(&progress, &mut |map| {
        if map.index == 1 {
            progress.cancel();
        }
        session.bake_preview_map(0, map);
    });
    assert!(result.is_err(), "a cancelled bake fails");
    // The two maps that landed are whole and equal to a full bake's; the others are still neutral
    let shown = session.bake_preview_baked(0).unwrap();
    assert_eq!(shown.maps.len(), 2);
    for (map, data) in &shown.maps {
        assert!(**data == full.maps[map], "{map:?}");
    }
    // What the app does on cancel: end the preview; the document keeps what it had
    session.bake_preview_end();
    assert_eq!(session.doc.baked.get(&0).cloned(), before);
    assert!(session.bake_preview_baked(0).is_none());
}
