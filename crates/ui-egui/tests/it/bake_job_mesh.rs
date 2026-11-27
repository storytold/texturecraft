//! A bake job belongs to the mesh it started on: when `file.changeMesh` or a reimport swaps the mesh meanwhile, the job is cancelled and its result dropped; a cancel keeps the maps it finished

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

use egui_kittest::Harness;
use texturecraft_engine::StreamedMap;
use texturecraft_engine::bake::Progress;
use texturecraft_engine::core::BakedMap;
use texturecraft_ui_egui::{BakeJob, RunningBake, TextureCraftApp};

/// An app with a running bake job (its worker never answers) started on mesh `mesh_id`
fn with_running_job(mesh_id: impl FnOnce(u64) -> u64, check: impl FnOnce(&Harness<'static, TextureCraftApp>, &Progress)) {
    let (mut harness, progress, _senders) = running_job(mesh_id);
    harness.step();
    check(&harness, &progress);
}

/// The ends a test plays the bake worker with: its result (never sent) and its finished maps
type WorkerSenders = (Sender<texturecraft_engine::Result<texturecraft_engine::StreamedBake>>, Sender<StreamedMap>);

/// An app with a running bake job of set 0 started on mesh `mesh_id`, whose worker sends only what the test sends
fn running_job(mesh_id: impl FnOnce(u64) -> u64) -> (Harness<'static, TextureCraftApp>, Arc<Progress>, WorkerSenders) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(4);
    let progress = Arc::new(Progress::new());
    let (result_sender, receiver) = std::sync::mpsc::channel();
    let (map_sender, maps) = std::sync::mpsc::channel();
    let job_mesh_id = mesh_id(harness.state().session.mesh_id);
    let maps_done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let running = RunningBake {
        set_index: 0,
        progress: progress.clone(),
        receiver,
        maps,
        maps_done,
        maps_total: 1,
        finished: Default::default(),
        sources: Default::default(),
        painted_stroke: 0,
    };
    harness.state_mut().bake = Some(BakeJob { queue: vec![], current: Some(running), total: 1, mesh_id: job_mesh_id, only: None, rebake: None });
    (harness, progress, (result_sender, map_sender))
}

#[test]
fn a_job_on_a_swapped_mesh_is_cancelled_and_dropped() {
    with_running_job(
        |current| current + 1,
        |harness, progress| {
            assert!(harness.state().bake.is_none(), "the job is dropped");
            assert!(progress.cancel.load(Ordering::Relaxed), "and its worker is told to stop");
        },
    );
}

#[test]
fn a_job_on_the_current_mesh_keeps_running() {
    with_running_job(
        |current| current,
        |harness, progress| {
            assert!(harness.state().bake.is_some(), "the job stays");
            assert!(!progress.cancel.load(Ordering::Relaxed));
        },
    );
}

#[test]
fn a_cancel_keeps_the_maps_the_worker_sent_before_any_frame_took_them() {
    let (mut harness, _progress, (result_sender, map_sender)) = running_job(|current| current);
    let data = Arc::new(vec![0.5; 64 * 64]);
    let normal_before = harness.state().session.baked(0).and_then(|baked| baked.maps.get(&BakedMap::Normal).cloned());
    assert!(normal_before.is_some(), "the sample's Normal map is baked before the job");
    for (index, map) in [BakedMap::Curvature, BakedMap::AmbientOcclusion].into_iter().enumerate() {
        map_sender.send(StreamedMap { map, index, total: 2, resolution: 64, height: 64, data: data.clone() }).unwrap();
    }
    result_sender.send(Err(texturecraft_engine::EngineError::BakeCancelled)).unwrap();

    harness.state_mut().cancel_bake();
    let baked = harness.state().session.baked(0).unwrap();
    for map in [BakedMap::Curvature, BakedMap::AmbientOcclusion] {
        assert!(baked.maps.get(&map).is_some_and(|kept| Arc::ptr_eq(kept, &data)), "{map:?} is kept");
    }
    let normal = baked.maps.get(&BakedMap::Normal);
    assert!(normal.zip(normal_before.as_ref()).is_some_and(|(now, before)| Arc::ptr_eq(now, before)), "a map the worker never sent keeps the one baked before");
}

#[test]
fn a_cancel_does_not_keep_maps_from_a_worker_that_already_failed() {
    let (mut harness, _progress, (result_sender, map_sender)) = running_job(|current| current);
    let map = BakedMap::Normal;
    let before = harness.state().session.baked(0).unwrap().maps[&map].clone();
    let data = Arc::new(vec![0.5; 64 * 64 * map.components()]);
    map_sender.send(StreamedMap { map, index: 0, total: 2, resolution: 64, height: 64, data }).unwrap();
    result_sender.send(Err(texturecraft_engine::EngineError::Other("the second map failed".into()))).unwrap();

    harness.state_mut().cancel_bake();

    let after = &harness.state().session.baked(0).unwrap().maps[&map];
    assert!(Arc::ptr_eq(&before, after), "the failed set leaves the previous bake in place");
}
