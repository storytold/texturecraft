//! The project import's two windows (File › Import project..., or a project file of the reference app opened or
//! dropped): a notice that must be acknowledged before anything runs, then the conversion's progress, which opens the
//! converted project when it finishes and turns into the error view when it fails.
//!
//! The conversion is the engine's `file.importProject` run as a task on a worker thread ([`ImportTask::run`]), like a
//! texture set resize; its result opens through [`Session::apply_import`](texturecraft_engine::Session::apply_import).
//! Called through the control channel or MCP, the command runs on its own and opens no window.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::RichText;
use texturecraft_engine::import_cmds::{ImportOutput, ImportProgress, ImportTask, Step};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::widgets::primary_button;

/// Where failed imports are reported.
pub const ISSUES_URL: &str = "https://github.com/storytold/texturecraft/issues";

/// The import windows' state.
pub enum ImportUi {
    /// Window 1: the notice, with the import it starts.
    Notice { task: ImportTask },
    /// Window 2 while the conversion runs. `started` is app time (seconds).
    Running { file: String, path: Option<String>, progress: Arc<ImportProgress>, rx: Receiver<texturecraft_engine::Result<ImportOutput>>, started: f64 },
    /// Window 2's error view: the step that failed and the error.
    Failed { file: String, step: Step, error: String },
}

/// Opens the notice for `task` (nothing runs until it is acknowledged). An import already showing keeps its window.
pub fn ask(app: &mut TextureCraftApp, task: ImportTask) {
    if app.import.is_none() {
        app.import = Some(ImportUi::Notice { task });
    }
}

/// Whether an import window is showing.
pub fn active(app: &TextureCraftApp) -> bool {
    app.import.is_some()
}

/// The step as the progress window names it.
pub fn step_label(step: Step) -> String {
    let set = |text: &str, n: usize, m: usize| trf(text, &[("n", &n), ("m", &m)]);
    match step {
        Step::ReadingFile => tr("Reading the file").to_string(),
        Step::ConvertingSet { set: n, of } => set(tr("Converting texture set {n} of {m}"), n, of),
        Step::ReplayingStrokes { set: n, of } => set(tr("Converting texture set {n} of {m}: replaying strokes"), n, of),
        Step::CookingGraphs { set: n, of } => set(tr("Converting texture set {n} of {m}: cooking graphs"), n, of),
        Step::Saving => tr("Saving").to_string(),
        Step::ComputingView => tr("Computing the view").to_string(),
    }
}

/// `m:ss` (or `h:mm:ss`).
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", (s / 60) % 60, s % 60),
    }
}

/// What "Copy details" copies: the version, the step and the error, nothing of the file.
pub fn details(step: Step, error: &str) -> String {
    format!("TextureCraft {}\nStep: {step}\nError: {error}", texturecraft_engine::VERSION)
}

/// Draws the import window and moves the import along (called every frame).
pub fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(state) = app.import.take() else { return };
    app.import = match state {
        ImportUi::Notice { task } => notice(app, ctx, task),
        ImportUi::Running { file, path, progress, rx, started } => running(app, ctx, file, path, progress, rx, started),
        ImportUi::Failed { file, step, error } => failed(ctx, file, step, error),
    };
}

/// A centered modal window titled `title`: the project behind it takes no input while it shows (an edit made during
/// the conversion would be lost when the converted project opens). Only its buttons close it.
fn modal(ctx: &egui::Context, id: &str, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.label(RichText::new(title).font(crate::theme::semibold(14.0)).strong());
        ui.add_space(8.0);
        add(ui);
    });
}

const WIDTH: f32 = 440.0;

const POINTS: [&str; 4] = [
    crate::i18n::n_("This is an early, one-way conversion into a TextureCraft project. The original file is never changed."),
    crate::i18n::n_("The result will not match the original project 100%; expect differences and issues."),
    crate::i18n::n_("Conversion takes a fairly long time and a fairly large amount of memory."),
    crate::i18n::n_("All of this is expected to improve in future versions."),
];

fn notice(app: &mut TextureCraftApp, ctx: &egui::Context, task: ImportTask) -> Option<ImportUi> {
    let (mut go, mut cancel) = (false, false);
    let title = trf("Import {name}", &[("name", &task.file_name())]);
    modal(ctx, "import-notice", &title, |ui| {
        for point in POINTS {
            ui.label(format!("• {}", tr(point)));
            ui.add_space(4.0);
        }
        ui.label(format!(
            "• {}",
            tr("The import may fail on something not seen before. If it does, please open an issue, with the details the progress window gives:")
        ));
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.hyperlink(ISSUES_URL);
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                go = ui.add(primary_button(ui, tr("I understand, convert"))).clicked();
                cancel = ui.button(tr("Cancel")).clicked();
            });
        });
    });
    if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return None;
    }
    if !go {
        return Some(ImportUi::Notice { task });
    }
    Some(start(app, ctx, task))
}

/// Convert and prepare CPU-only materials on the worker. The viewport schedules GPU materials as it opens.
fn start(app: &mut TextureCraftApp, ctx: &egui::Context, task: ImportTask) -> ImportUi {
    let (file, path) = (task.file_name(), task.source_path().map(str::to_string));
    let progress = Arc::new(ImportProgress::with_live_log(&file));
    let worker = progress.clone();
    let prefer_gpu = app.ui.prefs.gpu_compositing && app.gpu.as_ref().is_some_and(crate::gpu::GpuLink::supports_gpu_layers);
    let rx = crate::spawn_worker("texturecraft-import", move || {
        task.for_interactive_open().run_with_view_where(&worker, |session| {
            (0..session.caches.len())
                .take_while(|_| !worker.is_cancelled())
                .filter(|&ts| {
                    let cpu = !prefer_gpu || !crate::gpu::GpuLink::plans_import_set(session, ts);
                    let (w, h) = session.caches[ts].size;
                    let backend = if cpu { "CPU view preparation" } else { "GPU viewport" };
                    worker.log_detail(format!("Texture set {} at {w}×{h} uses {backend}", ts + 1));
                    cpu
                })
                .collect()
        })
    });
    let started = ctx.input(|i| i.time);
    app.session.log.push(format!("Import {file}: started"));
    ImportUi::Running { file, path, progress, rx, started }
}

/// "I understand, convert" on the notice showing (the control channel's `ui.import.convert`, as a click): whether
/// a notice was showing.
pub fn acknowledge(app: &mut TextureCraftApp, ctx: &egui::Context) -> bool {
    match app.import.take() {
        Some(ImportUi::Notice { task }) => {
            app.import = Some(start(app, ctx, task));
            true
        }
        other => {
            app.import = other;
            false
        }
    }
}

/// What the import windows show, for the control channel (`ui.import.status`): the window (`none`, `notice`,
/// `running`, `failed`), and while running the step, its id, the fraction and the seconds elapsed; when failed the
/// step and the error.
pub fn status(app: &TextureCraftApp, now: f64) -> serde_json::Value {
    use serde_json::json;
    match &app.import {
        None => json!({ "window": "none" }),
        Some(ImportUi::Notice { task }) => json!({ "window": "notice", "file": task.file_name() }),
        Some(ImportUi::Running { file, progress, started, .. }) => json!({
            "window": "running",
            "file": file,
            "step": step_label(progress.step()),
            "stepId": progress.step().id(),
            "fraction": progress.fraction(),
            "elapsed": now - started,
            "cancelled": progress.is_cancelled(),
            "log": progress.log_lines(),
        }),
        Some(ImportUi::Failed { file, step, error }) => json!({ "window": "failed", "file": file, "step": step.id(), "error": error }),
    }
}

fn running(
    app: &mut TextureCraftApp,
    ctx: &egui::Context,
    file: String,
    path: Option<String>,
    progress: Arc<ImportProgress>,
    rx: Receiver<texturecraft_engine::Result<ImportOutput>>,
    started: f64,
) -> Option<ImportUi> {
    let now = ctx.input(|i| i.time);
    match rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(_) if progress.is_cancelled() => {
            app.session.log.push(format!("Import {file}: cancelled after {}", clock(now - started)));
            return None;
        }
        Ok(Ok(out)) => {
            return match app.session.apply_import(out) {
                Ok(_) => {
                    app.clear_project_jobs();
                    app.session.log.push(format!("Import {file}: finished in {}", clock(now - started)));
                    app.ui.mode = crate::state::Mode::Painting;
                    None
                }
                Err(e) => Some(fail(app, file, path.as_deref(), Step::Saving, &e.to_string())),
            };
        }
        Ok(Err(e)) => return Some(fail(app, file, path.as_deref(), progress.step(), &e.to_string())),
        Err(TryRecvError::Disconnected) => return Some(fail(app, file, path.as_deref(), progress.step(), "the import worker stopped")),
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let stopping = progress.is_cancelled();
    let mut cancel = false;
    modal(ctx, "import-progress", &trf("Converting {name}", &[("name", &file)]), |ui| {
        let text = if stopping { tr("Stopping...").to_string() } else { step_label(progress.step()) };
        ui.add(egui::ProgressBar::new(progress.fraction()).desired_width(WIDTH).show_percentage());
        ui.label(text);
        ui.label(RichText::new(trf("Elapsed: {time}", &[("time", &clock(now - started))])).weak());
        ui.add_space(8.0);
        ui.label(tr("Import log"));
        egui::ScrollArea::vertical().id_salt("import_live_log").max_height(144.0).auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
            for line in progress.log_lines() {
                ui.add(egui::Label::new(RichText::new(line).monospace().size(11.0)).wrap());
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.add_enabled(!stopping, egui::Button::new(tr("Cancel"))).clicked();
            });
        });
    });
    if cancel {
        progress.cancel();
    }
    Some(ImportUi::Running { file, path, progress, rx, started })
}

/// The error view: the error with the file's folder left out (the details name the file only).
fn fail(app: &mut TextureCraftApp, file: String, path: Option<&str>, step: Step, error: &str) -> ImportUi {
    let error = match path {
        Some(p) if !p.is_empty() => error.replace(p, &file),
        _ => error.to_string(),
    };
    app.session.log.push(format!("Import {file}: failed while {step}: {error}"));
    ImportUi::Failed { file, step, error }
}

fn failed(ctx: &egui::Context, file: String, step: Step, error: String) -> Option<ImportUi> {
    let mut close = false;
    modal(ctx, "import-progress", &trf("Converting {name}", &[("name", &file)]), |ui| {
        ui.label(tr("The conversion failed; please open an issue and paste the details into it."));
        ui.add_space(6.0);
        ui.hyperlink(ISSUES_URL);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button(tr("Close")).clicked();
                if ui.button(tr("Copy details")).clicked() {
                    ui.ctx().copy_text(details(step, &error));
                }
            });
        });
    });
    (!close).then_some(ImportUi::Failed { file, step, error })
}
