//! Turntable export: the angles of an N-frame 360-degree spin, option checks, the animated GIF
//! encoder and the PNG frame names. UI-free and GPU-free (it never renders): the headless CLI and
//! the app render the frames, this module says which view each frame shows and packs the pixels.
//!
//! GIF is the only animated format: MP4/WebM have no pure-Rust encoder that is both practical
//! and license-compatible (H.264 encoders are C/C++ or patent-encumbered, the pure-Rust AV1
//! encoder `rav1e` is BSD-2-Clause but a heavy build that needs an assembler on x86). Frames also
//! export as a numbered PNG sequence (full alpha) that any video tool turns into MP4 or WebM.

use thiserror::Error;

/// Most frames one turntable may have.
pub const MAX_FRAMES: u32 = 720;
/// Highest frame rate (GIF delays are whole hundredths of a second, so the real rate is `100 / round(100 / fps)`).
pub const MAX_FPS: u32 = 60;
/// Largest GIF a turntable may encode (it is built in memory).
pub const MAX_GIF_BYTES: usize = 256 << 20;
/// Pixels a turntable may render in total (frames x width x height): keeps one request bounded.
pub const MAX_TOTAL_PIXELS: u64 = 1 << 31;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TurntableError {
    #[error("{0}")]
    Invalid(String),
    #[error("GIF encoding failed: {0}")]
    Encode(String),
}

fn invalid(m: impl Into<String>) -> TurntableError {
    TurntableError::Invalid(m.into())
}

/// What turns around the up (+Y) axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spin {
    /// The camera orbits the model; the environment stays put unless `rotate_env` is set.
    #[default]
    Camera,
    /// The camera stays; only the environment (and the key light and shadows that live in it) turns.
    Environment,
}

impl Spin {
    pub fn parse(s: &str) -> Result<Spin, TurntableError> {
        match s.to_ascii_lowercase().as_str() {
            "camera" => Ok(Spin::Camera),
            "environment" | "env" => Ok(Spin::Environment),
            other => Err(invalid(format!("unknown spin `{other}` (camera or environment)"))),
        }
    }
}

/// What is behind the model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Background {
    /// The viewport's environment (or studio backdrop), as shown on screen.
    #[default]
    Viewport,
    /// Transparent where nothing is hit. Needs path tracing (the CPU tracer measures coverage);
    /// GIF keeps 1-bit transparency, PNG frames keep full alpha.
    Transparent,
}

impl Background {
    pub fn parse(s: &str) -> Result<Background, TurntableError> {
        match s.to_ascii_lowercase().as_str() {
            "viewport" | "environment" | "env" => Ok(Background::Viewport),
            "transparent" => Ok(Background::Transparent),
            other => Err(invalid(format!("unknown background `{other}` (viewport or transparent)"))),
        }
    }
}

/// The shape of a turntable: how many frames, how fast, what turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turntable {
    pub frames: u32,
    pub fps: u32,
    pub spin: Spin,
    /// With [`Spin::Camera`]: turn the environment (and its key light) with the camera, so the
    /// lighting stays fixed relative to the camera and the model seems to spin in place.
    pub rotate_env: bool,
}

impl Default for Turntable {
    fn default() -> Self {
        Turntable { frames: 36, fps: 24, spin: Spin::Camera, rotate_env: false }
    }
}

/// Offsets a frame adds to the session's own camera yaw and environment rotation, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameAngles {
    pub yaw: f32,
    pub env_rotation: f32,
}

impl Turntable {
    /// Check `frames` (2 to [`MAX_FRAMES`]) and `fps` (1 to [`MAX_FPS`]).
    pub fn new(frames: u32, fps: u32, spin: Spin, rotate_env: bool) -> Result<Turntable, TurntableError> {
        if !(2..=MAX_FRAMES).contains(&frames) {
            return Err(invalid(format!("frames {frames} must be 2 to {MAX_FRAMES}")));
        }
        if !(1..=MAX_FPS).contains(&fps) {
            return Err(invalid(format!("fps {fps} must be 1 to {MAX_FPS}")));
        }
        Ok(Turntable { frames, fps, spin, rotate_env })
    }

    /// The turn of frame `i`: `360 * i / frames`, so frame 0 is the start view and the loop closes
    /// without a repeated frame (the 360-degree view would equal frame 0).
    pub fn turn(&self, i: u32) -> f32 {
        (i % self.frames) as f32 * 360.0 / self.frames as f32
    }

    /// Camera and environment offsets of frame `i`. The environment turns in the same direction as
    /// the camera orbit: `env_rotation` rotates the environment about +Y like the camera's yaw
    /// rotates its eye, so equal values keep the lighting still relative to the camera.
    pub fn angles(&self, i: u32) -> FrameAngles {
        let t = self.turn(i);
        match self.spin {
            Spin::Camera => FrameAngles { yaw: t, env_rotation: if self.rotate_env { t } else { 0.0 } },
            Spin::Environment => FrameAngles { yaw: 0.0, env_rotation: t },
        }
    }

    /// Check a render size against the turntable's total pixel budget.
    pub fn check_size(&self, width: u32, height: u32) -> Result<(), TurntableError> {
        let total = u64::from(self.frames) * u64::from(width) * u64::from(height);
        if total > MAX_TOTAL_PIXELS {
            return Err(invalid(format!("{} frames of {width}x{height} are too many pixels; lower the frames or the size", self.frames)));
        }
        Ok(())
    }

    /// GIF frame delay in hundredths of a second (at least 1, GIF's finest step).
    pub fn delay_cs(&self) -> u16 {
        ((100.0 / self.fps as f32).round() as u16).max(1)
    }
}

/// File name of PNG frame `i` of `frames`: `<stem>_0007.png` (zero-padded so it sorts).
pub fn frame_name(stem: &str, i: u32, frames: u32) -> String {
    let digits = frames.saturating_sub(1).to_string().len().max(4);
    format!("{stem}_{i:0digits$}.png")
}

/// Builds an animated GIF in memory, one frame at a time (frames are not kept).
pub struct GifWriter {
    enc: Option<gif::Encoder<Vec<u8>>>,
    width: u16,
    height: u16,
    delay: u16,
    frames: u32,
}

impl GifWriter {
    /// A looping GIF of `width` x `height` pixels at `turntable`'s frame rate.
    pub fn new(width: u32, height: u32, turntable: &Turntable) -> Result<GifWriter, TurntableError> {
        let (w, h) = (u16::try_from(width), u16::try_from(height));
        let (Ok(w), Ok(h)) = (w, h) else { return Err(invalid("GIF sides are at most 65535 pixels")) };
        if w == 0 || h == 0 {
            return Err(invalid("GIF sides must be at least 1 pixel"));
        }
        let mut enc = gif::Encoder::new(Vec::new(), w, h, &[]).map_err(|e| TurntableError::Encode(e.to_string()))?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| TurntableError::Encode(e.to_string()))?;
        Ok(GifWriter { enc: Some(enc), width: w, height: h, delay: turntable.delay_cs(), frames: 0 })
    }

    /// Add a frame of RGBA8 pixels (`width * height * 4` bytes). Alpha under 128 becomes the GIF's
    /// transparent color.
    pub fn push(&mut self, rgba: &[u8]) -> Result<(), TurntableError> {
        let want = usize::from(self.width) * usize::from(self.height) * 4;
        if rgba.len() != want {
            return Err(invalid(format!("frame has {} bytes, expected {want}", rgba.len())));
        }
        let mut px = rgba.to_vec();
        for p in px.chunks_exact_mut(4) {
            if p[3] < 128 {
                p.copy_from_slice(&[0, 0, 0, 0]);
            } else {
                p[3] = 255;
            }
        }
        // Speed 10 = the encoder's fast palette search (1 is best quality, 30 is fastest).
        let mut frame = gif::Frame::from_rgba_speed(self.width, self.height, &mut px, 10);
        frame.delay = self.delay;
        // Each frame repaints the whole canvas (no stacking of transparent frames).
        frame.dispose = gif::DisposalMethod::Background;
        let enc = self.enc.as_mut().ok_or_else(|| TurntableError::Encode("the GIF is already finished".into()))?;
        enc.write_frame(&frame).map_err(|e| TurntableError::Encode(e.to_string()))?;
        self.frames += 1;
        if enc.get_ref().len() > MAX_GIF_BYTES {
            return Err(invalid("the GIF is over the 256 MiB limit; lower the frames or the size"));
        }
        Ok(())
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The finished file. Errors when no frame was added.
    pub fn finish(mut self) -> Result<Vec<u8>, TurntableError> {
        if self.frames == 0 {
            return Err(invalid("a GIF needs at least one frame"));
        }
        let enc = self.enc.take().ok_or_else(|| TurntableError::Encode("the GIF is already finished".into()))?;
        enc.into_inner().map_err(|e| TurntableError::Encode(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tt(frames: u32) -> Turntable {
        Turntable::new(frames, 24, Spin::Camera, false).expect("valid")
    }

    #[test]
    fn angles_are_evenly_spaced_and_never_repeat_the_start() {
        let t = tt(36);
        let a: Vec<f32> = (0..36).map(|i| t.angles(i).yaw).collect();
        assert_eq!(a[0], 0.0);
        assert!((a[9] - 90.0).abs() < 1e-4 && (a[18] - 180.0).abs() < 1e-4 && (a[27] - 270.0).abs() < 1e-4);
        assert!((a[35] - 350.0).abs() < 1e-3, "the last frame stops one step short of 360");
        for w in a.windows(2) {
            assert!((w[1] - w[0] - 10.0).abs() < 1e-3);
        }
        assert!((t.turn(36) - 0.0).abs() < 1e-6, "past the end wraps to the start");
    }

    #[test]
    fn spin_modes_pick_camera_and_environment_angles() {
        let cam = Turntable::new(8, 24, Spin::Camera, false).unwrap();
        assert_eq!(cam.angles(2), FrameAngles { yaw: 90.0, env_rotation: 0.0 });
        let follow = Turntable::new(8, 24, Spin::Camera, true).unwrap();
        assert_eq!(follow.angles(2), FrameAngles { yaw: 90.0, env_rotation: 90.0 });
        let env = Turntable::new(8, 24, Spin::Environment, true).unwrap();
        assert_eq!(env.angles(2), FrameAngles { yaw: 0.0, env_rotation: 90.0 });
    }

    #[test]
    fn options_are_checked() {
        assert!(Turntable::new(1, 24, Spin::Camera, false).is_err());
        assert!(Turntable::new(MAX_FRAMES + 1, 24, Spin::Camera, false).is_err());
        assert!(Turntable::new(36, 0, Spin::Camera, false).is_err());
        assert!(Turntable::new(36, MAX_FPS + 1, Spin::Camera, false).is_err());
        assert!(Turntable::new(MAX_FRAMES, MAX_FPS, Spin::Camera, false).is_ok());
        assert!(tt(720).check_size(4096, 4096).is_err());
        assert!(tt(36).check_size(1280, 800).is_ok());
        assert!(Spin::parse("sideways").is_err() && Background::parse("pink").is_err());
        assert_eq!((Spin::parse("ENV"), Background::parse("Transparent")), (Ok(Spin::Environment), Ok(Background::Transparent)));
    }

    #[test]
    fn delays_follow_fps() {
        let d = |fps| Turntable::new(2, fps, Spin::Camera, false).unwrap().delay_cs();
        assert_eq!((d(1), d(10), d(24), d(25), d(60)), (100, 10, 4, 4, 2));
    }

    #[test]
    fn frame_names_sort_and_pad() {
        assert_eq!(frame_name("spin", 7, 36), "spin_0007.png");
        assert_eq!(frame_name("spin", 0, 720), "spin_0000.png");
        assert_eq!(frame_name("x", 5, 20000), "x_00005.png");
    }

    /// A frame with a solid color and a quarter transparent corner.
    fn solid(w: u32, h: u32, rgb: [u8; 3], clear_corner: bool) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let clear = clear_corner && x < w / 2 && y < h / 2;
                v.extend_from_slice(&[rgb[0], rgb[1], rgb[2], if clear { 0 } else { 255 }]);
            }
        }
        v
    }

    #[test]
    fn gif_encodes_and_decodes() {
        let t = Turntable::new(3, 10, Spin::Camera, false).unwrap();
        let mut g = GifWriter::new(16, 8, &t).unwrap();
        let colors = [[250, 20, 20], [20, 250, 20], [20, 20, 250]];
        for (i, c) in colors.iter().enumerate() {
            g.push(&solid(16, 8, *c, i == 2)).unwrap();
        }
        assert_eq!(g.frames(), 3);
        let bytes = g.finish().unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");

        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut dec = opts.read_info(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!((dec.width(), dec.height()), (16, 8));
        let mut n = 0;
        while let Some(f) = dec.read_next_frame().unwrap() {
            assert_eq!((f.width, f.height, f.delay), (16, 8, 10));
            let px = &f.buffer;
            let (r, g, b) = (px[(4 * 8 + 12) * 4], px[(4 * 8 + 12) * 4 + 1], px[(4 * 8 + 12) * 4 + 2]);
            let want = colors[n];
            assert!(
                (i32::from(r) - i32::from(want[0])).abs() < 24
                    && (i32::from(g) - i32::from(want[1])).abs() < 24
                    && (i32::from(b) - i32::from(want[2])).abs() < 24,
                "frame {n}: {r},{g},{b}"
            );
            if n == 2 {
                assert_eq!(px[3], 0, "the cleared corner is transparent");
                assert_eq!(px[(4 * 8 + 12) * 4 + 3], 255);
            } else {
                assert_eq!(px[3], 255);
            }
            n += 1;
        }
        assert_eq!(n, 3);
    }

    #[test]
    fn gif_rejects_bad_input() {
        let t = tt(2);
        assert!(GifWriter::new(0, 4, &t).is_err() && GifWriter::new(70_000, 4, &t).is_err());
        let mut g = GifWriter::new(4, 4, &t).unwrap();
        assert!(g.push(&[0; 10]).is_err(), "a frame of the wrong size");
        assert!(g.finish().is_err(), "no frames");
    }
}
