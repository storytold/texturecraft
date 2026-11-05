//! Turntable export in the app (Display panel › Turntable, and the `viewport.turntable` control
//! method): spin the camera (or only the environment) 360 degrees over N frames and save an
//! animated GIF and/or a numbered PNG sequence, with progress and Cancel. Desktop only.
//!
//! Two ways to draw the frames:
//! - **Path traced** (`samples > 0`): a worker thread runs the CPU path tracer on a copy of the
//!   scene, so the UI stays live and a transparent background is possible.
//! - **Viewport** (`samples == 0`): the live 3D view is turned one frame at a time and read back
//!   when it has settled (the frame size is the on-screen 3D view). The camera and environment
//!   rotation are restored at the end, also on Cancel.
//!
//! The pure parts (angles, GIF encoding, file names) are `texturecraft_formats::turntable`, shared
//! with the headless `texturecraft-cli turntable`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

use serde::Deserialize;
use serde_json::{Value, json};
use texturecraft_engine::formats::turntable::{Background, GifWriter, Spin, Turntable, frame_name};
use texturecraft_render::trace::Tracer;

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::state::ViewLayout;

/// Largest frame edge of a turntable made from the app.
const MAX_SIDE: u32 = 4096;

/// What to render and where to write it (also the params of `viewport.turntable`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub frames: u32,
    pub fps: u32,
    /// Pixels for path-traced frames; 0 is the size of the 3D view. Viewport frames always have
    /// the on-screen size.
    pub width: u32,
    pub height: u32,
    /// `camera` or `environment`.
    pub spin: String,
    pub rotate_env: bool,
    /// `viewport` or `transparent` (transparent needs `samples`).
    pub background: String,
    /// Path trace every frame with this many samples; 0 draws the viewport as shown.
    pub samples: u32,
    pub gif: Option<String>,
    pub dir: Option<String>,
    pub stem: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            frames: 36,
            fps: 24,
            width: 0,
            height: 0,
            spin: "camera".into(),
            rotate_env: false,
            background: "viewport".into(),
            samples: 0,
            gif: None,
            dir: None,
            stem: "turntable".into(),
        }
    }
}

/// The Display panel's turntable settings and the running job.
pub struct State {
    pub opts: Options,
    pub gif_on: bool,
    pub png_on: bool,
    job: Option<Job>,
}

impl Default for State {
    fn default() -> Self {
        State { opts: Options::default(), gif_on: true, png_on: false, job: None }
    }
}

impl State {
    /// `(frames done, frames total)` while a turntable is being made.
    pub fn progress(&self) -> Option<(u32, u32)> {
        self.job.as_ref().map(|j| (j.shared.done.load(Ordering::Relaxed), j.shared.total))
    }

    /// Ask the running job to stop; it removes what it wrote and restores the camera.
    pub fn cancel(&self) {
        if let Some(j) = &self.job {
            j.shared.cancel.store(true, Ordering::Release);
        }
    }
}

struct Shared {
    done: AtomicU32,
    total: u32,
    cancel: AtomicBool,
}

struct Job {
    shared: Arc<Shared>,
    kind: Kind,
}

enum Kind {
    Trace(mpsc::Receiver<Result<Report, Failure>>),
    Live(Box<Live>),
}

struct Report {
    frames: u32,
    files: usize,
    gif: Option<PathBuf>,
}

enum Failure {
    Cancelled,
    Error(String),
}

/// The files being written; on failure or cancel everything written is removed.
struct Sink {
    gif: Option<(GifWriter, PathBuf)>,
    dir: Option<PathBuf>,
    stem: String,
    frames: u32,
    written: Vec<PathBuf>,
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let res = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map_err(|e| format!("cannot write {}: {e}", path.display()))
}

impl Sink {
    fn push(&mut self, i: u32, w: u32, h: u32, rgba: Vec<u8>) -> Result<(), String> {
        if let Some(g) = self.gif.as_mut().map(|(g, _)| g) {
            g.push(&rgba).map_err(|e| e.to_string())?;
        }
        if let Some(dir) = &self.dir {
            let (_, _, png) = crate::control::encode_png(w, h, rgba).ok_or("cannot encode a frame as PNG")?;
            let path = dir.join(frame_name(&self.stem, i, self.frames));
            write_atomic(&path, &png)?;
            self.written.push(path);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Report, Failure> {
        let mut gif = None;
        if let Some((g, path)) = self.gif.take() {
            let bytes = g.finish().map_err(|e| Failure::Error(e.to_string()));
            let res = bytes.and_then(|b| write_atomic(&path, &b).map_err(Failure::Error));
            if let Err(e) = res {
                self.abort();
                return Err(e);
            }
            gif = Some(path);
        }
        Ok(Report { frames: self.frames, files: self.written.len(), gif })
    }

    fn abort(&mut self) {
        for f in self.written.drain(..) {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// Options checked and turned into what a job needs.
struct Plan {
    tt: Turntable,
    transparent: bool,
    samples: u32,
    size: (u32, u32),
    sink: Sink,
}

fn plan(app: &TextureCraftApp, o: &Options) -> Result<Plan, String> {
    let spin = Spin::parse(&o.spin).map_err(|e| e.to_string())?;
    let tt = Turntable::new(o.frames, o.fps, spin, o.rotate_env).map_err(|e| e.to_string())?;
    let transparent = Background::parse(&o.background).map_err(|e| e.to_string())? == Background::Transparent;
    if o.gif.is_none() && o.dir.is_none() {
        return Err("choose an output: a GIF file and/or a folder for PNG frames".into());
    }
    if transparent && o.samples == 0 {
        return Err(tr("A transparent background needs path tracing").into());
    }
    if o.samples > 4096 {
        return Err("samples must be 0 to 4096".into());
    }
    if o.stem.is_empty() || o.stem.contains(['/', '\\']) {
        return Err("the frame name stem must be a plain file name".into());
    }
    if let Some(g) = &o.gif
        && !g.to_ascii_lowercase().ends_with(".gif")
    {
        return Err(format!("the GIF path `{g}` must end in .gif"));
    }
    if o.samples == 0 && app.ui.layout == ViewLayout::Only2D {
        return Err(tr("Show the 3D view to make a turntable from the viewport").into());
    }
    let r = app.viewport_rect;
    let side = |v: u32, d: f32| if v == 0 { (d.round() as u32).clamp(16, MAX_SIDE) } else { v };
    let size = (side(o.width, r.width()), side(o.height, r.height()));
    if !(16..=MAX_SIDE).contains(&size.0) || !(16..=MAX_SIDE).contains(&size.1) {
        return Err(format!("the size must be 16 to {MAX_SIDE} pixels a side"));
    }
    tt.check_size(size.0, size.1).map_err(|e| e.to_string())?;
    let gif = match &o.gif {
        Some(p) => Some((GifWriter::new(size.0, size.1, &tt).map_err(|e| e.to_string())?, PathBuf::from(p))),
        None => None,
    };
    let dir = o.dir.as_ref().map(PathBuf::from);
    if let Some(d) = &dir {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    Ok(Plan { tt, transparent, samples: o.samples, size, sink: Sink { gif, dir, stem: o.stem.clone(), frames: tt.frames, written: Vec::new() } })
}

/// Start a turntable. Errors say what to fix (also for the control channel).
pub fn start(app: &mut TextureCraftApp, ctx: &egui::Context, o: &Options) -> Result<(), String> {
    if app.turntable.job.is_some() {
        return Err(tr("A turntable is already being made").into());
    }
    let Plan { tt, transparent, samples, size, sink } = plan(app, o)?;
    let shared = Arc::new(Shared { done: AtomicU32::new(0), total: tt.frames, cancel: AtomicBool::new(false) });
    let kind = if samples > 0 {
        let scene = crate::render_mode::trace_scene(app);
        let (settings, display, camera) = (crate::render_mode::settings(app), crate::render_mode::display(app), app.session.camera);
        let (rx, tx) = {
            let (tx, rx) = mpsc::channel();
            (rx, tx)
        };
        let (sh, ctx2) = (shared.clone(), ctx.clone());
        let work = move || {
            let r = run_trace(scene, camera, settings, display, (tt, transparent, samples, size), sink, &sh);
            let _ = tx.send(r);
            ctx2.request_repaint();
        };
        std::thread::Builder::new().name("texturecraft-turntable".into()).spawn(work).map_err(|e| format!("cannot start the turntable worker: {e}"))?;
        Kind::Trace(rx)
    } else {
        let base = (app.session.camera.yaw, app.session.doc.project.display.env_rotation);
        Kind::Live(Box::new(Live { tt, sink, i: 0, wait: 0, set: true, base }))
    };
    app.turntable.job = Some(Job { shared, kind });
    ctx.request_repaint();
    Ok(())
}

fn run_trace(
    mut scene: texturecraft_render::trace::TraceScene,
    camera: texturecraft_engine::geom::OrbitCamera,
    settings: texturecraft_render::trace::TraceSettings,
    display: texturecraft_render::trace::TraceDisplay,
    (tt, transparent, samples, size): (Turntable, bool, u32, (u32, u32)),
    mut sink: Sink,
    shared: &Shared,
) -> Result<Report, Failure> {
    let base_env = scene.env_rotation;
    for i in 0..tt.frames {
        let a = tt.angles(i);
        scene.env_rotation = base_env + a.env_rotation.to_radians();
        let mut cam = camera;
        cam.yaw += a.yaw.to_radians();
        let mut tracer = Tracer::new(size.0, size.1);
        for _ in 0..samples {
            if shared.cancel.load(Ordering::Acquire) {
                sink.abort();
                return Err(Failure::Cancelled);
            }
            tracer.pass(&scene, &cam, &settings);
        }
        if shared.cancel.load(Ordering::Acquire) {
            sink.abort();
            return Err(Failure::Cancelled);
        }
        let rgba = if transparent { tracer.rgba8_transparent(&display) } else { tracer.rgba8(&display) };
        if let Err(e) = sink.push(i, tracer.width, tracer.height, rgba) {
            sink.abort();
            return Err(Failure::Error(e));
        }
        shared.done.store(i + 1, Ordering::Relaxed);
    }
    sink.finish()
}

/// A turntable drawn by the live 3D view, one frame per few app frames.
struct Live {
    tt: Turntable,
    sink: Sink,
    /// The frame being set or captured.
    i: u32,
    wait: u8,
    /// Next step sets the view (else it captures it).
    set: bool,
    /// The camera yaw and environment rotation to restore.
    base: (f32, f32),
}

fn restore(app: &mut TextureCraftApp, base: (f32, f32)) {
    app.session.camera.yaw = base.0;
    app.session.doc.project.display.env_rotation = base.1;
}

fn view_is_settling(app: &TextureCraftApp) -> bool {
    app.render_state.as_ref().is_some_and(|rs| {
        rs.renderer.read().callback_resources.get::<crate::gpu::GpuState>().is_some_and(|g| g.renderer.accumulating(crate::viewport::VIEW_3D))
    })
}

fn step_live(app: &mut TextureCraftApp, ctx: &egui::Context, live: &mut Live, shared: &Shared) -> Option<Result<Report, Failure>> {
    if shared.cancel.load(Ordering::Acquire) {
        restore(app, live.base);
        live.sink.abort();
        return Some(Err(Failure::Cancelled));
    }
    ctx.request_repaint();
    if live.set {
        let a = live.tt.angles(live.i);
        app.session.camera.yaw = live.base.0 + a.yaw.to_radians();
        app.session.doc.project.display.env_rotation = live.base.1 + a.env_rotation;
        // The new view is drawn this frame; give the renderer two more before reading it back.
        live.wait = 2;
        live.set = false;
        return None;
    }
    if live.wait > 0 {
        live.wait -= 1;
        return None;
    }
    if view_is_settling(app) {
        return None;
    }
    let Some((w, h, rgba)) = crate::control::capture_rgba(app, crate::viewport::VIEW_3D) else {
        restore(app, live.base);
        live.sink.abort();
        return Some(Err(Failure::Error(tr("The 3D view has not been drawn yet").into())));
    };
    // The first frame fixes the size the GIF was made for.
    if let Err(e) = live.sink.push(live.i, w, h, rgba) {
        restore(app, live.base);
        live.sink.abort();
        return Some(Err(Failure::Error(e)));
    }
    live.i += 1;
    shared.done.store(live.i, Ordering::Relaxed);
    if live.i >= live.tt.frames {
        restore(app, live.base);
        let sink = std::mem::replace(&mut live.sink, Sink { gif: None, dir: None, stem: String::new(), frames: 0, written: Vec::new() });
        return Some(sink.finish());
    }
    live.set = true;
    None
}

/// Drive the running job (call every frame).
pub fn poll(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(mut job) = app.turntable.job.take() else { return };
    let finished = match &mut job.kind {
        Kind::Trace(rx) => match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(Failure::Error(tr("The turntable worker stopped").into()))),
        },
        Kind::Live(live) => step_live(app, ctx, live, &job.shared),
    };
    match finished {
        None => app.turntable.job = Some(job),
        Some(Ok(r)) => {
            let what = match (&r.gif, r.files) {
                (Some(g), 0) => g.display().to_string(),
                (Some(g), n) => format!("{} + {n} PNG", g.display()),
                (None, n) => format!("{n} PNG"),
            };
            app.toast(trf("Turntable exported: {frames} frames ({what})", &[("frames", &r.frames), ("what", &what)]), false);
        }
        Some(Err(Failure::Cancelled)) => app.toast(tr("Turntable cancelled"), false),
        Some(Err(Failure::Error(e))) => app.toast(trf("Couldn't export the turntable: {error}", &[("error", &e)]), true),
    }
}

/// The control method `viewport.turntable {…Options}` (starts the job) and
/// `viewport.turntable.cancel`.
pub fn control(app: &mut TextureCraftApp, ctx: &egui::Context, method: &str, p: &Value) -> Result<Value, String> {
    if method == "viewport.turntable.cancel" {
        app.turntable.cancel();
        return Ok(Value::Null);
    }
    let o: Options = serde_json::from_value(if p.is_null() { json!({}) } else { p.clone() }).map_err(|e| format!("invalid turntable params: {e}"))?;
    start(app, ctx, &o)?;
    Ok(json!({ "started": true, "frames": o.frames }))
}

/// The Display panel section.
pub fn section(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr("Turntable")).strong());
    if let Some((done, total)) = app.turntable.progress() {
        let frac = done as f32 / total.max(1) as f32;
        ui.add(egui::ProgressBar::new(frac).text(trf("Frame {done} of {total}", &[("done", &done), ("total", &total)])));
        if ui.button(tr("Cancel")).clicked() {
            app.turntable.cancel();
        }
        return;
    }
    let st = &mut app.turntable;
    ui.horizontal(|ui| {
        ui.label(tr("Frames"));
        ui.add(egui::DragValue::new(&mut st.opts.frames).range(2..=texturecraft_engine::formats::turntable::MAX_FRAMES));
        ui.label(tr("Frame rate (fps)"));
        ui.add(egui::DragValue::new(&mut st.opts.fps).range(1..=texturecraft_engine::formats::turntable::MAX_FPS));
    });
    egui::ComboBox::from_label(tr("Spin")).selected_text(if st.opts.spin == "environment" { tr("Environment") } else { tr("Camera") }).show_ui(ui, |ui| {
        ui.selectable_value(&mut st.opts.spin, "camera".to_string(), tr("Camera"));
        ui.selectable_value(&mut st.opts.spin, "environment".to_string(), tr("Environment"));
    });
    if st.opts.spin == "camera" {
        ui.checkbox(&mut st.opts.rotate_env, tr("The light turns with the camera"));
    }
    let mut traced = st.opts.samples > 0;
    if ui.checkbox(&mut traced, tr("Path trace every frame")).changed() {
        st.opts.samples = if traced { 64 } else { 0 };
    }
    if traced {
        let mut n = st.opts.samples;
        ui.horizontal(|ui| {
            ui.label(tr("Samples"));
            ui.add(egui::DragValue::new(&mut n).range(1..=4096));
        });
        st.opts.samples = n;
        let mut transparent = st.opts.background == "transparent";
        if ui.checkbox(&mut transparent, tr("Transparent background")).changed() {
            st.opts.background = if transparent { "transparent" } else { "viewport" }.into();
        }
        ui.horizontal(|ui| {
            ui.label(tr("Width"));
            ui.add(egui::DragValue::new(&mut st.opts.width).range(0..=MAX_SIDE));
            ui.label(tr("Height"));
            ui.add(egui::DragValue::new(&mut st.opts.height).range(0..=MAX_SIDE));
        });
    } else {
        st.opts.background = "viewport".into();
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut st.gif_on, tr("Animated GIF"));
        ui.checkbox(&mut st.png_on, tr("PNG frames"));
    });
    let ready = st.gif_on || st.png_on;
    if ui.add_enabled(ready, egui::Button::new(tr("Export turntable..."))).clicked() {
        let mut o = app.turntable.opts.clone();
        let (gif_on, png_on) = (app.turntable.gif_on, app.turntable.png_on);
        let name = texturecraft_engine::formats::export::sanitize_file_stem(&app.session.doc.project.name);
        o.stem = name.clone();
        o.gif = None;
        o.dir = None;
        if gif_on {
            let Some(p) = rfd::FileDialog::new().add_filter(tr("Animated GIF"), &["gif"]).set_file_name(format!("{name}_turntable.gif")).save_file() else {
                return;
            };
            o.gif = Some(p.display().to_string());
        }
        if png_on {
            let Some(d) = rfd::FileDialog::new().set_title(tr("PNG frames")).pick_folder() else { return };
            o.dir = Some(d.display().to_string());
        }
        let ctx = ui.ctx().clone();
        if let Err(e) = start(app, &ctx, &o) {
            app.toast(trf("Couldn't export the turntable: {error}", &[("error", &e)]), true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(dir: &Path) -> Sink {
        Sink { gif: None, dir: Some(dir.to_path_buf()), stem: "t".into(), frames: 3, written: Vec::new() }
    }

    #[test]
    fn options_read_from_control_params() {
        let o: Options = serde_json::from_value(json!({"frames": 12, "rotateEnv": true, "samples": 8, "gif": "a.gif"})).unwrap();
        assert_eq!((o.frames, o.fps, o.rotate_env, o.samples, o.gif.as_deref()), (12, 24, true, 8, Some("a.gif")));
        assert!(serde_json::from_value::<Options>(json!({"frames": "many"})).is_err());
    }

    #[test]
    fn an_aborted_sink_removes_what_it_wrote() {
        let dir = std::env::temp_dir().join(format!("tc-ui-turntable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = sink(&dir);
        for i in 0..2 {
            s.push(i, 2, 2, vec![255; 16]).unwrap();
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        s.abort();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The worker path end to end on the CPU: frames, size, cancel (no GPU or window needed).
    #[test]
    fn the_trace_worker_writes_frames_and_cancels_cleanly() {
        use texturecraft_engine::Session;
        use texturecraft_engine::geom::primitives;
        let mut s = Session::new_project("t", primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
        s.update();
        let app_scene = || {
            let sets: Vec<_> = s
                .doc
                .project
                .texture_sets
                .iter()
                .zip(&s.caches)
                .map(|(ts, c)| texturecraft_render::trace::TraceSet { material: ts.material, composite: c.composite.clone(), height_strength: 1.0 })
                .collect();
            let d = &s.doc.project.display;
            let (m, b) = texturecraft_render::displace::trace_geometry(&s.mesh, &s.bvh, &sets, d);
            texturecraft_render::trace::TraceScene::for_display(m, b, sets, d, None)
        };
        let display = texturecraft_render::trace::TraceDisplay::from_display(&s.doc.project.display);
        let settings = texturecraft_render::trace::TraceSettings::default();
        let dir = std::env::temp_dir().join(format!("tc-ui-turntable-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tt = Turntable::new(3, 10, Spin::Camera, true).unwrap();
        let shared = Shared { done: AtomicU32::new(0), total: 3, cancel: AtomicBool::new(false) };
        let gif = GifWriter::new(24, 16, &tt).unwrap();
        let sk = Sink { gif: Some((gif, dir.join("t.gif"))), dir: Some(dir.clone()), stem: "t".into(), frames: 3, written: Vec::new() };
        let r = run_trace(app_scene(), s.camera, settings, display, (tt, false, 2, (24, 16)), sk, &shared).ok().unwrap();
        assert_eq!((r.frames, r.files), (3, 3));
        assert_eq!(shared.done.load(Ordering::Relaxed), 3);
        assert!(dir.join("t.gif").is_file() && dir.join("t_0002.png").is_file());

        // Cancelled before the first pass: nothing new is written.
        let dir2 = dir.join("c");
        std::fs::create_dir_all(&dir2).unwrap();
        let shared = Shared { done: AtomicU32::new(0), total: 3, cancel: AtomicBool::new(true) };
        let r = run_trace(app_scene(), s.camera, settings, display, (tt, false, 2, (24, 16)), sink(&dir2), &shared);
        assert!(matches!(r, Err(Failure::Cancelled)));
        assert_eq!(std::fs::read_dir(&dir2).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
