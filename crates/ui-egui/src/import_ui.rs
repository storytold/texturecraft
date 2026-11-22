//! The project import's two windows (File › Import project..., or a project file of the reference app opened or
//! dropped): a notice that must be acknowledged before anything runs, then the conversion's progress, which opens the
//! converted project when it finishes and turns into the error view when it fails.
//!
//! The conversion is the engine's `file.importProject` run as a task on a worker thread ([`ImportTask::run`]), like a
//! texture set resize; its result opens through [`Session::apply_import`](texturecraft_engine::Session::apply_import).
//! Called through the control channel or MCP, the command runs on its own and opens no window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use egui::RichText;
use texturecraft_engine::geom::OrbitCamera;
use texturecraft_engine::import_cmds::{ImportOutput, ImportPreview, ImportProgress, ImportTask, Step};
use texturecraft_render::{Frame, Shade, ViewKind};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::widgets::primary_button;

/// Where failed imports are reported.
pub const ISSUES_URL: &str = "https://github.com/storytold/texturecraft/issues";
pub(crate) const VIEW_PREVIEW: u64 = 3;

/// The import windows' state.
pub enum ImportUi {
    /// Window 1: the notice, with the import it starts.
    Notice { task: ImportTask },
    /// Window 2 while the conversion runs. `started` is app time (seconds).
    Running { file: String, path: Option<String>, progress: Arc<ImportProgress>, rx: Receiver<texturecraft_engine::Result<ImportOutput>>, started: f64 },
    /// Window 2's error view: the step that failed and the error.
    Failed { file: String, step: Step, error: String },
}

pub(crate) struct Presentation {
    origin_project: u64,
    preview: Option<PreviewView>,
}

struct PreviewView {
    source: ImportPreview,
    camera: OrbitCamera,
    lost: Arc<AtomicBool>,
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
        ImportUi::Running { .. } => running(app, ctx, state, None),
        ImportUi::Failed { file, step, error } => failed(ctx, file, step, error),
    };
}

/// Switches to a read-only viewport once the worker supplies saved textures. The current
/// project remains in its own session until the worker finishes the editable project.
pub(crate) fn show_preview(app: &mut TextureCraftApp, ui: &mut egui::Ui) -> bool {
    let Some(ImportUi::Running { progress, .. }) = app.import.as_ref() else { return false };
    let Some(presentation) = app.import_presentation.as_mut() else { return false };
    if presentation.preview.is_none()
        && let Some(source) = progress.take_preview()
    {
        let camera = source.camera;
        presentation.preview = Some(PreviewView { source, camera, lost: Arc::new(AtomicBool::new(true)) });
    }
    if presentation.preview.is_none() {
        return false;
    }
    if !app.ui.hide_ui {
        crate::titlebar::show(app, ui);
    }
    let Some(import) = app.import.take() else { return false };
    let ctx = ui.ctx().clone();
    app.import = running(app, &ctx, import, Some(ui));
    true
}

pub(crate) fn preview_active(app: &TextureCraftApp) -> bool {
    app.import_presentation.as_ref().is_some_and(|presentation| presentation.preview.is_some())
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
    let progress = ImportProgress::with_live_log(&file);
    #[cfg(not(target_arch = "wasm32"))]
    let progress = if app.gpu.is_some() {
        let repaint = ctx.clone();
        progress.with_preview(move || repaint.request_repaint())
    } else {
        progress
    };
    let progress = Arc::new(progress);
    let worker = progress.clone();
    let prefer_gpu = app.ui.prefs.gpu_compositing && app.gpu.as_ref().is_some_and(crate::gpu::GpuLink::supports_gpu_layers);
    let task = task.with_graph_accelerator(app.material_graph_accelerator());
    let rx = crate::spawn_worker("texturecraft-import", move || {
        task.for_interactive_open().run_with_view_where(&worker, |session| {
            (0..session.caches.len())
                .take_while(|_| !worker.is_cancelled())
                .filter(|&ts| {
                    if prefer_gpu && ts == session.ts() {
                        session.doc.graphs.warm_saved_uses(ts);
                    }
                    let (w, h) = session.caches[ts].size;
                    let plan = if prefer_gpu { crate::gpu::GpuLink::plan_import_set(session, ts) } else { Err("GPU compositing is disabled.") };
                    match plan {
                        Ok(()) => worker.log_detail(format!("Texture set {} at {w}×{h} uses GPU viewport", ts + 1)),
                        Err(reason) => worker.log_detail(format!("Texture set {} at {w}×{h} uses CPU view preparation ({reason})", ts + 1)),
                    }
                    plan.is_err()
                })
                .collect()
        })
    });
    let started = ctx.input(|i| i.time);
    app.import_presentation = Some(Presentation { origin_project: app.session.project_id, preview: None });
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
            "preview": preview_active(app),
            "log": progress.log_lines(),
        }),
        Some(ImportUi::Failed { file, step, error }) => json!({ "window": "failed", "file": file, "step": step.id(), "error": error }),
    }
}

fn running(app: &mut TextureCraftApp, ctx: &egui::Context, state: ImportUi, preview_ui: Option<&mut egui::Ui>) -> Option<ImportUi> {
    let ImportUi::Running { file, path, progress, rx, started } = state else { return Some(state) };
    let now = ctx.input(|i| i.time);
    if app.import_presentation.as_ref().is_some_and(|presentation| presentation.origin_project != app.session.project_id) {
        progress.cancel();
        finish_preview(app, ctx);
        return None;
    }
    // An outstanding close confirmation refers to the original project. Keep it in
    // place until the confirmation has been answered.
    let result = if app.ui.windows.unsaved.is_none() { rx.try_recv() } else { Err(TryRecvError::Empty) };
    match result {
        Err(TryRecvError::Empty) => {}
        Ok(_) if progress.is_cancelled() => {
            finish_preview(app, ctx);
            app.session.log.push(format!("Import {file}: cancelled after {}", clock(now - started)));
            return None;
        }
        Ok(Ok(out)) => {
            let camera = finish_preview(app, ctx);
            let handoff = std::time::Instant::now();
            return match app.session.install_import(out) {
                Ok(_) => {
                    app.clear_project_jobs();
                    app.ui.windows.export = false;
                    if let Some(camera) = camera {
                        app.session.camera = camera;
                    }
                    app.session.log.push(format!("Import {file}: finished in {}", clock(now - started)));
                    progress.log_detail(format!("Editable project installed in {} ms", handoff.elapsed().as_millis()));
                    app.ui.mode = crate::state::Mode::Painting;
                    None
                }
                Err(e) => Some(fail(app, file, path.as_deref(), Step::Saving, &e.to_string())),
            };
        }
        Ok(Err(e)) => {
            finish_preview(app, ctx);
            return Some(fail(app, file, path.as_deref(), progress.step(), &e.to_string()));
        }
        Err(TryRecvError::Disconnected) => {
            finish_preview(app, ctx);
            return Some(fail(app, file, path.as_deref(), progress.step(), "the import worker stopped"));
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let stopping = progress.is_cancelled();
    let mut cancel = false;
    let title = trf("Converting {name}", &[("name", &file)]);
    let show_overview = preview_ui.is_some();
    let mut show_progress = |ui: &mut egui::Ui| {
        let text = if stopping { tr("Stopping...").to_string() } else { step_label(progress.step()) };
        ui.add(egui::ProgressBar::new(progress.fraction()).desired_width(WIDTH).show_percentage());
        ui.label(text);
        ui.label(RichText::new(trf("Elapsed: {time}", &[("time", &clock(now - started))])).weak());
        ui.add_space(8.0);
        if show_overview {
            egui::CollapsingHeader::new(tr("Import log")).show(ui, |ui| show_log(ui, &progress));
        } else {
            ui.label(tr("Import log"));
            show_log(ui, &progress);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.add_enabled(!stopping, egui::Button::new(tr("Cancel"))).clicked();
            });
        });
    };
    if let Some(ui) = preview_ui {
        egui::Panel::top("import_preview_progress").show(ui, |ui| {
            ui.heading(&title);
            ui.label(tr("Stored material preview. Editing is available after conversion."));
            show_progress(ui);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(preview) = app.import_presentation.as_mut().and_then(|presentation| presentation.preview.as_mut()) {
                preview.show(ui, app.ui.prefs.background);
            }
        });
        cancel |= app.ui.windows.unsaved.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Escape));
    } else {
        modal(ctx, "import-progress", &title, show_progress);
    }
    if cancel {
        progress.cancel();
    }
    Some(ImportUi::Running { file, path, progress, rx, started })
}

fn show_log(ui: &mut egui::Ui, progress: &ImportProgress) {
    egui::ScrollArea::vertical().id_salt("import_live_log").max_height(144.0).auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
        for line in progress.log_lines() {
            ui.add(egui::Label::new(RichText::new(line).monospace().size(11.0)).wrap());
        }
    });
}

fn finish_preview(app: &mut TextureCraftApp, ctx: &egui::Context) -> Option<OrbitCamera> {
    let view = app.import_presentation.take()?.preview?;
    if let Some(render_state) = &app.render_state
        && let Some(gpu) = render_state.renderer.write().callback_resources.get_mut::<crate::gpu::GpuState>()
    {
        gpu.renderer.drop_view(VIEW_PREVIEW);
    }
    if let Some(gpu) = &app.gpu {
        // The saved preview occupied the renderer's scene without changing the editable session.
        gpu.lost.store(true, Ordering::Release);
    }
    ctx.request_repaint();
    (view.camera != view.source.camera).then_some(view.camera)
}

impl PreviewView {
    fn show(&mut self, ui: &mut egui::Ui, background: [[f32; 3]; 2]) {
        use egui::{Key, PointerButton};

        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tr("Stored material preview")));
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.dragged_by(PointerButton::Primary) {
            let delta = response.drag_delta() * 0.006;
            self.camera.orbit(delta.x, delta.y);
        }
        if response.dragged_by(PointerButton::Middle) {
            let delta = response.drag_delta() / rect.height().max(1.0);
            self.camera.pan(delta.x, delta.y);
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            self.camera.zoom(scroll * 0.002);
        }
        if response.has_focus() {
            ui.input(|input| {
                let key = |k| f32::from(u8::from(input.key_pressed(k)));
                self.camera.orbit(0.15 * (key(Key::ArrowRight) - key(Key::ArrowLeft)), 0.15 * (key(Key::ArrowUp) - key(Key::ArrowDown)));
                self.camera.zoom(0.15 * (key(Key::Plus) - key(Key::Minus)));
            });
        }

        let pixels = ui.ctx().pixels_per_point();
        let size = [(rect.width() * pixels).round().max(1.0) as u32, (rect.height() * pixels).round().max(1.0) as u32];
        let frame = Frame {
            view_id: VIEW_PREVIEW,
            size,
            kind: ViewKind::Scene,
            camera: self.camera,
            shade: Shade {
                background,
                normal_y: if self.source.normal_format == texturecraft_engine::core::NormalFormat::DirectX { -1.0 } else { 1.0 },
                spec_gloss: self.source.spec_gloss,
                ..Shade::default()
            },
            visible: vec![true; self.source.materials.len()],
            overlay: Vec::new(),
        };
        let upload = self.lost.swap(false, Ordering::AcqRel).then(|| crate::gpu::SceneSync::import_preview(&self.source));
        let callback = crate::gpu::ViewCallback { frame, sync: Mutex::new(upload), rect, lost: self.lost.clone() };
        ui.painter().add(eframe::egui_wgpu::Callback::new_paint_callback(rect, callback));
        if response.has_focus() {
            ui.painter().rect_stroke(rect.shrink(1.0), 0.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
        }
    }
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    #[test]
    fn an_import_preview_owns_input_over_the_previous_projects_export_window() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut previous = texturecraft_engine::sample_session(128, 64).unwrap();
        previous.modified = false;
        let original_camera = previous.camera;
        let project_id = previous.project_id;
        let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, previous));
        h.state_mut().ui.prefs.language = "en".into();
        h.state_mut().ui.windows.export = true;
        h.run_steps(5);
        assert!(h.query_by_label("Reset all").is_some());

        h.state_mut().file_dialogs.set_override(Some(Box::new(|_| crate::file_dialogs::Answer::Paths(vec!["second.spp".into()]))));
        let ctx = h.ctx.clone();
        for abort in [true, false] {
            assert!(crate::windows::file_action(h.state_mut(), &ctx, "file.importProject"));
            h.run_steps(4);
            assert!(matches!(h.state().import, Some(ImportUi::Notice { .. })));
            assert!(h.query_by_label("Reset all").is_none(), "the previous project's Export window stays behind the import");

            // Simulate the worker publishing the saved view and waiting for the editable result.
            let progress = Arc::new(ImportProgress::new());
            let (tx, rx) = std::sync::mpsc::channel();
            let started = ctx.input(|i| i.time);
            let app = h.state_mut();
            let source = ImportPreview {
                mesh: app.session.mesh.clone(),
                mesh_id: app.session.mesh_id,
                materials: app.session.doc.project.texture_sets.iter().map(|set| set.material).collect(),
                texture_set: 0,
                camera: original_camera,
                normal_format: Default::default(),
                spec_gloss: false,
                overview: texturecraft_engine::MaterialOverview { size: (64, 64), maps: Vec::new() },
            };
            app.import = Some(ImportUi::Running { file: "second.spp".into(), path: None, progress: progress.clone(), rx, started });
            app.import_presentation = Some(Presentation {
                origin_project: project_id,
                preview: Some(PreviewView { source, camera: original_camera, lost: Arc::new(AtomicBool::new(true)) }),
            });
            h.run_steps(4);

            let center = h.get_by_role_and_label(egui::Role::Image, "Stored material preview").rect().center();
            let end = center + egui::vec2(100.0, 20.0);
            h.hover_at(center);
            h.run_steps(2);
            h.event(Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(2);
            h.hover_at(end);
            h.run_steps(2);
            h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
            let orbit = h.state().import_presentation.as_ref().unwrap().preview.as_ref().unwrap().camera;
            assert_ne!(orbit.yaw, original_camera.yaw, "pointer drag orbits the preview after the native picker and Export window");
            assert_eq!(h.state().session.camera, original_camera, "the previous project's camera is intact");

            if abort {
                h.get_by_label("Cancel").click();
                h.run_steps(2);
                assert!(progress.is_cancelled());
            }
            let mut converted = texturecraft_engine::sample_session(128, 64).unwrap();
            converted.doc.project.name = "Second project".into();
            tx.send(Ok(ImportOutput::from_session(converted))).unwrap();
            h.run_steps(4);
            assert!(h.state().import.is_none());
            if abort {
                assert_eq!(h.state().session.project_id, project_id, "Cancel keeps the previous project");
                assert!(h.query_by_label("Reset all").is_some(), "Cancel returns to the previous project's Export window");
            } else {
                assert_eq!(h.state().session.project().name, "Second project");
                assert_eq!(h.state().session.camera, orbit, "completing the import keeps the angle chosen in the preview");
                assert!(h.query_by_label("Reset all").is_none(), "the previous project's Export window stays closed");
            }
        }
    }
}
