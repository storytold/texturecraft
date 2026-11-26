//! A bake job belongs to the mesh it started on: when `file.changeMesh` (or a reimport) swaps
//! the mesh meanwhile, the job is cancelled and its result is dropped.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui_kittest::Harness;
use texturecraft_engine::bake::Progress;
use texturecraft_ui_egui::{BakeJob, RunningBake, TextureCraftApp};

/// An app with a running bake job (its worker never answers) started on mesh `mesh_id`.
fn with_running_job(mesh_id: impl FnOnce(u64) -> u64, check: impl FnOnce(&Harness<'static, TextureCraftApp>, &Progress)) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(4);
    let progress = Arc::new(Progress::new());
    let (_result_sender, receiver) = std::sync::mpsc::channel();
    let (_map_sender, maps) = std::sync::mpsc::channel();
    let job_mesh_id = mesh_id(harness.state().session.mesh_id);
    let running =
        RunningBake { set_index: 0, progress: progress.clone(), receiver, maps, maps_done: Arc::new(std::sync::atomic::AtomicUsize::new(0)), maps_total: 1 };
    harness.state_mut().bake = Some(BakeJob { queue: vec![], current: Some(running), total: 1, mesh_id: job_mesh_id, only: None });

    harness.step();
    check(&harness, &progress);
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
