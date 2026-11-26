use egui::Context;

use super::{OrbJob, OrbResult, newest_per_place};

/// The thread that renders orbs
pub(super) struct Worker {
    /// Each job comes with the context to repaint when its picture is ready
    pub(super) jobs: std::sync::mpsc::Sender<(OrbJob, Context)>,
    pub(super) results: std::sync::mpsc::Receiver<OrbResult>,
}

impl Worker {
    pub(super) fn start() -> Option<Worker> {
        let (jobs, job_rx) = std::sync::mpsc::channel::<(OrbJob, Context)>();
        let (result_tx, results) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("texturecraft-material-orbs".into()).spawn(move || {
            // Its own pool keeps orbs from queuing ahead of painting and compositing
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1));
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("texturecraft-orb-{i}")).build();
            let mut waiting: Vec<OrbJob> = Vec::new();
            let mut repaint: Option<Context> = None;
            loop {
                if waiting.is_empty() {
                    // Idle, the thread holds no context: the context owns the job sender, so a held one would keep both alive
                    drop(repaint.take());
                    let Ok((job, ctx)) = job_rx.recv() else { break };

                    waiting.push(job);
                    repaint = Some(ctx);
                }
                let previous_len = waiting.len();
                for (job, ctx) in job_rx.try_iter() {
                    waiting.push(job);
                    repaint = Some(ctx);
                }
                if waiting.len() > previous_len {
                    for (key, request) in newest_per_place(&mut waiting) {
                        if result_tx.send(OrbResult::Dropped { key, request }).is_err() {
                            return;
                        }
                    }
                }

                // Every quick picture before any full one, so a list fills in at once
                let next = waiting.iter().position(|job| job.quick).unwrap_or(0);
                if next >= waiting.len() {
                    continue;
                }

                let mut job = waiting.remove(next);
                let image = match &pool {
                    Ok(pool) => pool.install(|| job.picture()),
                    Err(_) => job.picture(),
                };
                let full = !job.quick;
                if result_tx.send(OrbResult::Picture { key: job.key, request: job.request, full, image }).is_err() {
                    return;
                }

                if let Some(ctx) = &repaint {
                    ctx.request_repaint();
                }
                if !full {
                    job.quick = false;
                    waiting.push(job);
                }
            }
        });
        match spawned {
            Ok(_) => Some(Worker { jobs, results }),
            Err(e) => {
                log::warn!("could not start the material orb worker: {e}");
                None
            }
        }
    }
}
