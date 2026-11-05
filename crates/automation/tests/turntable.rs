//! Turntable export, headless: frame count and size, camera and environment angles, transparent
//! background, GIF round trip, cancel safety, bad requests and the `turntable` RPC method. The
//! frames are path traced on the CPU (`cpu: true`), so no GPU or display is needed.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use texturecraft_automation::{AuthorizedWorkspace, AutomationError, Filesystem, Headless, TurntableParams, rpc};
use texturecraft_engine::Session;
use texturecraft_engine::formats::turntable::{Background, Spin, Turntable};
use texturecraft_engine::geom::primitives;

fn headless(mesh: texturecraft_engine::geom::Mesh, fs: Filesystem) -> Headless {
    Headless::new(Session::new_project("t", mesh, "builtin:cube", None, 128).unwrap(), fs)
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tc-turntable-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn params(frames: u32, spin: Spin, rotate_env: bool) -> TurntableParams {
    TurntableParams {
        turntable: Turntable::new(frames, 12, spin, rotate_env).unwrap(),
        width: Some(40),
        height: Some(30),
        samples: Some(6),
        cpu: true,
        ..Default::default()
    }
}

fn s(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

fn run(h: &mut Headless, p: &TurntableParams) -> Result<texturecraft_automation::TurntableReport, AutomationError> {
    h.turntable(p, &AtomicBool::new(false), &mut |_, _| {})
}

fn rgba(path: &str) -> image::RgbaImage {
    image::open(path).unwrap().into_rgba8()
}

/// Mean absolute difference of the RGB channels, 0..255.
fn diff(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions());
    let sum: u64 = a.pixels().zip(b.pixels()).map(|(p, q)| (0..3).map(|i| u64::from(p[i].abs_diff(q[i]))).sum::<u64>()).sum();
    sum as f64 / (a.width() * a.height() * 3) as f64
}

#[test]
fn writes_a_gif_and_a_png_sequence_with_the_asked_frames_and_size() {
    let dir = tmp("both");
    let mut h = headless(primitives::cube(2.0), Filesystem::TrustedLocal);
    let mut p = params(6, Spin::Camera, false);
    p.gif = Some(s(&dir.join("spin.gif")));
    p.frames_dir = Some(s(&dir.join("frames")));
    p.stem = "cube".into();
    let mut ticks = Vec::new();
    let r = h.turntable(&p, &AtomicBool::new(false), &mut |done, total| ticks.push((done, total))).unwrap();
    assert_eq!((r.frames, r.width, r.height), (6, 40, 30));
    assert_eq!(ticks, (1..=6).map(|i| (i, 6)).collect::<Vec<_>>(), "progress after every frame");
    assert_eq!(r.files.len(), 6);
    for (i, f) in r.files.iter().enumerate() {
        assert!(f.ends_with(&format!("cube_{i:04}.png")), "{f}");
        assert_eq!(rgba(f).dimensions(), (40, 30));
    }
    let on_disk = std::fs::read_dir(dir.join("frames")).unwrap().count();
    assert_eq!(on_disk, 6, "no stray files");

    let bytes = std::fs::read(dir.join("spin.gif")).unwrap();
    assert_eq!(bytes.len(), r.gif_bytes);
    let mut dec = gif::DecodeOptions::new().read_info(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!((dec.width(), dec.height()), (40, 30));
    let mut n = 0;
    while let Some(f) = dec.read_next_frame().unwrap() {
        assert_eq!(f.delay, 8, "12 fps is 8 hundredths of a second");
        n += 1;
    }
    assert_eq!(n, 6);
    assert_eq!(dec.repeat(), gif::Repeat::Infinite);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_session_is_left_as_it_was() {
    let dir = tmp("restore");
    let mut h = headless(primitives::cube(2.0), Filesystem::TrustedLocal);
    h.session.doc.project.display.env_rotation = 30.0;
    h.session.camera.yaw = 0.5;
    let (cam, env) = (h.session.camera, h.session.doc.project.display.env_rotation);
    let mut p = params(3, Spin::Camera, true);
    p.frames_dir = Some(s(&dir));
    run(&mut h, &p).unwrap();
    assert_eq!((h.session.camera, h.session.doc.project.display.env_rotation), (cam, env));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A cube looks the same every quarter turn, so if the environment turns with the camera
/// (`rotate_env`, in the same direction) all four frames match; with a fixed environment the
/// lighting changes between them. This pins the camera direction and the environment sign.
#[test]
fn the_environment_turns_with_the_camera_in_the_same_direction() {
    let dir = tmp("angles");
    let mut h = headless(primitives::cube(2.0), Filesystem::TrustedLocal);
    let frames = |h: &mut Headless, sub: &str, follow: bool| {
        let mut p = params(4, Spin::Camera, follow);
        p.samples = Some(24);
        p.frames_dir = Some(s(&dir.join(sub)));
        run(h, &p).unwrap().files.iter().map(|f| rgba(f)).collect::<Vec<_>>()
    };
    let follow = frames(&mut h, "follow", true);
    let fixed = frames(&mut h, "fixed", false);
    let follow_spread = (1..4).map(|i| diff(&follow[0], &follow[i])).fold(0.0, f64::max);
    let fixed_spread = (1..4).map(|i| diff(&fixed[0], &fixed[i])).fold(0.0, f64::max);
    assert!(follow_spread < 1.5, "co-rotating lighting: quarter turns of a cube look alike ({follow_spread})");
    assert!(fixed_spread > 3.0 * follow_spread.max(0.3), "fixed lighting changes the shading ({fixed_spread} vs {follow_spread})");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transparent_background_has_alpha_and_environment_spin_keeps_the_silhouette() {
    let dir = tmp("alpha");
    let mut h = headless(primitives::cube(2.0), Filesystem::TrustedLocal);
    let alpha = |img: &image::RgbaImage| img.pixels().map(|p| p[3]).collect::<Vec<_>>();

    // Environment spin: the camera never moves, so every frame covers the same pixels.
    let mut p = params(4, Spin::Environment, false);
    p.background = Background::Transparent;
    p.frames_dir = Some(s(&dir.join("env")));
    p.gif = Some(s(&dir.join("env.gif")));
    let r = run(&mut h, &p).unwrap();
    let imgs: Vec<_> = r.files.iter().map(|f| rgba(f)).collect();
    assert_eq!(imgs[0].get_pixel(0, 0)[3], 0, "the corner shows nothing");
    assert_eq!(imgs[0].get_pixel(20, 15)[3], 255, "the cube is opaque");
    for i in imgs.iter().skip(1) {
        assert_eq!(alpha(i), alpha(&imgs[0]));
    }
    assert!((1..4).any(|i| diff(&imgs[0], &imgs[i]) > 0.5), "the lighting turns");

    let mut dec = {
        let mut o = gif::DecodeOptions::new();
        o.set_color_output(gif::ColorOutput::RGBA);
        o.read_info(std::io::Cursor::new(std::fs::read(dir.join("env.gif")).unwrap())).unwrap()
    };
    let f = dec.read_next_frame().unwrap().unwrap();
    assert_eq!(f.buffer[3], 0, "the GIF keeps the transparent corner");
    assert_eq!(f.buffer[(15 * 40 + 20) * 4 + 3], 255);

    // Camera spin: the cube turns 45 degrees, so its silhouette changes.
    let mut p = params(8, Spin::Camera, false);
    p.background = Background::Transparent;
    p.frames_dir = Some(s(&dir.join("cam")));
    let r = run(&mut h, &p).unwrap();
    assert_ne!(alpha(&rgba(&r.files[0])), alpha(&rgba(&r.files[1])));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancel_stops_early_and_leaves_nothing_behind() {
    let dir = tmp("cancel");
    let mut h = headless(primitives::cube(2.0), Filesystem::TrustedLocal);
    h.session.doc.project.display.env_rotation = 30.0;
    let mut p = params(10, Spin::Camera, true);
    p.gif = Some(s(&dir.join("spin.gif")));
    p.frames_dir = Some(s(&dir.join("frames")));
    let cancel = AtomicBool::new(false);
    let mut seen = 0;
    let err = h
        .turntable(&p, &cancel, &mut |done, _| {
            seen = done;
            if done == 3 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
    assert!(matches!(err, AutomationError::Cancelled), "{err}");
    assert_eq!(seen, 3, "no frame starts after the cancel");
    assert_eq!(std::fs::read_dir(dir.join("frames")).unwrap().count(), 0, "the frames written so far are removed");
    assert!(!dir.join("spin.gif").exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "only the empty frames folder remains");
    assert_eq!(h.session.doc.project.display.env_rotation, 30.0, "the environment is restored");

    // Already cancelled: nothing is created at all.
    let again = tmp("cancel-early");
    let mut p = params(4, Spin::Camera, false);
    p.frames_dir = Some(s(&again.join("never")));
    let err = h.turntable(&p, &AtomicBool::new(true), &mut |_, _| {}).unwrap_err();
    assert!(matches!(err, AutomationError::Cancelled));
    assert!(!again.join("never").exists());

    // Cancelling a transparent run (path tracing checks between passes) works the same.
    let mut p = params(4, Spin::Camera, false);
    p.background = Background::Transparent;
    p.frames_dir = Some(s(&dir.join("t")));
    let cancel = AtomicBool::new(false);
    let err = h.turntable(&p, &cancel, &mut |_, _| cancel.store(true, Ordering::Relaxed)).unwrap_err();
    assert!(matches!(err, AutomationError::Cancelled));
    assert_eq!(std::fs::read_dir(dir.join("t")).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&again);
}

#[test]
fn bad_requests_are_errors_not_panics() {
    let dir = tmp("bad");
    let mut h = headless(primitives::cube(2.0), Filesystem::TrustedLocal);
    let msg = |h: &mut Headless, p: &TurntableParams| run(h, p).unwrap_err().to_string();

    assert!(msg(&mut h, &params(4, Spin::Camera, false)).contains("needs an output"));
    let mut p = params(4, Spin::Camera, false);
    p.gif = Some(s(&dir.join("x.png")));
    assert!(msg(&mut h, &p).contains(".gif"));
    p.gif = Some(s(&dir.join("x.gif")));
    p.background = Background::Transparent;
    p.samples = None;
    assert!(msg(&mut h, &p).contains("samples"));
    p.background = Background::Viewport;
    p.samples = Some(0);
    assert!(msg(&mut h, &p).contains("samples"));
    p.samples = Some(2);
    p.width = Some(5);
    assert!(msg(&mut h, &p).contains("16"));
    p.width = Some(4096);
    p.height = Some(4096);
    p.turntable = Turntable::new(720, 24, Spin::Camera, false).unwrap();
    assert!(msg(&mut h, &p).contains("too many pixels"));
    let mut p = params(4, Spin::Camera, false);
    p.gif = Some(s(&dir.join("x.gif")));
    p.stem = "../evil".into();
    assert!(msg(&mut h, &p).contains("stem"));
    p.stem = "ok".into();
    p.pitch = Some(f32::NAN);
    assert!(msg(&mut h, &p).contains("finite"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing written");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_rpc_method_writes_inside_the_workspace_only() {
    let dir = tmp("rpc");
    let ws = Filesystem::Workspace(AuthorizedWorkspace::new(Some(&dir), Some(&dir)).unwrap());
    let h = Mutex::new(headless(primitives::cube(2.0), ws));
    let req = |params: serde_json::Value| rpc::respond(&h, &json!({"id": 1, "method": "turntable", "params": params}).to_string());

    let ok = req(
        json!({"gif": "spin.gif", "dir": "frames", "frames": 3, "fps": 10, "width": 32, "height": 24, "samples": 2, "cpu": true, "spin": "environment", "background": "transparent"}),
    );
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!((ok["result"]["frames"].as_u64(), ok["result"]["width"].as_u64()), (Some(3), Some(32)));
    assert_eq!(ok["result"]["files"].as_array().map(Vec::len), Some(3));
    assert!(dir.join("spin.gif").is_file() && dir.join("frames/turntable_0002.png").is_file());

    for bad in [
        json!({"gif": "../escape.gif", "frames": 2, "samples": 1, "cpu": true}),
        json!({"gif": "/tmp/abs.gif", "frames": 2, "samples": 1, "cpu": true}),
        json!({"gif": "a.gif", "frames": 1}),
        json!({"gif": "a.gif", "spin": "sideways"}),
        json!({"gif": "a.gif", "frames": "many"}),
        json!({"gif": "a.gif", "width": 4096, "height": 4096}),
    ] {
        let r = req(bad.clone());
        assert_eq!(r["ok"], false, "{bad} -> {r}");
    }
    assert!(!dir.join("../escape.gif").exists());
    let methods = rpc::respond(&h, r#"{"id":2,"method":"methods"}"#);
    assert!(methods["result"].as_array().is_some_and(|m| m.iter().any(|v| v == "turntable")));
    let _ = std::fs::remove_dir_all(&dir);
}
