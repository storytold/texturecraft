//! Headless turntable export: render N frames of a 360-degree spin through the same entry point
//! as `render` ([`Headless::render_png`], so size, path tracing and any render option added there
//! apply) and pack them as an animated GIF and/or a numbered PNG sequence.
//!
//! The spin never edits the document: the camera angle is a per-frame override and the
//! environment rotation is set for each frame and restored afterwards, also on cancel or error.

use std::sync::atomic::{AtomicBool, Ordering};

use texturecraft_engine::formats::turntable::{Background, GifWriter, Turntable, frame_name};
use texturecraft_render::trace::TraceSettings;

use crate::budgets::{MAX_LOCAL_RENDER_SIDE, MAX_PNG_SIDE, encode_png};
use crate::headless::{MAX_TRACE_SAMPLES, RenderOptions};
use crate::{AutomationError, Headless, bad};

/// What a turntable renders and where it goes.
#[derive(Clone, Debug)]
pub struct TurntableParams {
    pub turntable: Turntable,
    pub background: Background,
    /// Pixels; default the session viewport.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Camera pitch override in degrees (default the session camera's).
    pub pitch: Option<f32>,
    /// Path trace each frame with this many samples (default: the GPU viewport).
    pub samples: Option<u32>,
    pub bounces: Option<u32>,
    /// Path trace on the CPU even when a GPU with compute shaders exists.
    pub cpu: bool,
    /// Write the animated GIF here.
    pub gif: Option<String>,
    /// Write the PNG sequence into this directory (created if missing).
    pub frames_dir: Option<String>,
    /// File name stem of the PNG frames: `<stem>_0000.png`.
    pub stem: String,
}

impl Default for TurntableParams {
    fn default() -> Self {
        TurntableParams {
            turntable: Turntable::default(),
            background: Background::Viewport,
            width: None,
            height: None,
            pitch: None,
            samples: None,
            bounces: None,
            cpu: false,
            gif: None,
            frames_dir: None,
            stem: "turntable".into(),
        }
    }
}

/// What a finished turntable wrote.
#[derive(Clone, Debug)]
pub struct TurntableReport {
    pub frames: u32,
    pub width: u32,
    pub height: u32,
    /// What drew the first frame (the GPU adapter, or the CPU path tracer).
    pub by: String,
    pub gif: Option<String>,
    pub gif_bytes: usize,
    /// The PNG frames, in order.
    pub files: Vec<String>,
}

fn join(dir: &str, name: &str) -> String {
    if dir == "." { name.to_string() } else { format!("{}/{name}", dir.trim_end_matches('/')) }
}

fn te(e: impl std::fmt::Display) -> AutomationError {
    bad(e.to_string())
}

impl Headless {
    /// Render the turntable and write it. `progress(done, total)` is called after every frame;
    /// setting `cancel` stops before the next frame (a frame in the middle of a GPU render or a
    /// GPU path trace finishes first) and returns [`AutomationError::Cancelled`]. A cancelled or
    /// failed run leaves no output behind: the PNG frames already written are removed and the GIF
    /// is only written once every frame is encoded (atomically, like every file here).
    pub fn turntable(&mut self, p: &TurntableParams, cancel: &AtomicBool, progress: &mut dyn FnMut(u32, u32)) -> Result<TurntableReport, AutomationError> {
        if p.gif.is_none() && p.frames_dir.is_none() {
            return Err(bad("a turntable needs an output: `gif` (a file) and/or `dir` (PNG frames)"));
        }
        if let Some(path) = &p.gif
            && !path.to_ascii_lowercase().ends_with(".gif")
        {
            return Err(bad(format!("the GIF path `{path}` must end in .gif")));
        }
        if p.stem.is_empty() || p.stem.contains(['/', '\\']) {
            return Err(bad("the frame name stem must be a plain file name"));
        }
        if let Some(n) = p.samples
            && !(1..=MAX_TRACE_SAMPLES).contains(&n)
        {
            return Err(bad(format!("samples must be 1 to {MAX_TRACE_SAMPLES}")));
        }
        if p.background == Background::Transparent && p.samples.is_none() {
            return Err(bad("a transparent background needs path tracing: set `samples` (the viewport render has no coverage mask)"));
        }
        if p.pitch.is_some_and(|v| !v.is_finite()) {
            return Err(bad("camera angles must be finite"));
        }
        let max = if self.trusted() { MAX_LOCAL_RENDER_SIDE } else { MAX_PNG_SIDE };
        let side = |v: Option<u32>, d: f32| {
            let v = v.unwrap_or(d.round() as u32);
            if (16..=max).contains(&v) { Ok(v) } else { Err(bad(format!("render size {v} must be 16 to {max} pixels"))) }
        };
        let (width, height) = (side(p.width, self.session.viewport.x)?, side(p.height, self.session.viewport.y)?);
        p.turntable.check_size(width, height).map_err(te)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(AutomationError::Cancelled);
        }
        if let Some(dir) = &p.frames_dir {
            self.create_dir_all(dir)?;
        }

        let base_env = self.session.doc.project.display.env_rotation;
        let mut written = Vec::new();
        let result = self.turntable_run(p, (width, height), cancel, progress, &mut written);
        // Whatever happened, the document is as it was.
        self.session.doc.project.display.env_rotation = base_env;
        match result {
            Ok(report) => Ok(report),
            Err(e) => {
                for f in &written {
                    let _ = self.remove_file(f);
                }
                Err(e)
            }
        }
    }

    fn turntable_run(
        &mut self,
        p: &TurntableParams,
        (width, height): (u32, u32),
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u32, u32),
        written: &mut Vec<String>,
    ) -> Result<TurntableReport, AutomationError> {
        let t = p.turntable;
        let base_yaw = self.session.camera.yaw.to_degrees();
        let base_env = self.session.doc.project.display.env_rotation;
        let mut gif = match &p.gif {
            Some(_) => Some(GifWriter::new(width, height, &t).map_err(te)?),
            None => None,
        };
        let mut by = String::new();
        for i in 0..t.frames {
            if cancel.load(Ordering::Relaxed) {
                return Err(AutomationError::Cancelled);
            }
            let a = t.angles(i);
            self.session.doc.project.display.env_rotation = base_env + a.env_rotation;
            let yaw = base_yaw + a.yaw;
            let (png, rgba, drawn_by) = match p.background {
                Background::Viewport => {
                    let o = RenderOptions {
                        view: None,
                        width: Some(width),
                        height: Some(height),
                        yaw: Some(yaw),
                        pitch: p.pitch,
                        samples: p.samples,
                        bounces: p.bounces,
                        cpu: p.cpu,
                    };
                    let r = self.render_png(&o)?;
                    // Only a GIF needs the pixels back.
                    let rgba = match gif {
                        Some(_) => Some(decode_rgba(&r.png, width, height)?),
                        None => None,
                    };
                    (r.png, rgba, r.by)
                }
                Background::Transparent => {
                    let mut camera = self.session.camera;
                    camera.yaw = yaw.clamp(-1e6, 1e6).to_radians();
                    if let Some(v) = p.pitch {
                        camera.pitch = v.clamp(-89.0, 89.0).to_radians();
                    }
                    let settings = TraceSettings { bounces: p.bounces.unwrap_or(6).clamp(1, 32) };
                    let samples = p.samples.unwrap_or(1);
                    let Some((w, h, px)) = crate::render::path_trace_transparent(&mut self.session, [width, height], camera, samples, &settings, cancel) else {
                        return Err(AutomationError::Cancelled);
                    };
                    (encode_png(w, h, px.clone())?, Some(px), "CPU path tracer".to_string())
                }
            };
            if i == 0 {
                by = drawn_by;
            }
            if let Some(dir) = &p.frames_dir {
                let path = join(dir, &frame_name(&p.stem, i, t.frames));
                self.write(&path, &png)?;
                written.push(path);
            }
            if let (Some(g), Some(px)) = (gif.as_mut(), rgba.as_deref()) {
                g.push(px).map_err(te)?;
            }
            progress(i + 1, t.frames);
        }
        let mut gif_bytes = 0;
        if let (Some(g), Some(path)) = (gif, &p.gif) {
            let bytes = g.finish().map_err(te)?;
            gif_bytes = bytes.len();
            self.write(path, &bytes)?;
        }
        Ok(TurntableReport { frames: t.frames, width, height, by, gif: p.gif.clone(), gif_bytes, files: std::mem::take(written) })
    }
}

/// The RGBA8 pixels of a PNG this module just rendered (size checked).
fn decode_rgba(png: &[u8], width: u32, height: u32) -> Result<Vec<u8>, AutomationError> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| AutomationError::Other(format!("cannot read the rendered frame: {e}")))?
        .into_rgba8();
    if img.dimensions() != (width, height) {
        return Err(AutomationError::Other("a rendered frame has the wrong size".into()));
    }
    Ok(img.into_raw())
}
