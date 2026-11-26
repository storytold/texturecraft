//! The map preview: while Mesh Map Settings shows a map's settings, the 3D view shows that map on the model, re-baked small on a worker as the settings change

use crate::bake_scene::SharedHandle;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::TryRecvError;

use texturecraft_engine::StreamedMap;
use texturecraft_engine::bake::Progress;
use texturecraft_engine::core::{BakePaintTarget, BakeResolution, BakedMap};

use super::live_preview_target::LivePreviewTarget;
use super::preview_input::PreviewInput;
use super::running_preview::RunningPreview;
use crate::TextureCraftApp;
use crate::docking::Panel;
use crate::i18n::{baked_map_label, tr, trf};
use crate::state::Mode;

/// Sizes a dragged slider's previews step through, smallest first
const DRAG_SIZES: [BakeResolution; 5] = [BakeResolution::R64, BakeResolution::R128, BakeResolution::R256, BakeResolution::R512, BakeResolution::R1024];

/// A drag preview this quick bakes one size up next time
const DRAG_STEP_UP_SECONDS: f64 = 0.07;

/// A drag preview this slow bakes one size down next time
const DRAG_STEP_DOWN_SECONDS: f64 = 0.14;

/// How often the view looks for a finished preview: about one frame at 60 Hz, so it shows at once
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// The map preview's state: what it shows, what its picture was made from (and the session revision and size it was last checked at), the bake running for it, the drag change waiting for it, whether a drag still needs the full-size bake, whether its last bake could not start, and each map's drag size on this machine
#[derive(Default)]
pub(crate) struct LivePreview {
    shown: Option<LivePreviewTarget>,
    made_from: Option<PreviewInput>,
    checked_at: Option<(u64, BakeResolution)>,
    cannot_bake: bool,
    running: Option<RunningPreview>,
    drag_waiting: bool,
    refine_on_release: bool,
    drag_sizes: BTreeMap<BakedMap, BakeResolution>,
}

impl LivePreview {
    /// The map the preview shows now, if any
    pub(crate) fn shown(&self) -> Option<BakedMap> {
        self.shown.map(|target| target.map)
    }

    /// Stop the preview and give the 3D view back; a real bake calls this before it takes the view
    pub(crate) fn release(&mut self, session: &mut texturecraft_engine::Session) {
        self.cancel();
        self.refine_on_release = false;
        self.made_from = None;
        self.checked_at = None;
        self.cannot_bake = false;
        if self.shown.take().is_some() {
            session.bake_preview_end();
        }
    }

    fn cancel(&mut self) {
        self.drag_waiting = false;
        self.running = None;
    }

    /// The size `map` bakes at while a slider drags: it starts small, follows how fast this machine bakes it, and never passes `chosen`
    fn drag_size(&self, map: BakedMap, chosen: BakeResolution) -> BakeResolution {
        let learned = self.drag_sizes.get(&map).copied().unwrap_or_else(|| first_drag_size(map));
        if learned.pixels() > chosen.pixels() { chosen } else { learned }
    }

    /// A drag preview of `map` at `size` took `seconds`: step its drag size up or down one size, never past `chosen`
    fn learn_drag_size(&mut self, map: BakedMap, size: BakeResolution, seconds: f64, chosen: BakeResolution) {
        let Some(index) = DRAG_SIZES.iter().position(|step| *step == size) else { return };

        let next = if seconds < DRAG_STEP_UP_SECONDS {
            DRAG_SIZES.get(index + 1).copied().filter(|bigger| bigger.pixels() <= chosen.pixels()).unwrap_or(size)
        } else if seconds > DRAG_STEP_DOWN_SECONDS {
            index.checked_sub(1).and_then(|smaller| DRAG_SIZES.get(smaller).copied()).unwrap_or(size)
        } else {
            size
        };
        self.drag_sizes.insert(map, next);
    }
}

/// The texture set and the map whose settings a Mesh Map Settings panel in front shows, in Bake mode
pub(crate) fn map_settings_shown(app: &TextureCraftApp) -> Option<LivePreviewTarget> {
    let set = app.session.ts();
    let settings = &app.session.doc.project.texture_sets.get(set)?.bake;
    let shows = app.ui.mode == Mode::Baking && app.ui.dock.is_visible(Panel::MeshMapSettings);
    shows.then(|| LivePreviewTarget { set, map: super::selected_map(app.ui.selected_baked_map, settings) })
}

/// What the preview should show: the map whose settings show, while the preview is on and no real bake runs
fn wanted(app: &TextureCraftApp) -> Option<LivePreviewTarget> {
    map_settings_shown(app).filter(|_| app.ui.bake_visualization.map_preview && app.bake.is_none())
}

/// A slider dragged in Mesh Map Settings changed a setting: a running drag bake finishes and shows before the newest value bakes, so pictures keep coming while the slider moves
pub(crate) fn slider_dragged(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(target) = app.live_preview.shown else { return };

    app.live_preview.refine_on_release = true;

    // Web builds bake on the UI thread, so they wait for the release; a set that can't bake waits too
    if cfg!(target_arch = "wasm32") || app.live_preview.cannot_bake {
        return;
    }

    if app.live_preview.running.as_ref().is_some_and(|running| running.dragging) {
        app.live_preview.drag_waiting = true;
    } else {
        let size = app.live_preview.drag_size(target.map, app.ui.bake_visualization.map_preview_size);
        start(app, ctx, target, size, true);
    }
}

/// Once a frame: follow what the preview should show, bake again when its picture is out of date, and take a finished bake
pub(crate) fn poll(app: &mut TextureCraftApp, ctx: &egui::Context) {
    // A new session (a project opened, the mesh replaced) has no preview layer: start over
    if app.live_preview.shown.is_some() && !app.session.bake_preview_active() {
        app.live_preview.release(&mut app.session);
    }

    let wanted = wanted(app);
    if wanted != app.live_preview.shown {
        app.live_preview.release(&mut app.session);
        if let Some(target) = wanted {
            show(app, ctx, target);
        }
    }

    // Refine after release and refresh after external edits
    if let Some(target) = app.live_preview.shown {
        let released = app.live_preview.refine_on_release && ctx.dragged_id().is_none();
        let chosen = app.ui.bake_visualization.map_preview_size;

        // Compare input snapshots only after an edit or a size change
        let check = (app.session.revision, chosen);
        let unchecked = app.live_preview.checked_at != Some(check);
        let out_of_date = unchecked && !app.live_preview.refine_on_release && app.live_preview.made_from != input(app, target, chosen);
        if unchecked && !app.live_preview.refine_on_release {
            app.live_preview.checked_at = Some(check);
        }
        if released || out_of_date {
            app.live_preview.refine_on_release = false;
            start(app, ctx, target, chosen, false);
        }
    }

    take_finished(app, ctx);
}

/// Show a finished preview bake, learn the drag size from it, and start the drag change that waited for it; a failed one goes to the Log
fn take_finished(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(running) = &app.live_preview.running else { return };

    ctx.request_repaint_after(POLL_INTERVAL);
    let finished = match running.receiver.try_recv() {
        Err(TryRecvError::Empty) => return,
        Ok(finished) => finished,
        Err(TryRecvError::Disconnected) => Err(texturecraft_engine::EngineError::Other(tr("The map preview worker stopped").into())),
    };
    let Some(running) = app.live_preview.running.take() else { return };

    // A failure keeps the old picture and says why
    let mut output = match finished {
        Ok(output) => output,
        Err(error) => {
            let map = tr(baked_map_label(running.target.map));
            app.session.log.warn("Baker", trf("The {map} preview failed: {error}", &[("map", &map), ("error", &error.to_string())]));
            app.live_preview.drag_waiting = false;
            return;
        }
    };

    if running.dragging {
        let seconds = ctx.input(|input| input.time) - running.started;
        app.live_preview.learn_drag_size(running.target.map, running.size, seconds, app.ui.bake_visualization.map_preview_size);
    }
    if let Some(data) = output.maps.remove(&running.target.map) {
        let streamed = StreamedMap { map: running.target.map, index: 0, total: 1, resolution: output.resolution, height: output.height, data: Arc::new(data) };
        app.session.bake_preview_map(running.target.set, streamed);
    }

    // The newest drag change waited for this picture
    if std::mem::take(&mut app.live_preview.drag_waiting) && app.live_preview.refine_on_release {
        let size = app.live_preview.drag_size(running.target.map, app.ui.bake_visualization.map_preview_size);
        start(app, ctx, running.target, size, true);
    }
}

/// Start showing `target`: the map's last real bake at once when there is one (it may predate the settings), then a preview bake at the chosen size
fn show(app: &mut TextureCraftApp, ctx: &egui::Context, target: LivePreviewTarget) {
    app.session.bake_preview_begin(vec![(target.set, vec![target.map])]);
    app.live_preview.shown = Some(target);

    let baked = app.session.baked(target.set).and_then(|baked| baked.maps.get(&target.map).map(|data| (baked.resolution, data.clone())));
    if let Some((resolution, data)) = baked {
        let height = (data.len() / target.map.components()).checked_div(resolution as usize).unwrap_or(0) as u32;
        app.session.bake_preview_map(target.set, StreamedMap { map: target.map, index: 0, total: 1, resolution, height, data });
    }
    start(app, ctx, target, app.ui.bake_visualization.map_preview_size, false);
}

/// What a preview of `target` at `size` would be made from now
fn input(app: &TextureCraftApp, target: LivePreviewTarget, size: BakeResolution) -> Option<PreviewInput> {
    let set = app.session.doc.project.texture_sets.get(target.set)?;
    let meshes = &app.session.doc.bake_meshes;
    let painted = [BakePaintTarget::Skew, BakePaintTarget::Cage].map(|kind| SharedHandle::of(app.session.doc.bake_paint.get(&(target.set, kind))));
    let previous_pixels = app.session.doc.baked.get(&target.set).filter(|_| !set.bake_excluded_tiles.is_empty()).and_then(|baked| baked.maps.get(&target.map));

    Some(PreviewInput {
        settings: set.bake.clone(),
        mesh_id: app.session.mesh_id,
        uv_tiles: set.uv_tiles.clone(),
        excluded_tiles: set.bake_excluded_tiles.clone(),
        tile_resolutions: set.tile_resolutions.clone(),
        high: SharedHandle::of(meshes.merged_high().map(|source| &source.mesh)),
        cage: SharedHandle::of(meshes.cage.as_ref().map(|source| &source.mesh)),
        previous_pixels: SharedHandle::of(previous_pixels),
        painted,
        size,
    })
}

/// Bake `target`'s map alone at `size` on the preview pool (`dragging`: for a slider still moving), cancelling the preview bake still running
fn start(app: &mut TextureCraftApp, ctx: &egui::Context, target: LivePreviewTarget, size: BakeResolution, dragging: bool) {
    app.live_preview.cancel();
    app.live_preview.made_from = input(app, target, size);
    let mut task = match app.session.bake_map_preview_task(target.set, target.map) {
        Ok(task) => task,
        Err(error) => {
            // Log only the first failure until a bake can start
            if !app.live_preview.cannot_bake {
                let map = tr(baked_map_label(target.map));
                app.session.log.warn("Baker", trf("The {map} preview can't bake: {error}", &[("map", &map), ("error", &error.to_string())]));
            }
            app.live_preview.cannot_bake = true;
            return;
        }
    };
    app.live_preview.cannot_bake = false;

    let own = task.settings.common.resolution;
    let size = if own.pixels() < size.pixels() { own } else { size };
    task.settings.common.resolution = size;
    task.settings.common.resolution_height = None;

    let progress = Arc::new(Progress::new());
    let worker_progress = progress.clone();

    let pending = Arc::new(Mutex::new(Some(task)));
    let worker_task = pending.clone();
    let run = move || {
        let task = worker_task.lock().unwrap_or_else(|error| error.into_inner()).take();
        let task = task.ok_or_else(|| texturecraft_engine::EngineError::Other("bake cancelled".into()))?;
        task.run(&worker_progress)
    };

    // The preview pool keeps bake setup off the UI thread
    let receiver = match preview_pool() {
        Some(pool) => {
            let (sender, receiver) = std::sync::mpsc::channel();
            pool.spawn(move || {
                let _ = sender.send(run());
            });
            receiver
        }
        None => crate::spawn_worker("texturecraft-map-preview", run),
    };
    let started = ctx.input(|input| input.time);
    app.live_preview.running = Some(RunningPreview { target, size, started, dragging, progress, receiver, pending });
}

/// A dragged slider's first preview size before this machine's speed is known: 64 for the ray-traced bakers (well over 100 ms at 128 on a 4-core machine), else 128
fn first_drag_size(map: BakedMap) -> BakeResolution {
    match map {
        BakedMap::AmbientOcclusion | BakedMap::Thickness | BakedMap::BentNormal | BakedMap::Curvature => BakeResolution::R64,
        _ => BakeResolution::R128,
    }
}

/// The preview bakes' own thread pool, a thread short of the cores, so the UI's parallel work never waits behind a preview; None where no pool can start
#[cfg(not(target_arch = "wasm32"))]
fn preview_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(1, |cores| cores.get().saturating_sub(1).max(1));
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|index| format!("texturecraft-map-preview-{index}")).build();
        if let Err(error) = &pool {
            log::warn!("map previews share the global thread pool: {error}");
        }
        pool.ok()
    })
    .as_ref()
}

/// Web builds have no threads: the preview bakes on the UI thread
#[cfg(target_arch = "wasm32")]
fn preview_pool() -> Option<&'static rayon::ThreadPool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drag_size_steps_up_while_fast_down_while_slow_and_never_past_the_chosen_size() {
        let mut preview = LivePreview::default();
        let map = BakedMap::AmbientOcclusion;
        let largest = BakeResolution::R1024;
        let start = preview.drag_size(map, largest);

        preview.learn_drag_size(map, start, DRAG_STEP_UP_SECONDS / 2.0, largest);
        let bigger = preview.drag_size(map, largest);
        assert!(bigger.pixels() > start.pixels(), "a quick bake steps up");

        preview.learn_drag_size(map, bigger, DRAG_STEP_DOWN_SECONDS * 2.0, largest);
        assert_eq!(preview.drag_size(map, largest), start, "a slow bake steps back down");

        preview.learn_drag_size(map, start, DRAG_STEP_UP_SECONDS / 2.0, start);
        assert_eq!(preview.drag_size(map, largest), start, "never steps past the chosen size");

        preview.learn_drag_size(map, start, DRAG_STEP_UP_SECONDS / 2.0, largest);
        assert_eq!(preview.drag_size(map, start), start, "a smaller chosen size caps what was learned");
    }
}
