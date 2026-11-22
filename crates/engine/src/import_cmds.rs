//! `file.importProject`: a project file of the reference app becomes a TextureCraft project, which opens as the
//! session's project. The conversion is one-way: the original file is only read.
//!
//! The translator (`texturecraft-import`) builds the project, its layer pixels, baked maps and image resources; here the
//! mesh is stored as a binary glTF file in the project (the original's mesh format is not kept) and the result opens
//! through the same path as a saved project, so an imported project is exactly what its saved file will hold.
//! Brush strokes are replayed with TextureCraft's brush by [`crate::import_replay`], which the translator calls per texture
//! set. Programmatic calls never open dialogs; the two-window import (notice, progress) belongs to the UI.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use texturecraft_compose::graphs::{CookTrace, GraphAccelerator};
use texturecraft_formats::ProjectFile;
use texturecraft_import::import::StoredPreview;
use texturecraft_import::{Cancel, Hooks, ImportOptions, Imported};
/// The project file extension the import reads, and the import's steps.
pub use texturecraft_import::{PROJECT_EXT, Step};

use crate::commands::{CommandSpec, always, bad, boolp, strp};
use crate::{EngineError, Result, Session, cmd};

mod live_log;
use live_log::LiveLog;

const ID: &str = "file.importProject";
/// Largest project file accepted as `bytes` (the web build holds it in memory).
const MAX_BYTES: usize = 2 << 30;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "file.importProject",
        "Import project...",
        [],
        None,
        "{path?: the project file (native only), bytes?: the project file as base64 (any target), name?: file name for the report (with bytes), originalOnTop?: bool (show the original's stored result on top of stacks that are not reproduced; default false), originalCorrection?: bool (a locked correction layer on top of stacks that are not reproduced, holding the stored result on the texels the layers miss; default false), nativeFilters?: bool (replace a few filter graphs by close built-in filters; default false), maxResolution?: px (cap texture set resolutions), measure?: bool (compare each stack with the stored result; default true), replayStrokes?: bool (replay brush strokes with TextureCraft's brush, approximated; default true), graphs?: \"decompile\" | \"keep\" (run the project's compiled graphs decompiled, or keep them as locked placeholders; default decompile), resultCache?: folder (native only: keep graph renders there and reuse them in later imports; default none), resultCacheGb?: the folder's limit in GB (default 20), translateOnly?: bool (translate the layer stacks only: no stored result, strokes, graph cooks or safety net; the project holds no pixels, for a census of locks and placeholders; default false)} → the session, plus {import: {sets: [{name, strategy, layers, locked, placeholders, channels, scores}], resultCache?: {folder, found, missed, written, damaged, readBytes, writtenBytes}}}. One-way: the original file is only read",
        always,
        import
    )]
}

/// The import options from command params.
fn options(p: &Value) -> Result<ImportOptions> {
    let mut o = ImportOptions::default();
    if let Some(v) = boolp(p, "originalOnTop") {
        o.original_on_top = v;
    }
    if let Some(v) = boolp(p, "originalCorrection") {
        o.original_correction = v;
    }
    if let Some(v) = boolp(p, "nativeFilters") {
        o.native_filters = v;
    }
    if let Some(v) = boolp(p, "measure") {
        o.measure = v;
    }
    if let Some(v) = boolp(p, "replayStrokes") {
        o.replay_strokes = v;
    }
    if let Some(v) = boolp(p, "translateOnly") {
        o.translate_only = v;
    }
    match p.get("graphs").filter(|v| !v.is_null()).map(|v| v.as_str()) {
        None => {}
        Some(Some("decompile")) => o.graphs = texturecraft_import::GraphMode::Decompile,
        Some(Some("keep")) => o.graphs = texturecraft_import::GraphMode::Keep,
        Some(_) => return Err(bad(ID, "`graphs` must be \"decompile\" or \"keep\"")),
    }
    if let Some(r) = p.get("maxResolution").filter(|v| !v.is_null()) {
        let r = r.as_u64().and_then(|r| u32::try_from(r).ok()).filter(|r| r.is_power_of_two() && (128..=8192).contains(r));
        o.max_resolution = Some(r.ok_or_else(|| bad(ID, "`maxResolution` must be a power of two from 128 to 8192"))?);
    }
    Ok(o)
}

impl Session {
    /// A new session from an imported project: the mesh stored as binary glTF, then opened like a saved project.
    pub fn from_imported(imported: Imported) -> Result<Session> {
        let name = imported.project.name.clone();
        let glb =
            texturecraft_formats::glb_bytes(&imported.mesh, &name, &[]).map_err(|e| EngineError::Other(format!("cannot store the imported mesh: {e}")))?;
        let mut project = imported.project;
        project.mesh = "mesh.glb".into();
        project.mesh_file = "mesh.glb".into();
        let pf = ProjectFile {
            project,
            pixels: imported.pixels,
            baked: imported.baked,
            mesh: Some(("glb".into(), glb)),
            high_poly: Vec::new(),
            cage: None,
            bake_paint: Default::default(),
            thumbnail: None,
            resources: imported.resources,
            graphs: imported.graphs,
            strokes: imported.strokes,
            stroke_replay: None,
            warnings: Vec::new(),
        };
        let mut s = Session::from_project_file(pf)?;
        // open with the view the original was saved with
        if s.doc.project.cameras.iter().any(|c| c.name == texturecraft_import::cameras::SAVED_VIEW) {
            let _ = crate::execute(&mut s, "camera.select", &json!({"name": texturecraft_import::cameras::SAVED_VIEW}));
        }
        s.modified = true;
        Ok(s)
    }

    /// Imports the project file `bytes` (`name` names it) and opens it as the session's project.
    pub fn import_project_bytes(&mut self, bytes: Vec<u8>, name: &str, options: &ImportOptions) -> Result<Value> {
        let task = ImportTask {
            source: ImportSource::Bytes { bytes, name: name.to_string() },
            options: options.clone(),
            result_cache: None,
            view: false,
            accelerator: self.doc.graphs.accelerator(),
        };
        let out = task.run(&ImportProgress::new())?;
        self.apply_import(out)
    }

    /// Opens a finished import ([`ImportTask::run`]) as the session's project; the reply is the session with the
    /// import's summary. An import whose progress was cancelled is refused (its graph store renders nothing more).
    pub fn apply_import(&mut self, out: ImportOutput) -> Result<Value> {
        let summary = self.install_import(out)?;
        let mut reply = crate::inspect::session(self);
        if let Some(o) = reply.as_object_mut() {
            o.insert("import".into(), summary);
        }
        Ok(reply)
    }

    /// Open a converted project and return its import summary. Front ends drawing the view
    /// can use this without building a complete session inspection.
    pub fn install_import(&mut self, out: ImportOutput) -> Result<Value> {
        if out.cancel.is_cancelled() {
            return Err(bad(ID, CANCELLED));
        }
        self.switch_to(out.session);
        Ok(out.summary)
    }
}

/// The error text of a cancelled import.
pub const CANCELLED: &str = "import cancelled";

/// One import's progress (fraction, step) and its cancel flag, shared between the thread running it and whoever
/// watches (the UI's progress window, a test, a log).
pub struct ImportProgress {
    state: Mutex<(f32, Step)>,
    /// Set to stop the import: it stops between steps, texture sets and strokes, and a graph render in progress stops
    /// at its next node.
    pub cancel: Cancel,
    listener: Option<Box<dyn Fn(f32, Step) + Send + Sync>>,
    live_log: Option<Arc<LiveLog>>,
    preview: Option<PreviewProgress>,
}

struct PreviewProgress {
    pending: Mutex<Option<ImportPreview>>,
    wake: Box<dyn Fn() + Send + Sync>,
}

/// The original saved view in renderer upload formats, independent of the editable project.
pub struct ImportPreview {
    pub mesh: Arc<texturecraft_geom::Mesh>,
    pub mesh_id: u64,
    pub materials: Vec<usize>,
    pub texture_set: usize,
    pub camera: texturecraft_geom::OrbitCamera,
    pub normal_format: texturecraft_core::NormalFormat,
    pub spec_gloss: bool,
    pub overview: crate::MaterialOverview,
}

impl Default for ImportProgress {
    fn default() -> Self {
        ImportProgress::new()
    }
}

impl ImportProgress {
    pub fn new() -> ImportProgress {
        ImportProgress { state: Mutex::new((0.0, Step::ReadingFile)), cancel: Cancel::new(), listener: None, live_log: None, preview: None }
    }

    /// Progress that also calls `listener` on every report (fraction, step), on the thread running the import.
    pub fn with_listener(listener: impl Fn(f32, Step) + Send + Sync + 'static) -> ImportProgress {
        ImportProgress { listener: Some(Box::new(listener)), ..ImportProgress::new() }
    }

    /// Keep a live diagnostic history for this import, with timestamped entries in the desktop performance log.
    pub fn with_live_log(file: &str) -> ImportProgress {
        ImportProgress { live_log: Some(Arc::new(LiveLog::new(file))), ..ImportProgress::new() }
    }

    /// The desktop viewport can draw a stored material while the worker imports edit data.
    pub fn with_preview(mut self, wake: impl Fn() + Send + Sync + 'static) -> Self {
        self.preview = Some(PreviewProgress { pending: Mutex::default(), wake: Box::new(wake) });
        self
    }

    pub fn take_preview(&self) -> Option<ImportPreview> {
        self.preview.as_ref()?.pending.lock().ok()?.take()
    }

    fn publish_preview(&self, preview: StoredPreview) {
        let Some(progress) = self.preview.as_ref() else { return };
        let mut camera = texturecraft_geom::OrbitCamera::default();
        let (lo, hi) = preview.mesh.bounds();
        camera.frame(lo, hi);
        if let Some(saved) = &preview.camera {
            crate::camera_cmds::apply(&mut camera, saved);
        }
        let overview = crate::MaterialOverview::from_stored(&preview.composite);
        let spec_gloss = preview.composite.channels.contains_key(&texturecraft_core::Channel::Glossiness)
            && preview.composite.channels.contains_key(&texturecraft_core::Channel::Specular);
        if let Ok(mut out) = progress.pending.lock()
            && !self.is_cancelled()
        {
            *out = Some(ImportPreview {
                mesh: Arc::new(preview.mesh),
                mesh_id: crate::session::next_mesh_id(),
                materials: preview.materials,
                texture_set: preview.texture_set,
                camera,
                normal_format: preview.normal_format,
                spec_gloss,
                overview,
            });
            self.log_detail("Stored material preview prepared");
            drop(out);
            (progress.wake)();
        }
    }

    /// The latest diagnostic lines, in the order they appeared.
    pub fn log_lines(&self) -> Vec<String> {
        self.live_log.as_ref().map_or_else(Vec::new, |log| log.lines())
    }

    /// Publish an import diagnostic to the live history and local performance log.
    pub fn log_detail(&self, message: impl AsRef<str>) {
        if let Some(log) = &self.live_log {
            log.record(message.as_ref());
        }
    }

    fn cook_trace(&self) -> Option<CookTrace> {
        let log = Arc::clone(self.live_log.as_ref()?);
        #[cfg(not(target_arch = "wasm32"))]
        let cpu_report = {
            let log = Arc::clone(&log);
            let warn = std::sync::Once::new();
            Some(Arc::new(move |report| {
                if let Err(error) = crate::session_log::write_graph_cpu_report(&report) {
                    warn.call_once(|| log.record(&format!("Could not write graph CPU diagnostics: {error}")));
                }
            }) as Arc<dyn Fn(texturecraft_compose::graphs::GraphCpuReport) + Send + Sync>)
        };
        #[cfg(target_arch = "wasm32")]
        let cpu_report = None;
        Some(CookTrace { line: Arc::new(move |message| log.record(&message)), cpu_report })
    }

    /// The fraction done, 0..1 (never goes back).
    pub fn fraction(&self) -> f32 {
        self.lock().0
    }

    /// The step running now (or the step that failed, after a failure).
    pub fn step(&self) -> Step {
        self.lock().1
    }

    /// Asks the import to stop.
    pub fn cancel(&self) {
        let requested = self.is_cancelled();
        self.cancel.cancel();
        if !requested && let Some(log) = &self.live_log {
            let _ = log.push("Cancellation requested");
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Reports `step` at `fraction` (the import does; a front end's tests may, to show a step).
    pub fn report(&self, fraction: f32, step: Step) {
        let (fraction, changed_step) = {
            let mut state = self.lock();
            let f = if fraction.is_finite() { fraction.clamp(state.0, 1.0) } else { state.0 };
            let changed_step = state.1 != step;
            *state = (f, step);
            (f, changed_step)
        };
        if changed_step && self.live_log.is_some() {
            self.log_detail(format!("{step} ({:.0}%)", fraction * 100.0));
        }
        if let Some(l) = &self.listener {
            l(fraction, step);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (f32, Step)> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(bad(ID, CANCELLED)) } else { Ok(()) }
    }
}

/// Where an import reads the project file.
enum ImportSource {
    #[cfg(not(target_arch = "wasm32"))]
    Path(String),
    Bytes {
        bytes: Vec<u8>,
        name: String,
    },
}

/// One import, ready to run on a worker thread ([`ImportTask::run`]) and be opened by [`Session::apply_import`]; the
/// command runs the same steps in one go.
pub struct ImportTask {
    source: ImportSource,
    options: ImportOptions,
    result_cache: Option<crate::result_cache::ResultCache>,
    /// Whether the run also composites the converted project's view ([`ImportTask::with_view`]).
    view: bool,
    accelerator: Option<Arc<dyn GraphAccelerator>>,
}

/// A finished import: the converted project as a session, the summary the command replies with, and the flag it ran
/// under.
pub struct ImportOutput {
    session: Session,
    summary: Value,
    cancel: Cancel,
}

impl ImportOutput {
    /// The converted project.
    pub fn session(&self) -> &Session {
        &self.session
    }
}

impl ImportTask {
    /// The import `file.importProject` runs for `p` (its params checked).
    pub fn from_params(p: &Value) -> Result<ImportTask> {
        let options = options(p)?;
        if let Some(v) = p.get("bytes").filter(|v| !v.is_null()) {
            let bytes = bytes_param(v)?;
            if bytes.len() > MAX_BYTES {
                return Err(bad(ID, format!("the file is {} bytes; at most {MAX_BYTES} are read in memory", bytes.len())));
            }
            let name = strp(p, "name").unwrap_or("Imported project").to_string();
            return Ok(ImportTask {
                source: ImportSource::Bytes { bytes, name },
                options,
                result_cache: crate::result_cache::ResultCache::from_params(ID, p)?,
                view: false,
                accelerator: None,
            });
        }
        Ok(ImportTask { source: path_source(p)?, options, result_cache: crate::result_cache::ResultCache::from_params(ID, p)?, view: false, accelerator: None })
    }

    /// Imports the file at `path` with the default options.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn path(path: impl Into<String>) -> ImportTask {
        ImportTask { source: ImportSource::Path(path.into()), options: ImportOptions::default(), result_cache: None, view: false, accelerator: None }
    }

    /// Imports the project file `bytes` (named `name`) with the default options.
    pub fn bytes(name: impl Into<String>, bytes: Vec<u8>) -> ImportTask {
        ImportTask {
            source: ImportSource::Bytes { bytes, name: name.into() },
            options: ImportOptions::default(),
            result_cache: None,
            view: false,
            accelerator: None,
        }
    }

    /// Also composites the converted project's view before the run ends (step [`Step::ComputingView`]), so that the
    /// project opens with nothing left to composite: a front end showing progress keeps it until the project can be
    /// shown, and the first composite (on a large project it can take a minute) runs on the import's thread instead of
    /// the front end's. The pixels are the ones the front end would composite itself.
    pub fn with_view(mut self, view: bool) -> ImportTask {
        self.view = view;
        self
    }

    /// Skip the optional comparison with the stored reference for an interactive open.
    /// Reuse completed graph renders across native imports, then prepare the viewport.
    pub fn for_interactive_open(mut self) -> ImportTask {
        self.options.measure &= self.options.original_correction || self.options.original_on_top;
        #[cfg(not(target_arch = "wasm32"))]
        if self.result_cache.is_none() {
            self.result_cache = crate::result_cache::ResultCache::for_interactive_open();
        }
        self
    }

    /// Also keeps graph renders in `cache`'s folder and reuses those found there (native builds).
    pub fn with_result_cache(mut self, cache: Option<crate::result_cache::ResultCache>) -> ImportTask {
        self.result_cache = cache;
        self
    }

    pub fn with_graph_accelerator(mut self, accelerator: Option<Arc<dyn GraphAccelerator>>) -> ImportTask {
        self.accelerator = accelerator;
        self
    }

    /// The path the import reads (`None` for bytes).
    pub fn source_path(&self) -> Option<&str> {
        match &self.source {
            #[cfg(not(target_arch = "wasm32"))]
            ImportSource::Path(p) => Some(p),
            ImportSource::Bytes { .. } => None,
        }
    }

    /// The file name the import reads (without its folder).
    pub fn file_name(&self) -> String {
        match &self.source {
            #[cfg(not(target_arch = "wasm32"))]
            ImportSource::Path(p) => std::path::Path::new(p).file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned()),
            ImportSource::Bytes { name, .. } => name.clone(),
        }
    }

    /// Converts the file and builds the project's session, reporting to `progress` and stopping when it is cancelled.
    /// The steps: reading the file, per texture set converting, replaying strokes and cooking graphs, then saving
    /// (the converted project put together and stored as a TextureCraft project, in memory).
    pub fn run(self, progress: &ImportProgress) -> Result<ImportOutput> {
        let view = self.view.then_some(all_view_sets);
        self.run_with_view_policy(progress, view)
    }

    /// Prepare selected texture sets on the import worker before the application opens the project.
    /// The selector sees the converted session and returns the texture sets that need CPU compositing.
    /// The viewport can prepare the remaining sets with its chosen backend.
    pub fn run_with_view_where(self, progress: &ImportProgress, cpu_sets: impl FnOnce(&Session) -> BTreeSet<usize>) -> Result<ImportOutput> {
        self.run_with_view_policy(progress, Some(cpu_sets))
    }

    fn run_with_view_policy(self, progress: &ImportProgress, view: Option<impl FnOnce(&Session) -> BTreeSet<usize>>) -> Result<ImportOutput> {
        progress.log_detail("Reading the file");
        let diagnostic_path = progress.live_log.as_ref().and(self.source_path()).filter(|path| !path.is_empty()).map(str::to_owned);
        let ImportTask { source, options, result_cache, accelerator, .. } = self;
        let cache = result_cache.map(|c| c.open());
        #[cfg(not(target_arch = "wasm32"))]
        let kept = cache.as_ref().map(|c| c.kept.clone());
        #[cfg(target_arch = "wasm32")]
        let kept = None;
        let result = run_kept(progress, kept, view, accelerator, move |hooks| match source {
            #[cfg(not(target_arch = "wasm32"))]
            ImportSource::Path(path) => texturecraft_import::import_path(std::path::Path::new(&path), &options, hooks).map_err(|e| (e, Some(path))),
            ImportSource::Bytes { bytes, name } => {
                if bytes.len() > MAX_BYTES {
                    return Err((
                        texturecraft_import::ImportError::Invalid(format!("the file is {} bytes; at most {MAX_BYTES} are read in memory", bytes.len())),
                        None,
                    ));
                }
                texturecraft_import::import_bytes(bytes, &name, &options, hooks).map_err(|e| (e, None))
            }
        });
        if let Some(log) = &progress.live_log {
            match &result {
                Ok(_) => log.record("Import ready to open"),
                Err(_) if progress.is_cancelled() => log.record("Import cancelled"),
                Err(error) => {
                    let mut message = error.to_string();
                    if let Some(path) = diagnostic_path.as_deref() {
                        message = message.replace(path, log.file());
                    }
                    log.record(&format!("Import failed while {} ({message})", progress.step()));
                }
            }
        }
        let mut out = result?;
        if let (Some(c), Some(o)) = (&cache, out.summary.as_object_mut()) {
            o.insert("resultCache".into(), c.summary());
        }
        Ok(out)
    }
}

/// Share of the progress bar the translator takes; the rest is saving.
const CONVERTING: f32 = 0.9;
/// With the view composited: the translator's share, then saving up to [`VIEW`], then the view.
const CONVERTING_WITH_VIEW: f32 = 0.85;
const VIEW: f32 = 0.88;
/// How long one composite call of the view step may take before the bar moves (milliseconds).
const VIEW_SLICE_MS: f64 = 250.0;

/// [`ImportTask::run`] with the conversion given (`convert` runs the translator with the hooks; its error may name the
/// file it read).
#[cfg(test)]
fn run_with(
    progress: &ImportProgress,
    convert: impl FnOnce(&mut Hooks) -> std::result::Result<Imported, (texturecraft_import::ImportError, Option<String>)>,
) -> Result<ImportOutput> {
    run_kept(progress, None, None::<fn(&Session) -> BTreeSet<usize>>, None, convert)
}

/// Runs the conversion with the import's hooks (and graph cooks also kept in `kept`), then builds the session.
fn run_kept(
    progress: &ImportProgress,
    kept: Option<std::sync::Arc<dyn texturecraft_compose::graphs::KeptCooks>>,
    view: Option<impl FnOnce(&Session) -> BTreeSet<usize>>,
    accelerator: Option<Arc<dyn GraphAccelerator>>,
    convert: impl FnOnce(&mut Hooks) -> std::result::Result<Imported, (texturecraft_import::ImportError, Option<String>)>,
) -> Result<ImportOutput> {
    progress.check()?;
    let (converting, saved) = if view.is_some() { (CONVERTING_WITH_VIEW, VIEW) } else { (CONVERTING, 1.0) };
    let report = |f: f32, step: Step| progress.report(f * converting, step);
    let publish = |preview| progress.publish_preview(preview);
    let preview = progress.preview.as_ref().map(|_| &publish as &(dyn Fn(StoredPreview) + Sync));
    let replayer = crate::import_replay::CancellableReplayer::new(progress.cancel.clone());
    let mut hooks =
        Hooks { progress: &report, cancel: &progress.cancel, replay: Some(&replayer), kept_cooks: kept, trace: progress.cook_trace(), accelerator, preview };
    let imported = convert(&mut hooks).map_err(|(e, path)| match (e, path) {
        (texturecraft_import::ImportError::Cancelled, _) => bad(ID, CANCELLED),
        (e, Some(path)) => bad(ID, format!("{path}: {e}")),
        (e, None) => bad(ID, e.to_string()),
    })?;
    progress.check()?;
    progress.report(converting, Step::Saving);
    if progress.live_log.is_some() {
        progress.log_detail(format!(
            "Converted {} texture sets ({} graph renders, {} disk cache hits)",
            imported.project.texture_sets.len(),
            imported.graphs.renders(),
            imported.graphs.kept_found()
        ));
    }
    let summary = summary(&imported);
    let session = Session::from_imported(imported)?;
    progress.check()?;
    progress.report(saved, Step::Saving);
    let mut out = ImportOutput { session, summary, cancel: progress.cancel.clone() };
    if let Some(select) = view {
        progress.report(saved, Step::ComputingView);
        progress.check()?;
        progress.log_detail(format!("Selecting viewport backends for {} texture sets", out.session.caches.len()));
        let sets = select(&out.session);
        compute_view(&mut out.session, progress, saved, &sets)?;
    }
    if progress.live_log.is_some() {
        out.session.doc.graphs.trace_cooks(None);
    }
    Ok(out)
}

impl ImportOutput {
    /// A finished import of a project converted already (`session`), with an empty summary: for front ends' tests.
    pub fn from_session(session: Session) -> ImportOutput {
        ImportOutput { session, summary: json!({}), cancel: Cancel::new() }
    }

    /// Composites the converted project's view until no tile is left (step [`Step::ComputingView`], from where the bar
    /// is to its end; [`ImportTask::with_view`] runs it). It is the session's own budgeted update, in slices so that the
    /// bar moves between them and a cancel stops after the slice in progress (the import then fails as cancelled). The
    /// first slice computes what a whole set reads at once (graph filters over the stack), which can take a minute on a
    /// large project. The pixels are those of [`Session::update`].
    pub fn compute_view(&mut self, progress: &ImportProgress) -> Result<()> {
        let sets = all_view_sets(&self.session);
        let from = progress.fraction();
        progress.report(from, Step::ComputingView);
        compute_view(&mut self.session, progress, from, &sets)
    }
}

fn all_view_sets(session: &Session) -> BTreeSet<usize> {
    (0..session.caches.len()).collect()
}

fn compute_view(session: &mut Session, progress: &ImportProgress, from: f32, sets: &BTreeSet<usize>) -> Result<()> {
    let pending_tiles =
        |session: &Session| sets.iter().filter_map(|&ts| session.caches.get(ts)).map(|cache| cache.dirty.len() + cache.settle.len()).sum::<usize>();
    let initial_tiles = pending_tiles(session);
    let mut pending = initial_tiles;
    let total = initial_tiles.max(1) as f32;
    let renders = session.doc.graphs.renders();
    if pending > 0 {
        progress.log_detail(format!("Preparing the CPU view for {} texture sets ({pending} tiles)", sets.len()));
    }
    let start = clock::Clock::start();
    while pending > 0 {
        progress.check()?;
        session.update_for_view_where(VIEW_SLICE_MS, || start.ms(), crate::TileDemand::default(), |ts| sets.contains(&ts));
        pending = pending_tiles(session);
        let done = 1.0 - pending as f32 / total;
        progress.report(from + (1.0 - from) * done, Step::ComputingView);
    }
    progress.check()?;
    progress.report(1.0, Step::ComputingView);
    if initial_tiles > 0 {
        progress.log_detail(format!("CPU view ready ({} graph renders)", session.doc.graphs.renders().saturating_sub(renders)));
    }
    Ok(())
}

/// Milliseconds since a start, for the view step's slices (the web build has no clock here: every slice runs one chunk).
mod clock {
    pub struct Clock(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

    impl Clock {
        pub fn start() -> Clock {
            #[cfg(not(target_arch = "wasm32"))]
            return Clock(std::time::Instant::now());
            #[cfg(target_arch = "wasm32")]
            return Clock();
        }

        pub fn ms(&self) -> f64 {
            #[cfg(not(target_arch = "wasm32"))]
            return self.0.elapsed().as_secs_f64() * 1000.0;
            #[cfg(target_arch = "wasm32")]
            return f64::INFINITY;
        }
    }
}

/// What the safety net found per texture set.
fn summary(imported: &Imported) -> Value {
    let sets: Vec<Value> = imported
        .sets
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "strategy": s.strategy.id(),
                "layers": s.layers,
                "locked": s.locked,
                "placeholders": s.placeholders,
                "channels": s.channels.iter().map(|(c, st)| json!({"channel": c.id(), "status": st})).collect::<Vec<_>>(),
                "scores": s.scores.iter().map(|(c, sc)| json!({"channel": c.id(), "mean": sc.mean, "max": sc.max, "texels": sc.texels})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "sets": sets })
}

/// Base64 (standard alphabet, padding optional) or a JSON array of byte values.
pub(crate) fn bytes_param(v: &Value) -> Result<Vec<u8>> {
    match v {
        Value::String(s) => decode_base64(s).ok_or_else(|| bad(ID, "`bytes` is not valid base64")),
        Value::Array(a) => {
            if a.len() > MAX_BYTES {
                return Err(bad(ID, "`bytes` is too long"));
            }
            a.iter()
                .map(|x| x.as_u64().and_then(|b| u8::try_from(b).ok()))
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(|| bad(ID, "`bytes` must hold values 0 to 255"))
        }
        _ => Err(bad(ID, "`bytes` must be a base64 string or an array of byte values")),
    }
}

/// Standard base64; `None` on any character outside the alphabet.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let clean = clean.strip_suffix(b"==").or_else(|| clean.strip_suffix(b"=")).unwrap_or(&clean);
    if clean.len() / 4 * 3 > MAX_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3 + 3);
    for chunk in clean.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= value(*c)? << (18 - 6 * i as u32);
        }
        let bytes = acc.to_be_bytes();
        match chunk.len() {
            4 => out.extend_from_slice(&bytes[1..4]),
            3 => out.extend_from_slice(&bytes[1..3]),
            2 => out.push(bytes[1]),
            _ => return None,
        }
    }
    Some(out)
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let out = ImportTask::from_params(p)?.with_graph_accelerator(s.doc.graphs.accelerator()).run(&ImportProgress::new())?;
    s.apply_import(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn path_source(p: &Value) -> Result<ImportSource> {
    let path = strp(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(ID, "give `path` (a project file) or `bytes`"))?;
    Ok(ImportSource::Path(path.to_string()))
}

#[cfg(target_arch = "wasm32")]
fn path_source(_: &Value) -> Result<ImportSource> {
    Err(bad(ID, "`path` is not available in the browser: give `bytes`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_byte_arrays_decode_and_garbage_is_refused() {
        assert_eq!(decode_base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGk="), Some(b"hi".to_vec()));
        assert_eq!(decode_base64("a"), None);
        assert_eq!(decode_base64("a*bc"), None);
        assert_eq!(bytes_param(&json!([1, 2, 255])).unwrap(), vec![1, 2, 255]);
        assert!(bytes_param(&json!([256])).is_err());
        assert!(bytes_param(&json!(3)).is_err());
    }

    use std::sync::Arc;
    use texturecraft_import::ImportError;

    type Converted = std::result::Result<Imported, (ImportError, Option<String>)>;

    /// A small converted project, as the translator hands it over: one sphere set, its graph store under the import's
    /// flag.
    fn synthetic(hooks: &Hooks) -> Imported {
        let s = Session::new_project("synthetic", texturecraft_geom::primitives::builtin("sphere").unwrap(), "builtin:sphere", None, 128).unwrap();
        Imported {
            project: s.doc.project.clone(),
            pixels: s.doc.pixels.clone(),
            baked: s.doc.baked.clone(),
            mesh: (*s.mesh).clone(),
            resources: Default::default(),
            graphs: texturecraft_compose::GraphStore::with_cancel(hooks.cancel.clone()),
            sets: Vec::new(),
            strokes: Default::default(),
        }
    }

    /// The steps the translator reports for `sets` texture sets, checking the flag between them as it does.
    fn translate(hooks: &mut Hooks, sets: usize, stop_at: Option<(usize, &ImportProgress)>) -> Converted {
        let check = |h: &Hooks| if h.cancel.is_cancelled() { Err((ImportError::Cancelled, None)) } else { Ok(()) };
        (hooks.progress)(0.0, Step::ReadingFile);
        for i in 0..sets {
            check(hooks)?;
            let (set, of) = (i + 1, sets);
            let start = 0.1 + 0.8 * i as f32 / sets as f32;
            (hooks.progress)(start, Step::ConvertingSet { set, of });
            if let Some((at, progress)) = stop_at
                && at == set
            {
                progress.cancel();
            }
            check(hooks)?;
            (hooks.progress)(start + 0.1, Step::ReplayingStrokes { set, of });
            (hooks.progress)(start + 0.2, Step::CookingGraphs { set, of });
        }
        check(hooks)?;
        (hooks.progress)(0.95, Step::Saving);
        Ok(synthetic(hooks))
    }

    fn sphere_session() -> Session {
        Session::new_project("t", texturecraft_geom::primitives::builtin("sphere").unwrap(), "builtin:sphere", None, 128).unwrap()
    }

    #[test]
    fn progress_reports_every_step_in_order_and_the_project_opens() {
        let seen = Arc::new(Mutex::new(Vec::<(f32, Step)>::new()));
        let log = seen.clone();
        let progress = ImportProgress::with_listener(move |f, step| log.lock().unwrap().push((f, step)));
        let out = run_with(&progress, |hooks| translate(hooks, 2, None)).unwrap();
        let seen = seen.lock().unwrap().clone();
        let mut ids: Vec<&str> = seen.iter().map(|(_, s)| s.id()).collect();
        ids.dedup();
        assert_eq!(ids, ["reading", "converting", "replaying", "cooking", "converting", "replaying", "cooking", "saving"]);
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "the fraction never goes back: {seen:?}");
        assert!(seen.iter().filter(|(_, s)| *s != Step::Saving).all(|(f, _)| *f <= CONVERTING), "the translator's share ends where saving starts");
        assert_eq!(progress.fraction(), 1.0);
        assert_eq!(progress.step(), Step::Saving);
        assert_eq!(seen.iter().filter_map(|(_, s)| s.set()).max(), Some((2, 2)));
        assert_eq!(Step::ConvertingSet { set: 2, of: 5 }.to_string(), "converting texture set 2 of 5");
        let mut s = sphere_session();
        let reply = s.apply_import(out).unwrap();
        assert_eq!(s.project().name, "synthetic");
        assert!(reply.get("import").is_some(), "the reply carries the import summary");
    }

    /// With the view asked for, the run ends by compositing the converted project: the bar keeps going up through
    /// the view step and the project opens with no tile left, its composite the one an update makes.
    #[test]
    fn the_view_step_composites_the_project_before_the_run_ends() {
        let seen = Arc::new(Mutex::new(Vec::<(f32, Step)>::new()));
        let log = seen.clone();
        let progress = ImportProgress::with_listener(move |f, step| log.lock().unwrap().push((f, step)));
        let out = run_kept(&progress, None, Some(all_view_sets), None, |hooks| translate(hooks, 1, None)).unwrap();
        let seen = seen.lock().unwrap().clone();
        let mut ids: Vec<&str> = seen.iter().map(|(_, s)| s.id()).collect();
        ids.dedup();
        assert_eq!(ids, ["reading", "converting", "replaying", "cooking", "saving", "view"]);
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "the fraction never goes back: {seen:?}");
        assert!(seen.iter().filter(|(_, s)| *s != Step::ComputingView).all(|(f, _)| *f <= VIEW), "the view has its own share: {seen:?}");
        assert_eq!(seen.last(), Some(&(1.0, Step::ComputingView)));
        assert_eq!(out.session().pending_tiles(), 0, "nothing left to composite");
        // the same pixels an update of the converted project gives
        let mut plain = run_with(&ImportProgress::new(), |hooks| translate(hooks, 1, None)).unwrap();
        assert!(plain.session.pending_tiles() > 0);
        plain.session.update();
        for (a, b) in out.session().caches.iter().zip(&plain.session.caches) {
            assert_eq!(a.composite, b.composite);
        }
        // Texture sets the viewport can prepare stay pending for its compositor.
        let progress = ImportProgress::new();
        let deferred = run_kept(&progress, None, Some(|_: &Session| BTreeSet::new()), None, |hooks| translate(hooks, 1, None)).unwrap();
        assert!(deferred.session().pending_tiles() > 0);
        assert_eq!((progress.fraction(), progress.step()), (1.0, Step::ComputingView));
        // a cancel during the view stops the run
        let progress = ImportProgress::with_listener(|_, _| {});
        let mut out = run_with(&progress, |hooks| translate(hooks, 1, None)).unwrap();
        progress.cancel();
        assert!(out.compute_view(&progress).unwrap_err().to_string().contains(CANCELLED));
    }

    #[test]
    fn a_cancel_stops_between_steps_and_nothing_opens() {
        // before anything runs
        let progress = ImportProgress::new();
        progress.cancel();
        let mut ran = false;
        let e = run_with(&progress, |hooks| {
            ran = true;
            translate(hooks, 1, None)
        });
        assert!(!ran && e.is_err_and(|e| e.to_string().contains(CANCELLED)));
        // during the second texture set: the third is never started
        let seen = Arc::new(Mutex::new(Vec::<Step>::new()));
        let log = seen.clone();
        let progress = ImportProgress::with_listener(move |_, step| log.lock().unwrap().push(step));
        let e = run_with(&progress, |hooks| translate(hooks, 3, Some((2, &progress))));
        assert!(e.is_err_and(|e| e.to_string().contains(CANCELLED)));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.last(), Some(&Step::ConvertingSet { set: 2, of: 3 }), "{seen:?}");
        assert_eq!(progress.step(), Step::ConvertingSet { set: 2, of: 3 }, "the step it stopped in stays readable");
        // after the translator, before the project is put together
        let progress = ImportProgress::new();
        let e = run_with(&progress, |hooks| {
            let r = translate(hooks, 1, None);
            progress.cancel();
            r
        });
        assert!(e.is_err_and(|e| e.to_string().contains(CANCELLED)));
    }

    #[test]
    fn a_finished_import_cancelled_before_it_opens_is_refused() {
        let progress = ImportProgress::new();
        let out = run_with(&progress, |hooks| translate(hooks, 1, None)).unwrap();
        progress.cancel();
        let mut s = sphere_session();
        assert!(s.apply_import(out).is_err());
        assert_eq!(s.project().name, "t", "the session keeps its project");
    }

    #[test]
    fn translator_errors_name_the_file_and_keep_their_step() {
        let progress = ImportProgress::new();
        let e = run_with(&progress, |hooks| {
            (hooks.progress)(0.3, Step::ConvertingSet { set: 1, of: 1 });
            Err((ImportError::Invalid("no texture set".into()), Some("/x/y.file".into())))
        })
        .err()
        .unwrap()
        .to_string();
        assert!(e.contains("/x/y.file: no texture set"), "{e}");
        assert_eq!(progress.step(), Step::ConvertingSet { set: 1, of: 1 });
        assert_eq!(ImportTask::path("/x/y.file").file_name(), "y.file");
        assert_eq!(ImportTask::bytes("z.file", vec![1]).file_name(), "z.file");
        // the translator's real errors on hostile bytes arrive the same way
        let e = ImportTask::bytes("z.file", vec![0, 1, 2]).run(&ImportProgress::new());
        assert!(e.is_err());
    }

    /// A graph render in progress stops at its next node when the import is cancelled: the import's graph store
    /// renders under the import's flag.
    #[test]
    fn a_cancel_stops_a_graph_render_in_progress() {
        let mut text = String::from("format 3\nsize 2048\nnode 1 perlin scale=16\n");
        for i in 2..=40 {
            text.push_str(&format!("node {i} blur in=@{} radius=48\n", i - 1));
        }
        text.push_str("output out 40\n");
        let progress = ImportProgress::new();
        let started = std::sync::OnceLock::new();
        let stopped_after = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                run_with(&progress, |hooks| {
                    let mut store = texturecraft_compose::GraphStore::with_cancel(hooks.cancel.clone());
                    let key = store.add_text(&text);
                    let (baked, images) = (texturecraft_compose::BakedMaps::default(), texturecraft_compose::ImageStore::default());
                    let env = texturecraft_compose::graphs::RenderEnv { baked: &baked, images: &images, side: 2048 };
                    let _ = started.set(());
                    (hooks.progress)(0.5, Step::CookingGraphs { set: 1, of: 1 });
                    let cooked = store.cook(&texturecraft_core::GraphRef::new(key, "slow"), &env, &[]);
                    assert!(cooked.is_err(), "the render was stopped");
                    Err((ImportError::Cancelled, None))
                })
            });
            while started.get().is_none() {
                std::thread::yield_now();
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            let t = std::time::Instant::now();
            progress.cancel();
            let r = worker.join().unwrap();
            assert!(r.is_err_and(|e| e.to_string().contains(CANCELLED)));
            t.elapsed()
        });
        eprintln!("graph render stopped {:.0} ms after the cancel", stopped_after.as_secs_f64() * 1000.0);
        assert!(stopped_after < std::time::Duration::from_secs(5), "{stopped_after:?}");
    }

    #[test]
    fn garbage_files_are_errors_not_panics() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::builtin("sphere").unwrap(), "builtin:sphere", None, 128).unwrap();
        for p in [json!({}), json!({"bytes": "AAAA"}), json!({"bytes": [0, 1, 2]}), json!({"path": "/nonexistent/file"}), json!({"maxResolution": 100})] {
            assert!(import(&mut s, &p).is_err(), "{p}");
        }
        assert_eq!(s.project().name, "t", "a failed import leaves the session as it was");
    }
}
