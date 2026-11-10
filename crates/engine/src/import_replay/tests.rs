//! Replays of synthetic strokes on a synthetic wall.

use std::sync::Arc;

use texturecraft_core::LayerId;
use texturecraft_geom::glam::Mat4;
use texturecraft_geom::primitives;
use texturecraft_import::replay::{StrokePoint, StrokeSpace};
use texturecraft_texel::from_u16;

use super::*;

const RES: u32 = 128;

/// A 2 × 2 wall at z = 0 facing +z; texture coordinates from its corners, the top row up.
fn wall() -> Mesh {
    let mut m = primitives::plane(2.0, 16);
    for p in &mut m.positions {
        *p = Vec3::new(p.x, -p.z, 0.0);
    }
    for n in &mut m.normals {
        *n = Vec3::Z;
    }
    m
}

fn camera(up: Vec3) -> StrokeSpace {
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, up);
    let projection = Mat4::perspective_rh_gl(40f32.to_radians(), 1.0, 0.1, 100.0);
    StrokeSpace::Mesh { view: view.to_cols_array(), projection: projection.to_cols_array() }
}

fn stroke(space: StrokeSpace, points: &[[f32; 2]]) -> ReplayStroke {
    ReplayStroke {
        space,
        points: points.iter().map(|p| StrokePoint { position: *p, pressure: 1.0 }).collect(),
        seed: 1,
        lazy: 0.0,
        copies: 0,
        copy_matrix: [0.0; 16],
    }
}

fn brush(radius: f32) -> ReplayBrush {
    ReplayBrush {
        radius,
        spacing: 0.1,
        flow: 1.0,
        size_pressure: None,
        flow_pressure: None,
        size_jitter: 0.0,
        flow_jitter: 0.0,
        position_jitter: 0.0,
        rotation: 0.0,
        rotation_jitter: 0.0,
        follow: false,
        alignment: TipAlignment::Tangent,
        culling: Some(0.5),
        tip: ReplayTip::Round { hardness: 0.9 },
    }
}

fn step(mode: PaintMode, strokes: Vec<ReplayStroke>) -> ReplayStep {
    ReplayStep { mode, values: vec![(Channel::BaseColor, [1.0, 0.0, 0.0], 1.0)], mask_value: 0.5, opacity: 1.0, brush: brush(0.1), strokes, notes: vec![] }
}

fn mask_job(strokes: Vec<ReplayStroke>) -> ReplayJob {
    ReplayJob { target: ReplayTarget::Mask(LayerId(7)), label: "test".into(), steps: vec![step(PaintMode::Paint, strokes)] }
}

fn replay(mesh: &Mesh, jobs: &[ReplayJob]) -> Vec<Result<ReplayedPixels, String>> {
    BrushReplayer.replay(&ReplayScene { mesh, material: 0, resolution: RES }, jobs)
}

/// The mask's (grey, coverage) at texel `(x, y)`.
fn mask_at(r: &Result<ReplayedPixels, String>, x: u32, y: u32) -> (f32, f32) {
    let Ok(ReplayedPixels::Mask(img)) = r else { panic!("a mask: {r:?}") };
    let t = img.get(x, y);
    (from_u16(t[0]), from_u16(t[1]))
}

#[test]
fn a_3d_view_stroke_paints_where_its_camera_saw_it() {
    let mesh = wall();
    let out = replay(&mesh, &[mask_job(vec![stroke(camera(Vec3::Y), &[[-0.3, 0.0], [0.3, 0.0]])])]);
    let c = RES / 2;
    let (grey, cover) = mask_at(&out[0], c, c);
    assert!(cover > 0.95 && (grey - 0.5 * cover).abs() < 0.01, "premultiplied grey {grey}, coverage {cover}");
    // the stroke spans about 0.3 · tan 20° · 5 = 0.55 either side of the centre (0.55 of a half texture)
    assert!(mask_at(&out[0], c + 30, c).1 > 0.9 && mask_at(&out[0], c - 30, c).1 > 0.9);
    assert_eq!(mask_at(&out[0], c + 50, c).1, 0.0);
    assert_eq!(mask_at(&out[0], c, c - 30).1, 0.0, "nothing above the stroke");
}

#[test]
fn a_rolled_camera_turns_its_strokes_on_the_mesh() {
    // the camera's up is +x: its right is −y, so a stroke across the view runs down the wall
    let mesh = wall();
    let out = replay(&mesh, &[mask_job(vec![stroke(camera(Vec3::X), &[[-0.3, 0.0], [0.3, 0.0]])])]);
    let c = RES / 2;
    assert!(mask_at(&out[0], c, c - 30).1 > 0.9 && mask_at(&out[0], c, c + 30).1 > 0.9);
    assert_eq!(mask_at(&out[0], c - 30, c).1, 0.0);
}

#[test]
fn symmetry_copies_apply_the_recorded_matrix() {
    let mesh = wall();
    let mut s = stroke(camera(Vec3::Y), &[[-0.3, 0.2]]);
    s.copies = 1;
    s.copy_matrix = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)).to_cols_array();
    let out = replay(&mesh, &[mask_job(vec![s])]);
    let c = RES / 2;
    // the dab lands left of the centre and above it; its mirror right of it
    let left = (0..c).find(|x| mask_at(&out[0], *x, c - 22).1 > 0.5).unwrap();
    let right = (c..RES).rev().find(|x| mask_at(&out[0], *x, c - 22).1 > 0.5).unwrap();
    assert!((c as i32 - left as i32 - (right as i32 + 1 - c as i32)).abs() <= 1, "{left} {right}");
    assert_eq!(mask_at(&out[0], c, c - 22).1, 0.0);
}

#[test]
fn a_2d_view_stroke_walks_the_texture() {
    let mesh = wall();
    let out = replay(&mesh, &[mask_job(vec![stroke(StrokeSpace::Uv, &[[0.25, 0.5], [0.75, 0.5]])])]);
    let c = RES / 2;
    assert!(mask_at(&out[0], c, c).1 > 0.95 && mask_at(&out[0], 40, c).1 > 0.95);
    assert_eq!(mask_at(&out[0], c, c - 20).1, 0.0, "a radius of 0.1 on a 2 wide wall is 6.4 texels");
    assert_eq!(mask_at(&out[0], 10, c).1, 0.0);
}

#[test]
fn content_paints_values_and_the_eraser_lowers_coverage() {
    let mesh = wall();
    let line = |y: f32| stroke(StrokeSpace::Uv, &[[0.2, y], [0.8, y]]);
    let job = ReplayJob {
        target: ReplayTarget::Content(LayerId(3)),
        label: "test".into(),
        steps: vec![step(PaintMode::Paint, vec![line(0.5)]), step(PaintMode::Erase, vec![stroke(StrokeSpace::Uv, &[[0.5, 0.3], [0.5, 0.7]])])],
    };
    let out = replay(&mesh, &[job]);
    let Ok(ReplayedPixels::Content(images)) = &out[0] else { panic!("content: {:?}", out[0]) };
    let (c, img) = &images[0];
    assert_eq!(*c, Channel::BaseColor);
    let px = |x: u32, y: u32| img.get(x, y).iter().map(|v| from_u16(*v)).collect::<Vec<f32>>();
    let painted = px(40, RES / 2);
    assert!(painted[0] > 0.99 && painted[1] < 0.01 && painted[3] > 0.95, "{painted:?}");
    assert!(px(RES / 2, RES / 2)[3] < 0.05, "erased where the strokes cross");
}

#[test]
fn hostile_jobs_do_not_crash() {
    let mesh = wall();
    let nan = StrokeSpace::Mesh { view: [f32::NAN; 16], projection: [0.0; 16] };
    let flat = StrokeSpace::Mesh { view: [0.0; 16], projection: [0.0; 16] };
    let mut copies = stroke(camera(Vec3::Y), &[[0.0, 0.0], [1e9, -1e9], [f32::MAX, 0.0]]);
    copies.copies = 64;
    copies.copy_matrix = [f32::INFINITY; 16];
    let mut tiny = step(PaintMode::Paint, vec![stroke(StrokeSpace::Uv, &[[0.0, 0.0], [1.0, 1.0]])]);
    tiny.brush.spacing = 0.0;
    tiny.brush.radius = 1e-9;
    tiny.brush.tip = ReplayTip::Image { width: 0, height: 0, values: Arc::from(Vec::new()) };
    let jobs = vec![
        mask_job(vec![stroke(nan, &[[0.0, 0.0]]), stroke(flat, &[[0.0, 0.0]]), copies, stroke(StrokeSpace::Uv, &[])]),
        ReplayJob { target: ReplayTarget::Mask(LayerId(1)), label: String::new(), steps: vec![tiny] },
    ];
    let out = replay(&mesh, &jobs);
    assert!(out[0].is_ok());
    assert!(out[1].is_err(), "an empty tip image is an error");
    // a set without triangles
    let empty = BrushReplayer.replay(&ReplayScene { mesh: &mesh, material: 5, resolution: RES }, &jobs);
    assert!(empty.iter().all(Result::is_err));
}

#[test]
fn a_cancelled_replayer_paints_nothing_more() {
    let mesh = wall();
    let jobs = vec![mask_job(vec![stroke(camera(Vec3::Y), &[[0.3, 0.5], [0.7, 0.5]])]); 3];
    let scene = ReplayScene { mesh: &mesh, material: 0, resolution: RES };
    let cancel = texturecraft_import::Cancel::new();
    let running = CancellableReplayer(cancel.clone()).replay(&scene, &jobs);
    assert!(running.iter().all(Result::is_ok), "an unset flag replays as the plain replayer does");
    cancel.cancel();
    let stopped = CancellableReplayer(cancel).replay(&scene, &jobs);
    assert_eq!(stopped.len(), jobs.len());
    assert!(stopped.iter().all(|r| r.as_ref().is_err_and(|e| e == "cancelled")), "{stopped:?}");
}
