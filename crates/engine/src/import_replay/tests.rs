//! Replays of synthetic strokes on a synthetic wall.

use std::sync::Arc;

use texturecraft_core::LayerId;
use texturecraft_geom::glam::Mat4;
use texturecraft_geom::primitives;
use texturecraft_import::replay::{PathPiece, ReplayPath, ReplayRibbon, RibbonTiling as Tiling, StrokePoint, StrokeSpace};
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
        position_both: true,
        position_normal: true,
        rotation: 0.0,
        rotation_jitter: 0.0,
        follow: false,
        alignment: TipAlignment::Tangent,
        culling: Some(0.5),
        tip: ReplayTip::Round { hardness: 0.9 },
        cycle: vec![],
        cycle_drawn_start: false,
        up: None,
    }
}

fn step(mode: PaintMode, strokes: Vec<ReplayStroke>) -> ReplayStep {
    ReplayStep {
        mode,
        values: vec![(Channel::BaseColor, [1.0, 0.0, 0.0], 1.0)],
        sources: vec![],
        mask_value: 0.5,
        mask_source: None,
        opacity: 1.0,
        brush: brush(0.1),
        stencil: None,
        projection: None,
        placed: None,
        strokes,
        paths: vec![],
        ribbon: None,
        particles: None,
        notes: vec![],
    }
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
fn a_mask_group_eraser_lowers_coverage_and_keeps_the_grey() {
    // a mask group's layer: a brush then an eraser across it; the straight grey stays where coverage is left
    let mesh = wall();
    let job = ReplayJob {
        target: ReplayTarget::Mask(LayerId(9)),
        label: "test".into(),
        steps: vec![
            step(PaintMode::Paint, vec![stroke(StrokeSpace::Uv, &[[0.2, 0.5], [0.8, 0.5]])]),
            step(PaintMode::Erase, vec![stroke(StrokeSpace::Uv, &[[0.5, 0.3], [0.5, 0.7]])]),
        ],
    };
    let out = replay(&mesh, &[job]);
    let (grey, cover) = mask_at(&out[0], 40, RES / 2);
    assert!(cover > 0.95 && (grey - 0.5 * cover).abs() < 0.01, "{grey} {cover}");
    let (grey, cover) = mask_at(&out[0], RES / 2, RES / 2);
    assert!(cover < 0.05 && grey < 0.03, "erased, not painted black at full coverage: {grey} {cover}");
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

/// The mask's coverage along row `y`.
fn row(r: &Result<ReplayedPixels, String>, y: u32) -> Vec<f32> {
    (0..RES).map(|x| mask_at(r, x, y).1).collect()
}

fn job_of(step: ReplayStep) -> ReplayJob {
    ReplayJob { target: ReplayTarget::Mask(LayerId(7)), label: "test".into(), steps: vec![step] }
}

#[test]
fn pen_pressure_lowers_the_flow_in_both_views() {
    let mesh = wall();
    let c = RES / 2;
    for space in [StrokeSpace::Uv, camera(Vec3::Y)] {
        let at = if space == StrokeSpace::Uv { [0.5, 0.5] } else { [0.0, 0.0] };
        let mut light = stroke(space, &[at]);
        light.points[0].pressure = 0.0;
        let mut s = step(PaintMode::Paint, vec![light]);
        s.brush.flow_pressure = Some(0.25);
        let out = replay(&mesh, &[job_of(s.clone())]);
        let (_, cover) = mask_at(&out[0], c, c);
        assert!((cover - 0.25).abs() < 0.02, "flow at the minimum under no pressure: {cover}");
        // without pressure on flow the same stroke lays full flow
        s.brush.flow_pressure = None;
        assert!(mask_at(&replay(&mesh, &[job_of(s)])[0], c, c).1 > 0.95);
    }
}

#[test]
fn jitter_is_drawn_from_the_stroke_seed() {
    let mesh = wall();
    let painted = |space: StrokeSpace, seed: u32, jitter: bool| {
        let mut s = stroke(space.clone(), if space == StrokeSpace::Uv { &[[0.2, 0.5], [0.8, 0.5]] } else { &[[-0.3, 0.0], [0.3, 0.0]] });
        s.seed = seed;
        let mut st = step(PaintMode::Paint, vec![s]);
        st.brush.spacing = 1.0;
        if jitter {
            (st.brush.size_jitter, st.brush.flow_jitter, st.brush.position_jitter) = (0.8, 0.5, 0.3);
        }
        let out = replay(&mesh, &[job_of(st)]);
        (0..RES).flat_map(|y| row(&out[0], y)).collect::<Vec<f32>>()
    };
    for space in [StrokeSpace::Uv, camera(Vec3::Y)] {
        assert_eq!(painted(space.clone(), 5, true), painted(space.clone(), 5, true), "the same seed lays the same dabs");
        assert_ne!(painted(space.clone(), 5, true), painted(space.clone(), 6, true), "another seed other dabs");
        assert_eq!(painted(space.clone(), 5, false), painted(space, 6, false), "without jitter the seed changes nothing");
    }
}

#[test]
fn symmetry_copies_of_2d_view_strokes_land_on_the_mesh() {
    let mesh = wall();
    // a dab left of the centre in the texture: the wall's x runs with the texture's u
    let mut s = stroke(StrokeSpace::Uv, &[[0.25, 0.5]]);
    s.copies = 1;
    s.copy_matrix = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)).to_cols_array();
    let out = replay(&mesh, &[mask_job(vec![s])]);
    let c = RES / 2;
    let line = row(&out[0], c);
    assert!(line[(RES / 4) as usize] > 0.95, "the dab itself");
    assert!(line[(RES * 3 / 4) as usize] > 0.95, "its mirror on the other side");
    assert_eq!(line[c as usize], 0.0);
}

#[test]
fn a_stencil_limits_the_stroke_on_its_view() {
    let mesh = wall();
    // left half of the image white, right half black, spanning the view
    let stencil = ReplayStencil {
        image: StencilImage::Image { width: 8, height: 1, values: Arc::from(vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]) },
        matrix: [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0],
    };
    for (space, points) in [(StrokeSpace::Uv, [[0.2, 0.5], [0.8, 0.5]]), (camera(Vec3::Y), [[-0.3, 0.0], [0.3, 0.0]])] {
        let mut s = step(PaintMode::Paint, vec![stroke(space, &points)]);
        s.stencil = Some(stencil.clone());
        let out = replay(&mesh, &[job_of(s)]);
        let line = row(&out[0], RES / 2);
        assert!(line[40] > 0.95, "painted through the white half: {}", line[40]);
        assert!(line[(RES - 40) as usize] < 0.01, "held back by the black half: {}", line[(RES - 40) as usize]);
    }
    // a uniform stencil scales the whole stroke
    let mut s = step(PaintMode::Paint, vec![stroke(StrokeSpace::Uv, &[[0.2, 0.5], [0.8, 0.5]])]);
    s.stencil = Some(ReplayStencil { image: StencilImage::Uniform(0.5), matrix: [0.0; 9] });
    let (_, cover) = mask_at(&replay(&mesh, &[job_of(s)])[0], RES / 2, RES / 2);
    assert!((cover - 0.5).abs() < 0.01, "{cover}");
}

#[test]
fn a_dynamic_alpha_lays_its_stamps_in_turn() {
    let mesh = wall();
    let full = ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![1.0]) };
    let empty = ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![0.0]) };
    // dabs a diameter apart along the row: every other one lays the empty stamp
    let mut s = step(PaintMode::Paint, vec![stroke(StrokeSpace::Uv, &[[0.2, 0.5], [0.8, 0.5]])]);
    (s.brush.spacing, s.brush.radius) = (1.0, 0.05);
    s.brush.tip = full.clone();
    s.brush.cycle = vec![full, empty];
    let out = replay(&mesh, &[job_of(s)]);
    let line = row(&out[0], RES / 2);
    // dabs 6.4 texels wide every 6.4 texels from texel 25.6: the first laid, the second not, the third laid
    let at = |k: f32| line[(25.6 + 6.4 * k) as usize];
    assert!(at(0.0) > 0.95 && at(1.0) < 0.01 && at(2.0) > 0.95, "{} {} {}", at(0.0), at(1.0), at(2.0));
}

/// A job painting a band down the texture at `u` (2D view, or its place through the camera), then smudging along
/// `smudge` in `space`.
fn smudge_job(target: ReplayTarget, space: StrokeSpace, band: [[f32; 2]; 2], smudge: [[f32; 2]; 2]) -> ReplayJob {
    let mut paint = step(PaintMode::Paint, vec![stroke(space.clone(), &band)]);
    paint.brush.radius = 0.25;
    let mut smudging = step(PaintMode::Smudge, vec![stroke(space, &smudge)]);
    smudging.values = vec![(Channel::BaseColor, [0.0; 3], 1.0)];
    (smudging.brush.spacing, smudging.brush.culling) = (0.05, None);
    ReplayJob { target, label: "test".into(), steps: vec![paint, smudging] }
}

#[test]
fn a_smudge_drags_what_the_steps_before_it_laid_in_both_views() {
    let mesh = wall();
    let c = RES / 2;
    // a band down the left of the wall (texels about 10 to 42), smudged to the right along the middle row
    let views =
        [(StrokeSpace::Uv, [[0.2, 0.2], [0.2, 0.8]], [[0.2, 0.5], [0.7, 0.5]]), (camera(Vec3::Y), [[-0.3, -0.3], [-0.3, 0.3]], [[-0.3, 0.0], [0.25, 0.0]])];
    for (space, band, along) in views {
        let out = replay(&mesh, &[smudge_job(ReplayTarget::Mask(LayerId(7)), space.clone(), band, along)]);
        let mut band_only = smudge_job(ReplayTarget::Mask(LayerId(7)), space.clone(), band, along);
        band_only.steps.truncate(1);
        let before = replay(&mesh, &[band_only]);
        let (dragged, untouched) = (mask_at(&out[0], 70, c), mask_at(&out[0], 70, c - 25));
        assert!(dragged.1 > 0.3 && (dragged.0 / dragged.1 - 0.5).abs() < 0.01, "{space:?}: the band's grey dragged right: {dragged:?}");
        assert_eq!(untouched.1, 0.0, "{space:?}: nothing off the smudge's path");
        assert!(mask_at(&before[0], 70, c).1 == 0.0, "{space:?}: the band alone does not reach there");
        // smudged the other way: the empty right drawn into the band thins it
        let back = replay(&mesh, &[smudge_job(ReplayTarget::Mask(LayerId(7)), space.clone(), band, [along[1], along[0]])]);
        let x = if space == StrokeSpace::Uv { 30 } else { 32 };
        assert!(mask_at(&back[0], x, c).1 < 0.9 * mask_at(&before[0], x, c).1, "{space:?}: {:?}", mask_at(&back[0], x, c));
    }
}

#[test]
fn a_smudge_moves_content_values_with_their_coverage_and_alone_moves_nothing() {
    let mesh = wall();
    let c = RES / 2;
    let out = replay(&mesh, &[smudge_job(ReplayTarget::Content(LayerId(3)), StrokeSpace::Uv, [[0.2, 0.2], [0.2, 0.8]], [[0.2, 0.5], [0.7, 0.5]])]);
    let Ok(ReplayedPixels::Content(images)) = &out[0] else { panic!("content: {:?}", out[0]) };
    let (channel, img) = &images[0];
    assert_eq!(*channel, Channel::BaseColor);
    let px: Vec<f32> = img.get(70, c).iter().map(|v| from_u16(*v)).collect();
    assert!(px[3] > 0.3 && (px[0] / px[3] - 1.0).abs() < 0.01 && px[1] < 0.01, "red dragged right, premultiplied: {px:?}");
    // a smudge with nothing laid before it in its job leaves the target empty
    let mut alone = smudge_job(ReplayTarget::Mask(LayerId(7)), StrokeSpace::Uv, [[0.2, 0.2], [0.2, 0.8]], [[0.2, 0.5], [0.7, 0.5]]);
    alone.steps.remove(0);
    let out = replay(&mesh, &[alone]);
    let Ok(ReplayedPixels::Mask(img)) = &out[0] else { panic!("a mask: {:?}", out[0]) };
    assert_eq!(img.allocated_count(), 0);
}

#[test]
fn projection_painting_lays_its_image_grey_on_the_view() {
    let mesh = wall();
    // the image: left half white, right half black, spanning the view
    let image = ReplayStencil {
        image: StencilImage::Image { width: 8, height: 1, values: Arc::from(vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]) },
        matrix: [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0],
    };
    for (space, points) in [(StrokeSpace::Uv, [[0.2, 0.5], [0.8, 0.5]]), (camera(Vec3::Y), [[-0.3, 0.0], [0.3, 0.0]])] {
        let mut s = step(PaintMode::Paint, vec![stroke(space.clone(), &points)]);
        s.projection = Some(image.clone());
        let out = replay(&mesh, &[job_of(s)]);
        let (white, black) = (mask_at(&out[0], 40, RES / 2), mask_at(&out[0], RES - 40, RES / 2));
        assert!(white.1 > 0.95 && (white.0 - white.1).abs() < 0.01, "{space:?}: the image's white: {white:?}");
        assert!(black.1 > 0.95 && black.0 < 0.01, "{space:?}: its black, laid at full coverage: {black:?}");
    }
}

/// A path on the wall through `anchors` (mesh x, y) with the handles given between them (`out`, `in` per piece).
fn path_of(anchors: &[[f32; 2]], handles: &[[[f32; 2]; 2]]) -> ReplayPath {
    let p = |a: [f32; 2]| [a[0], a[1], 0.0];
    let pieces = anchors
        .windows(2)
        .zip(handles)
        .map(|(w, h)| PathPiece { points: [p(w[0]), p(h[0]), p(h[1]), p(w[1])], normals: [[0.0, 0.0, 1.0]; 2], size: [1.0; 2] })
        .collect();
    ReplayPath { pieces, anchor: None, seed: 0, copies: 0, copy_matrix: [0.0; 16] }
}

/// A straight path with a click's handles (a third of the way from each end).
fn line_path(from: [f32; 2], to: [f32; 2]) -> ReplayPath {
    let at = |k: f32| [from[0] + (to[0] - from[0]) * k, from[1] + (to[1] - from[1]) * k];
    path_of(&[from, to], &[[at(1.0 / 3.0), at(2.0 / 3.0)]])
}

/// The set of the wall for calling the path code directly.
fn wall_set(mesh: &Mesh) -> SetMesh {
    SetMesh::new(&ReplayScene { mesh, material: 0, resolution: RES }).unwrap()
}

/// Mesh coordinates of texel positions measured on a 2 × 2 sheet at 512.
fn from_texels(t: [f32; 2]) -> [f32; 2] {
    [t[0] / 256.0 - 1.0, 1.0 - t[1] / 256.0]
}

#[test]
#[allow(clippy::excessive_precision, reason = "the coordinates as the file stores them")]
fn path_dabs_follow_the_curve_by_its_length() {
    let mesh = wall();
    let set = wall_set(&mesh);
    let tip = Tip::of(&brush(0.05)).unwrap();
    let mut b = brush(0.05);
    b.size_pressure = Some(0.05);
    // three clicks with the handles the reference app placed; its dab centres at spacing 300 % (texels of 512, measured)
    let arch = path_of(
        &[[-0.34138179, 0.0], [0.0, 0.36623982], [0.34138179, 0.0]],
        &[[[-0.26959929, 0.15066466], [-0.16689089, 0.36623982]], [[0.16689089, 0.36623982], [0.26959929, 0.15066466]]],
    );
    b.spacing = 3.0;
    let dabs = path_dabs(&set, &b, &tip, &arch);
    let measured = [[168.62, 256.02], [207.1, 189.78], [274.32, 165.81], [325.73, 220.93]];
    assert_eq!(dabs.len(), measured.len());
    for (d, m) in dabs.iter().zip(measured) {
        let m = from_texels(m);
        assert!((d.center.x - m[0]).abs() < 0.15 / 256.0 && (d.center.y - m[1]).abs() < 0.15 / 256.0, "{} vs {m:?}", d.center);
        assert!(d.center.z.abs() < 1e-5 && (d.radius - 0.05).abs() < 1e-6);
    }
    // a straight path: steps of spacing × diameter from the first anchor, none forced onto the last
    b.spacing = 1.5;
    let xs: Vec<f32> = path_dabs(&set, &b, &tip, &line_path([-0.34138179, 0.0], [0.34138179, 0.0])).iter().map(|d| d.center.x).collect();
    assert_eq!(xs.len(), 5);
    for (x, m) in xs.iter().zip([168.62, 207.03, 245.43, 283.83, 322.23]) {
        assert!((x - from_texels([m, 0.0])[0]).abs() < 0.15 / 256.0, "{xs:?}");
    }
    // one anchor: one dab
    let one = ReplayPath { pieces: vec![], anchor: Some(([0.1, 0.2, 0.0], [0.0, 0.0, 1.0], 1.0)), seed: 0, copies: 0, copy_matrix: [0.0; 16] };
    assert_eq!(path_dabs(&set, &b, &tip, &one).len(), 1);
}

#[test]
fn replayed_dabs_reach_along_the_surface_and_live_painting_keeps_the_plane() {
    let mesh = wall();
    let set = wall_set(&mesh);
    let b = brush(0.1);
    let tip = Tip::of(&b).unwrap();
    assert_eq!(raster_ctx(&set, &b, Vec3::Z, tip.shape()).reach, Reach::Surface);
    assert_eq!(Reach::default(), Reach::Plane);
}

#[test]
fn path_dabs_lie_along_the_path_shrink_off_the_surface_and_take_symmetry() {
    let mesh = wall();
    let set = wall_set(&mesh);
    let tip = Tip::of(&brush(0.05)).unwrap();
    let b = brush(0.05);
    let dabs = path_dabs(&set, &b, &tip, &line_path([-0.5, 0.0], [0.5, 0.0]));
    // the tip lies along the path (x along it, its top up the wall) unless the brush says where its top points
    assert!(dabs[0].tangent.abs_diff_eq(Vec3::X, 1e-5) && dabs[0].bitangent.abs_diff_eq(Vec3::Y, 1e-5), "{:?}", dabs[0]);
    let mut turned = b.clone();
    turned.up = Some([1.0, 0.0, 0.0]);
    let d = path_dabs(&set, &turned, &tip, &line_path([-0.5, 0.0], [0.5, 0.0]));
    assert!(d[0].bitangent.abs_diff_eq(Vec3::X, 1e-5), "its top towards the up direction: {:?}", d[0]);
    let mut follows = turned.clone();
    follows.follow = true;
    let d = path_dabs(&set, &follows, &tip, &line_path([-0.5, 0.0], [0.5, 0.0]));
    assert!(d[0].bitangent.abs_diff_eq(Vec3::X, 1e-5), "following the path, its top ahead: {:?}", d[0]);
    // a path 0.07 above the wall: dabs r·√(1 − (0.07 / 1.75r)²) = 0.6·r; 0.1 above: none
    let lift = |path: ReplayPath, z: f32| ReplayPath {
        pieces: path.pieces.iter().map(|p| PathPiece { points: p.points.map(|q| [q[0], q[1], z]), ..*p }).collect(),
        ..path
    };
    let high = path_dabs(&set, &b, &tip, &lift(line_path([-0.5, 0.0], [0.5, 0.0]), 0.07));
    assert!(!high.is_empty() && high.iter().all(|d| (d.radius - 0.03).abs() < 1e-4 && d.center.z.abs() < 1e-5), "{:?}", high[0]);
    assert!(path_dabs(&set, &b, &tip, &lift(line_path([-0.5, 0.0], [0.5, 0.0]), 0.1)).is_empty());
    // symmetry: the recorded matrix applied to every dab
    let mut mirrored = line_path([0.2, 0.3], [0.6, 0.3]);
    mirrored.copies = 1;
    mirrored.copy_matrix = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)).to_cols_array();
    let d = path_dabs(&set, &b, &tip, &mirrored);
    let n = d.len() / 2;
    assert!(n > 0 && d.len() == 2 * n);
    assert!(d[n].center.abs_diff_eq(Vec3::new(-d[0].center.x, d[0].center.y, 0.0), 1e-5));
}

#[test]
fn path_strokes_and_ribbons_paint_into_their_target() {
    let mesh = wall();
    let c = RES / 2;
    // a brush along a path, after the step's strokes
    let mut s = step(PaintMode::Paint, vec![]);
    s.brush.radius = 0.05;
    s.paths = vec![line_path([-0.5, 0.0], [0.5, 0.0])];
    let out = replay(&mesh, &[job_of(s.clone())]);
    let (grey, cover) = mask_at(&out[0], c, c);
    assert!(cover > 0.95 && (grey - 0.5 * cover).abs() < 0.01, "{grey} {cover}");
    assert!(mask_at(&out[0], c - 30, c).1 > 0.9 && mask_at(&out[0], c, c - 6).1 == 0.0);
    // a ribbon: a band 0.2 wide (6.4 texels either side), a square tip repeated in copies with gaps between them
    s.ribbon = Some(ReplayRibbon { width: 0.2, copy: 0.2, tiling: Tiling::Repeat, depth: 0.1 });
    s.brush.tip = ReplayTip::Image { width: 4, height: 1, values: Arc::from(vec![0.0, 1.0, 1.0, 0.0]) };
    let out = replay(&mesh, &[job_of(s.clone())]);
    let along = row(&out[0], c);
    assert!(along.iter().filter(|v| **v > 0.9).count() > 12, "{along:?}");
    // copies of 0.2 along a path 1 long: five, each with a gap at both ends (texels 32..96)
    let gaps = (33..95).filter(|x| along[*x as usize] < 0.1).count();
    assert!(gaps >= 8, "{along:?}");
    assert!(mask_at(&out[0], c, c + 5).1 > 0.5 || mask_at(&out[0], c + 2, c + 5).1 > 0.5, "the band's width");
    assert_eq!(mask_at(&out[0], c, c + 8).1, 0.0);
    // content targets take the band too
    let content = ReplayJob { target: ReplayTarget::Content(LayerId(3)), label: String::new(), steps: vec![s] };
    let Ok(ReplayedPixels::Content(images)) = &replay(&mesh, &[content])[0] else { panic!("content") };
    assert!(images.iter().any(|(ch, img)| *ch == Channel::BaseColor && from_u16(img.get(c + 1, c)[3]) > 0.5));
}

/// An image source 16 × 16 (grey, linear): `value(x, y)` per pixel.
fn image_source(value: impl Fn(u32, u32) -> f32) -> ReplaySource {
    let rgba = (0..256u32).map(|i| [value(i % 16, i / 16); 3]).map(|v| [v[0], v[1], v[2], 1.0]).collect();
    let image = texturecraft_compose::ImageRes { width: 16, height: 16, rgba, srgb: false };
    ReplaySource::Texels(Arc::new(ReplayTexels::Image(Arc::new(image))))
}

/// Dark on the image's left half, bright on its right.
fn halves(x: u32, _y: u32) -> f32 {
    if x < 8 { 0.2 } else { 0.8 }
}

/// A content job painting roughness from `source` (and nothing else).
fn source_job(mut s: ReplayStep, source: ReplaySource) -> ReplayJob {
    s.values.clear();
    s.sources = vec![(Channel::Roughness, source, 1.0)];
    ReplayJob { target: ReplayTarget::Content(LayerId(3)), label: "test".into(), steps: vec![s] }
}

/// Roughness (straight value, coverage) at texel `(x, y)`.
fn roughness_at(r: &Result<ReplayedPixels, String>, x: u32, y: u32) -> (f32, f32) {
    let Ok(ReplayedPixels::Content(images)) = r else { panic!("content: {r:?}") };
    let Some((_, img)) = images.iter().find(|(c, _)| *c == Channel::Roughness) else { return (0.0, 0.0) };
    let t = img.get(x, y);
    let cover = from_u16(t[1]);
    (if cover > 0.0 { from_u16(t[0]) / cover } else { 0.0 }, cover)
}

#[test]
fn a_brush_painting_an_image_lays_it_texel_for_texel() {
    let mesh = wall();
    let c = RES / 2;
    // the image: dark on the texture's left half, bright on its right; a stroke across the middle
    let job = source_job(step(PaintMode::Paint, vec![stroke(StrokeSpace::Uv, &[[0.2, 0.5], [0.8, 0.5]])]), image_source(halves));
    let out = replay(&mesh, &[job]);
    let (left, right) = (roughness_at(&out[0], 40, c), roughness_at(&out[0], 90, c));
    assert!(left.1 > 0.95 && (left.0 - 0.2).abs() < 0.01, "{left:?}");
    assert!(right.1 > 0.95 && (right.0 - 0.8).abs() < 0.01, "{right:?}");
    assert_eq!(roughness_at(&out[0], c, c - 20).1, 0.0, "only where the stroke reaches");
    // an image's alpha scales the coverage
    let mut half = image_source(|_, _| 0.5);
    if let ReplaySource::Texels(t) = &mut half
        && let ReplayTexels::Image(i) = Arc::make_mut(t)
    {
        Arc::make_mut(i).rgba.iter_mut().for_each(|p| p[3] = 0.5);
    }
    let out = replay(&mesh, &[source_job(step(PaintMode::Paint, vec![stroke(StrokeSpace::Uv, &[[0.2, 0.5], [0.8, 0.5]])]), half)]);
    assert!((roughness_at(&out[0], 40, c).1 - 0.5).abs() < 0.02);
}

#[test]
fn a_graph_source_paints_its_render_and_one_that_failed_fails_the_job() {
    let mesh = wall();
    let c = RES / 2;
    // a render of the texture's size: 0.3 above the middle row, 0.7 below
    let mut out_img = TileImage::new(RES, RES, 4);
    for y in 0..RES {
        for x in 0..RES {
            let v = if y < c { 0.3 } else { 0.7 };
            out_img.set(x, y, &[texturecraft_texel::to_u16(v), 0, 0, 65535]);
        }
    }
    let cooked = Arc::new(texturecraft_compose::Cooked::new(BTreeMap::from([("o".to_string(), out_img)]), Default::default()));
    let graph = ReplaySource::Texels(Arc::new(ReplayTexels::Graph { cooked, output: Some("o".into()), alpha: None }));
    let up_down = stroke(StrokeSpace::Uv, &[[0.5, 0.2], [0.5, 0.8]]);
    let out = replay(&mesh, &[source_job(step(PaintMode::Paint, vec![up_down.clone()]), graph)]);
    assert!((roughness_at(&out[0], c, 40).0 - 0.3).abs() < 0.01 && (roughness_at(&out[0], c, 90).0 - 0.7).abs() < 0.01);
    let failed = source_job(step(PaintMode::Paint, vec![up_down]), ReplaySource::Failed("did not render".into()));
    assert_eq!(replay(&mesh, &[failed])[0].as_ref().err().map(String::as_str), Some("did not render"));
}

#[test]
fn a_source_along_a_ribbon_is_read_at_each_texels_place_on_the_band() {
    let mesh = wall();
    let c = RES / 2;
    let mut s = step(PaintMode::Paint, vec![]);
    s.paths = vec![line_path([-0.5, 0.0], [0.5, 0.0])];
    s.ribbon = Some(ReplayRibbon { width: 0.2, copy: 0.2, tiling: Tiling::Repeat, depth: 0.1 });
    s.brush.tip = ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![1.0]) };
    // dark on each copy's first half, bright on its second: five copies along the path switch nine times
    let out = replay(&mesh, &[source_job(s.clone(), image_source(halves))]);
    let along: Vec<f32> = (33..95).map(|x| roughness_at(&out[0], x, c).0).collect();
    let switches = along.windows(2).filter(|w| (w[0] - 0.5).signum() != (w[1] - 0.5).signum()).count();
    assert!((8..=10).contains(&switches), "{along:?}");
    // across the band: the image's top row on the path's left (up the wall), its bottom row on its right
    let out = replay(&mesh, &[source_job(s, image_source(|_, y| if y < 8 { 0.2 } else { 0.8 }))]);
    assert!(roughness_at(&out[0], c, c - 4).0 < 0.4 && roughness_at(&out[0], c, c + 4).0 > 0.6);
}

#[test]
fn a_source_along_path_dabs_is_read_in_the_last_dabs_square() {
    let mesh = wall();
    let c = RES / 2;
    let mut s = step(PaintMode::Paint, vec![]);
    s.brush.radius = 0.1;
    s.brush.spacing = 3.0;
    s.brush.tip = ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![1.0]) };
    // dabs 0.6 apart from x = −0.6: at texels 25.6, 64 and 102.4, 6.4 texels a side; each reads the image over its square
    s.paths = vec![line_path([-0.6, 0.0], [0.7, 0.0])];
    let out = replay(&mesh, &[source_job(s.clone(), image_source(halves))]);
    for centre in [c - 38, c, c + 38] {
        let (l, r) = (roughness_at(&out[0], centre - 4, c), roughness_at(&out[0], centre + 3, c));
        assert!(l.1 > 0.9 && (l.0 - 0.2).abs() < 0.05 && (r.0 - 0.8).abs() < 0.05, "{centre}: {l:?} {r:?}");
    }
    // a mask painted with a graph along the path reads its grey the same way
    let mut m = s;
    m.mask_source = Some(image_source(halves));
    let out = replay(&mesh, &[job_of(m)]);
    let (grey, cover) = mask_at(&out[0], c + 3, c);
    assert!(cover > 0.9 && (grey / cover - 0.8).abs() < 0.05, "{grey} {cover}");
}

#[test]
fn projection_painting_on_content_lays_each_channels_source_on_the_view() {
    let mesh = wall();
    // the image spans the view, its left half dark: in the 2D view the texture's left, in the 3D view the wall's left
    for (space, points) in [(StrokeSpace::Uv, [[0.2, 0.5], [0.8, 0.5]]), (camera(Vec3::Y), [[-0.3, 0.0], [0.3, 0.0]])] {
        let mut s = step(PaintMode::Paint, vec![stroke(space.clone(), &points)]);
        s.placed = Some([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0]);
        let out = replay(&mesh, &[source_job(s.clone(), image_source(halves))]);
        let (dark, bright) = (roughness_at(&out[0], 40, RES / 2), roughness_at(&out[0], RES - 40, RES / 2));
        assert!(dark.1 > 0.95 && (dark.0 - 0.2).abs() < 0.02, "{space:?}: {dark:?}");
        assert!(bright.1 > 0.95 && (bright.0 - 0.8).abs() < 0.02, "{space:?}: {bright:?}");
        // an image placed on the view's right half only: nothing on its left
        s.placed = Some([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.5, 1.0]);
        let out = replay(&mesh, &[source_job(s, image_source(halves))]);
        assert_eq!(roughness_at(&out[0], 40, RES / 2).1, 0.0, "{space:?}");
    }
}
