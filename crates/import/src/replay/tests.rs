//! Replay steps from synthetic paint effects.

use std::sync::Arc;

use texturecraft_core::{Channel, ChannelFormat};
use texturecraft_import_reader::blockers::BlockerKind;
use texturecraft_import_reader::model::{
    ChannelMask, EffectHeader, PaintEffect, PaintTool, PressureResponse, Procedural, ProceduralSource, ProjectBrush, ProjectSource, ProjectStroke,
    StampEmitter, StrokeEmitter, StrokeKey, StrokeView, TimelineKey, Tweak, TweakValue, UniformSource,
};

use super::brush::{StepTarget, step};
use super::tip::old_round_at;
use super::*;
use crate::channels::SetChannels;
use crate::resources::ProjectImages;

fn channels() -> SetChannels {
    let mut c = SetChannels::default();
    c.declare(Channel::BaseColor, ChannelFormat::Srgb8);
    c.declare(Channel::Roughness, ChannelFormat::L8);
    c
}

fn uniform(channel: u64, v: [f32; 3], opacity: f32) -> ProjectSource {
    ProjectSource::Uniform(UniformSource { uid: None, channels: ChannelMask(channel), color: v, opacity })
}

fn shape(name: &str, tweaks: Vec<Tweak>) -> ProjectSource {
    let p = Procedural {
        uid: None,
        url: format!("resource://shelf/{name}/{name}?version=1"),
        tweaks,
        outputs: vec![],
        inputs: vec![],
        alpha_output: String::new(),
        mono_output: 0,
    };
    ProjectSource::Procedural(ProceduralSource { uid: None, channels: ChannelMask(0), mode: 0, stamp_cycle: 0, stroke_wires: 0, procedural: Some(p) })
}

fn hardness(h: f32) -> Vec<Tweak> {
    vec![Tweak { identifier: "hardness".into(), value: TweakValue::Floats(vec![h]) }]
}

fn stroke(view: StrokeView, points: &[[f32; 2]]) -> ProjectStroke {
    ProjectStroke {
        uid: None,
        points: points.iter().enumerate().map(|(i, p)| StrokeKey { position: *p, time: i as f32 }).collect(),
        pressure: vec![TimelineKey { time: 0.0, value: 0.2 }, TimelineKey { time: 2.0, value: 1.0 }],
        tilt_keys: 0,
        complete: true,
        view_matrix: [1.0; 16],
        projection_matrix: [2.0; 16],
        view,
        duplicate_count: 1,
        duplicate_matrix: [3.0; 16],
        random_seed: 42,
        lazy_distance: 0.08,
    }
}

fn effect(tool: PaintTool) -> PaintEffect {
    PaintEffect {
        header: EffectHeader { uid: Some(1), enabled: true, label: String::new(), channels: ChannelMask(0), blending: vec![], unhandled: vec![] },
        tool,
        opacity: 0.5,
        strokes: vec![stroke(StrokeView::Mesh, &[[0.5, 0.5], [0.75, 1.0], [1.0, 1.0]]), stroke(StrokeView::Uv, &[[0.25, 0.25]])],
        paths: vec![],
        brush_uid: None,
        brush: Some(ProjectBrush {
            size: 0.2,
            flow: 0.8,
            size_jitter: 0.5,
            emitter: StampEmitter::Stroke(StrokeEmitter {
                spacing: 0.15,
                rotation: 1.0,
                pressure_size: PressureResponse { enabled: true, minimum: 0.1 },
                ..StrokeEmitter::default()
            }),
            ..ProjectBrush::default()
        }),
        stamp_blending: 0,
        colors: vec![uniform(1, [1.0, 0.5, 0.25], 0.5), uniform(128, [0.3; 3], 1.0)],
        alpha: vec![shape("shape_v2", hardness(0.7))],
        stencil: None,
        stencil_matrix: None,
    }
}

fn of(e: &PaintEffect, target: StepTarget) -> Result<ReplayStep, texturecraft_import_reader::blockers::Blocker> {
    step(e, target, &channels(), &ProjectImages::none(), None)
}

#[test]
fn the_brush_maps_with_the_measured_rules() {
    let s = of(&effect(PaintTool::Brush), StepTarget::Content).unwrap();
    let b = &s.brush;
    assert!((b.radius - 0.1).abs() < 1e-6, "the size is a diameter in mesh units");
    assert_eq!((b.spacing, b.flow, b.size_jitter, b.rotation, b.size_pressure, b.flow_pressure), (0.15, 0.8, 0.5, 1.0, Some(0.1), None));
    assert_eq!((b.alignment, b.culling), (TipAlignment::Tangent, Some(0.5)));
    assert_eq!(b.tip, ReplayTip::Round { hardness: 0.7 });
    assert_eq!((s.mode, s.opacity), (PaintMode::Paint, 0.5));
    assert_eq!(s.values, vec![(Channel::BaseColor, [1.0, 0.5, 0.25], 0.5), (Channel::Roughness, [0.3; 3], 1.0)]);
    // 3D-view points: 0..1 with y up → normalised device coordinates; 2D-view points: V up → rows down
    let [mesh, uv] = &s.strokes[..] else { panic!("two strokes") };
    assert_eq!(mesh.points.iter().map(|p| p.position).collect::<Vec<_>>(), [[0.0, 0.0], [0.5, 1.0], [1.0, 1.0]]);
    assert!(matches!(mesh.space, StrokeSpace::Mesh { view, projection } if view == [1.0; 16] && projection == [2.0; 16]));
    assert!((mesh.points[1].pressure - 0.6).abs() < 1e-6, "pressure between the keys");
    assert_eq!((mesh.seed, mesh.copies, mesh.copy_matrix, mesh.lazy), (42, 1, [3.0; 16], 0.08));
    assert_eq!(uv.space, StrokeSpace::Uv);
    assert_eq!(uv.points[0].position, [0.25, 0.75]);
}

#[test]
fn masks_paint_a_grey_and_erasers_lower_coverage_or_paint_black() {
    let mut e = effect(PaintTool::Brush);
    e.colors = vec![uniform(0, [0.8; 3], 0.5)];
    assert_eq!(of(&e, StepTarget::Mask).unwrap().mask_value, 0.4);
    e.colors.clear();
    assert_eq!(of(&e, StepTarget::Mask).unwrap().mask_value, 1.0);
    let eraser = effect(PaintTool::Eraser);
    let s = of(&eraser, StepTarget::Mask).unwrap();
    assert_eq!((s.mode, s.mask_value), (PaintMode::Paint, 0.0), "an eraser in a mask paints black (measured)");
    let s = of(&eraser, StepTarget::Content).unwrap();
    assert_eq!(s.mode, PaintMode::Erase);
    assert_eq!(s.values.iter().map(|v| v.0).collect::<Vec<_>>(), [Channel::BaseColor, Channel::Roughness]);
}

#[test]
fn what_the_brush_cannot_replay_is_refused_with_a_reason() {
    let key = |e: &PaintEffect| of(e, StepTarget::Content).unwrap_err().key.to_string();
    // projection painting: not on a paint layer, never without an image; neither it nor a smudge along 3D paths
    assert_eq!(key(&effect(PaintTool::Projection)), "brush:projection");
    let mut e = effect(PaintTool::Projection);
    e.colors.clear();
    assert_eq!(of(&e, StepTarget::Mask).unwrap_err().key.to_string(), "brush:projection");
    for tool in [PaintTool::Projection, PaintTool::Smudge] {
        let mut e = effect(tool);
        e.paths.push(texturecraft_import_reader::model::StrokePath3d::line([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
        let b = of(&e, StepTarget::Mask).unwrap_err();
        assert_eq!(b.key.to_string(), "brush:3d-path");
        assert!(b.text.contains(if tool == PaintTool::Smudge { "smudge" } else { "projection" }), "{}", b.text);
    }
    assert_eq!(brush::alone(&effect(PaintTool::Smudge)).unwrap_err().key.to_string(), "brush:smudge-alone");
    assert!(brush::alone(&effect(PaintTool::Brush)).is_ok());
    let mut e = effect(PaintTool::Brush);
    e.brush = Some(ProjectBrush { emitter: StampEmitter::Physics(Default::default()), ..ProjectBrush::default() });
    assert_eq!(key(&e), "brush:particle");
    // a stencil drawn by a graph the project does not hold: its own blocker
    let mut e = effect(PaintTool::Brush);
    e.stencil = Some(shape("grunge_map", vec![]));
    let b = of(&e, StepTarget::Content).unwrap_err();
    assert_eq!((b.key.kind, b.key.name.as_str()), (BlockerKind::BrushAlpha, "grunge_map"));
    let mut e = effect(PaintTool::Brush);
    e.strokes[0].complete = false;
    assert_eq!(key(&e), "brush:unreadable-points");
    let mut e = effect(PaintTool::Brush);
    let mut path = texturecraft_import_reader::model::StrokePath3d::line([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    path.complete = false;
    e.paths.push(path);
    assert_eq!(key(&e), "brush:unreadable-points");
    let mut e = effect(PaintTool::Brush);
    e.colors = match shape("wood", vec![]) {
        ProjectSource::Procedural(mut p) => {
            p.channels = ChannelMask(1);
            vec![ProjectSource::Procedural(p)]
        }
        other => vec![other],
    };
    assert_eq!(key(&e), "brush:paints-procedural");
    // a tip that is a compiled graph: its own blocker, until graphs are computed
    let mut e = effect(PaintTool::Brush);
    e.alpha = vec![shape("shape_border_square", hardness(0.5))];
    let b = of(&e, StepTarget::Content).unwrap_err();
    assert_eq!((b.key.kind, b.key.name.as_str()), (BlockerKind::BrushAlpha, "shape_border_square"));
}

#[test]
fn tips_follow_the_measured_shapes() {
    let tip = |alpha: Vec<ProjectSource>| {
        let mut e = effect(PaintTool::Brush);
        e.alpha = alpha;
        of(&e, StepTarget::Content).unwrap().brush.tip
    };
    // the older round shape: full to 0.2 of the radius at hardness 0, then a smoothstep; the hardness scales it up
    assert_eq!(old_round_at(0.1, 0.0), 1.0);
    assert!((old_round_at(0.6, 0.0) - 0.5).abs() < 1e-6);
    assert!((old_round_at(0.6, 0.5) - 1.0).abs() < 1e-6);
    assert_eq!(old_round_at(1.0, 0.9), 0.0);
    let ReplayTip::Image { width, height, values } = tip(vec![shape("shape", hardness(0.0))]) else { panic!("an image") };
    assert_eq!((width, height), (tip::TIP_SIDE, tip::TIP_SIDE));
    let n = width as usize;
    assert!(values[n / 2 * n + n / 2] > 0.99 && values[0] == 0.0);
    // no alpha: the older round shape, soft; a uniform alpha: a full square of its value
    assert!(matches!(tip(vec![]), ReplayTip::Image { .. }));
    assert_eq!(tip(vec![uniform(0, [0.5; 3], 1.0)]), ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![0.5]) });
    // a round shape with another setting changed is no longer round: drawn by its graph
    let mut tweaks = hardness(0.2);
    tweaks.push(Tweak { identifier: "size_ratio".into(), value: TweakValue::Floats(vec![0.5]) });
    let mut e = effect(PaintTool::Brush);
    e.alpha = vec![shape("shape_v2", tweaks)];
    assert_eq!(of(&e, StepTarget::Content).unwrap_err().key.kind, BlockerKind::BrushAlpha);
}

#[test]
fn hostile_values_are_bounded() {
    let mut e = effect(PaintTool::Brush);
    e.opacity = f32::NAN;
    e.brush = Some(ProjectBrush { size: f32::INFINITY, flow: -3.0, ..ProjectBrush::default() });
    e.strokes[0].points.push(StrokeKey { position: [f32::NAN, 0.0], time: f32::NAN });
    e.strokes[0].duplicate_count = u32::MAX;
    e.strokes.push(stroke(StrokeView::Other(9), &[]));
    let s = of(&e, StepTarget::Content).unwrap();
    assert_eq!((s.opacity, s.brush.flow), (1.0, 0.0));
    assert!(s.brush.radius.is_finite());
    assert_eq!(s.strokes.len(), 2, "a stroke without points is left out");
    assert_eq!(s.strokes[0].points.len(), 3, "a point that is not a number is left out");
    assert_eq!(s.strokes[0].copies, brush::MAX_COPIES);
    assert_eq!(pressure_at(&[TimelineKey { time: 0.0, value: f32::NAN }], 1.0), 1.0);
}

#[test]
fn strokes_go_through_their_stencil_on_the_view() {
    let mut e = effect(PaintTool::Brush);
    e.stencil = Some(uniform(0, [0.8; 3], 0.5));
    let s = of(&e, StepTarget::Content).unwrap();
    // without a matrix the stencil spans the view
    assert_eq!(s.stencil, Some(ReplayStencil { image: StencilImage::Uniform(0.4), matrix: [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0] }));
    e.stencil_matrix = Some([2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, 0.0, 1.0]);
    assert_eq!(of(&e, StepTarget::Content).unwrap().stencil.map(|st| st.matrix), Some([2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, 0.0, 1.0]));
    assert!(of(&effect(PaintTool::Brush), StepTarget::Content).unwrap().stencil.is_none());
}

#[test]
fn pressure_jitter_and_symmetry_are_replayed_without_notes() {
    let mut e = effect(PaintTool::Brush);
    if let Some(b) = &mut e.brush {
        (b.flow_jitter, b.size_jitter) = (0.3, 0.2);
        if let StampEmitter::Stroke(em) = &mut b.emitter {
            em.pressure_flow = PressureResponse { enabled: true, minimum: 0.3 };
            (em.position_jitter, em.position_jitter_both_axes, em.position_jitter_distribution) = (0.5, false, 0);
        }
    }
    let s = of(&e, StepTarget::Content).unwrap();
    let b = &s.brush;
    assert_eq!((b.flow_pressure, b.position_jitter, b.position_both, b.position_normal), (Some(0.3), 0.5, false, false));
    // the 2D-view stroke has a symmetry copy too
    assert_eq!(s.strokes[1].copies, 1);
    assert!(s.notes.is_empty(), "{:?}", s.notes);
}

/// A graph alpha with `stampIndex`, cycled through `count` stamps.
fn dynamic(count: u32, wires: u32) -> ProjectSource {
    let ProjectSource::Procedural(mut p) = shape("ink_dynamic", vec![Tweak { identifier: "stampIndex".into(), value: TweakValue::Int(0) }]) else {
        panic!("a procedural")
    };
    (p.stamp_cycle, p.stroke_wires) = (count, wires);
    ProjectSource::Procedural(p)
}

#[test]
fn a_dynamic_alpha_cycles_through_its_stamps() {
    use crate::graph_library::GraphLibrary;
    use texturecraft_import_reader::decompile::Decompiled;
    let mut lib = GraphLibrary::default();
    let d = Decompiled {
        text: "format 3\nnode 1 color r=0.5 g=0.5 b=0.5\noutput grey 1\n".into(),
        images: vec![],
        image_defaults: vec![],
        outputs: vec![("grey".into(), 1)],
        warnings: vec![],
        colour_outputs: vec![],
    };
    lib.insert_as(&["ink_dynamic".to_string()], "", d);
    let with = |alpha: ProjectSource| {
        let mut e = effect(PaintTool::Brush);
        e.alpha = vec![alpha];
        step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), Some(&lib)).unwrap()
    };
    let s = with(dynamic(3, texturecraft_import_reader::model::STROKE_WIRE_RANDOM_START));
    assert_eq!((s.brush.cycle.len(), s.brush.cycle_drawn_start), (3, true));
    assert!(s.notes.is_empty(), "{:?}", s.notes);
    let s = with(dynamic(5, 0));
    assert_eq!((s.brush.cycle.len(), s.brush.cycle_drawn_start), (5, false));
    // more stamps than the guard: the guard's count
    assert_eq!(with(dynamic(u32::MAX, 0)).brush.cycle.len(), tip::MAX_CYCLE as usize);
    // a graph alpha that is not dynamic lays one stamp
    let ProjectSource::Procedural(mut plain) = dynamic(3, 0) else { panic!("a procedural") };
    if let Some(q) = &mut plain.procedural {
        q.tweaks.clear();
    }
    assert!(with(ProjectSource::Procedural(plain)).brush.cycle.is_empty());
}

#[test]
fn a_smudge_moves_what_is_there_on_the_channels_it_works_on() {
    let mut e = effect(PaintTool::Smudge);
    e.stencil = Some(uniform(0, [0.8; 3], 1.0));
    let s = of(&e, StepTarget::Content).unwrap();
    assert_eq!(s.mode, PaintMode::Smudge);
    assert_eq!(s.values, vec![(Channel::BaseColor, [0.0; 3], 1.0), (Channel::Roughness, [0.0; 3], 1.0)], "every channel it works on, no value");
    assert_eq!(s.opacity, 0.5);
    assert!(s.stencil.is_none() && s.projection.is_none());
    assert!(s.notes.iter().any(|n| n.contains("stencil")), "{:?}", s.notes);
    assert!(s.notes.iter().any(|n| n.contains("symmetry copies")), "{:?}", s.notes);
    let s = of(&e, StepTarget::Mask).unwrap();
    assert_eq!((s.mode, s.values.len()), (PaintMode::Smudge, 0));
}

#[test]
fn projection_painting_in_a_mask_places_its_image_on_the_view() {
    let mut e = effect(PaintTool::Projection);
    e.colors = vec![uniform(0, [0.6; 3], 0.5)];
    let s = of(&e, StepTarget::Mask).unwrap();
    assert_eq!(s.mode, PaintMode::Paint);
    assert_eq!(s.projection, Some(ReplayStencil { image: StencilImage::Uniform(0.3), matrix: [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 1.0] }));
    assert!(s.stencil.is_none());
    // the image and a stencil share `projMatrix`
    e.stencil = Some(uniform(0, [1.0; 3], 1.0));
    e.stencil_matrix = Some([2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, 0.0, 1.0]);
    let s = of(&e, StepTarget::Mask).unwrap();
    assert_eq!(s.projection.map(|p| p.matrix), s.stencil.map(|p| p.matrix));
    // a brush has no projection
    assert!(of(&effect(PaintTool::Brush), StepTarget::Mask).unwrap().projection.is_none());
}

fn ribbon_brush(tiling: u64) -> ProjectBrush {
    ProjectBrush {
        size: 0.3,
        emitter: StampEmitter::Ribbon(texturecraft_import_reader::model::RibbonBrush {
            width: 0.1,
            tiling_mode: tiling,
            stretch_mode: 1,
            stretch_guides: [0.25, 0.75],
            aspect_ratio: 2.0,
            count: 4,
            source_direction: 0,
            alignment: 0,
            corner_shape: 0,
            flip_along_width: false,
            repeat_mirror: false,
            omit_ends_when_closed: false,
            alpha_blending: 7,
            depth_hardness: 1.0,
            projection_depth_ratio: 3.0,
        }),
        ..ProjectBrush::default()
    }
}

#[test]
fn path_strokes_and_ribbons_become_steps() {
    use texturecraft_import_reader::model::StrokePath3d;
    // a brush along a path: its pieces as stored, the anchors' size as pressure, the seed and symmetry kept
    let mut e = effect(PaintTool::Brush);
    let mut path = StrokePath3d::line([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    path.random_seed = 7;
    path.duplicate_count = 500;
    if let Some(k) = path.points.last_mut() {
        k.size = 0.5;
    }
    e.paths.push(path);
    if let Some(ProjectBrush { emitter: StampEmitter::Stroke(em), .. }) = &mut e.brush {
        em.up_vector = Some([0.0, 0.0, 1.0]);
    }
    let s = of(&e, StepTarget::Content).unwrap();
    assert_eq!((s.strokes.len(), s.paths.len(), s.ribbon), (2, 1, None));
    let p = &s.paths[0];
    assert_eq!(p.pieces.len(), 1);
    assert_eq!(p.pieces[0].points[3], [1.0, 0.0, 0.0]);
    assert_eq!(p.pieces[0].size, [1.0, 0.5]);
    assert_eq!((p.anchor, p.seed, p.copies), (None, 7, brush::MAX_COPIES));
    assert_eq!(s.brush.up, Some([0.0, 0.0, 1.0]));
    assert!(s.notes.iter().any(|n| n.starts_with(brush::PATHS_APPROXIMATE)), "{:?}", s.notes);
    // one anchor: a path of no pieces at that anchor; one with no finite anchor is dropped
    let mut one = StrokePath3d::line([0.5; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    one.points.truncate(1);
    assert_eq!(brush::path(&one).unwrap().anchor, Some(([0.5; 3], [0.0, 1.0, 0.0], 1.0)));
    one.points[0].position[0] = f32::NAN;
    assert_eq!(brush::path(&one), None);
    // a ribbon brush: its band, its brush strokes left out
    for (mode, tiling) in [(0, RibbonTiling::Repeat), (1, RibbonTiling::Whole), (2, RibbonTiling::Caps { guides: [0.25, 0.75] })] {
        let mut e = effect(PaintTool::Brush);
        e.brush = Some(ribbon_brush(mode));
        e.paths.push(StrokePath3d::line([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
        let s = of(&e, StepTarget::Mask).unwrap();
        let r = s.ribbon.unwrap();
        assert_eq!(r.tiling, tiling);
        assert!((r.width - 0.1).abs() < 1e-6 && (r.copy - 0.2).abs() < 1e-6 && (r.depth - 0.15).abs() < 1e-6, "{r:?}");
        assert!(s.strokes.is_empty() && s.paths.len() == 1);
        assert!(s.notes.iter().any(|n| n.starts_with(brush::RIBBON_APPROXIMATE)) && s.notes.iter().any(|n| n.contains("left out")), "{:?}", s.notes);
    }
    // a band reaches at least half its width along the normal
    let mut e = effect(PaintTool::Brush);
    let mut b = ribbon_brush(0);
    if let StampEmitter::Ribbon(r) = &mut b.emitter {
        r.projection_depth_ratio = 0.1;
    }
    e.brush = Some(b);
    assert!((of(&e, StepTarget::Content).unwrap().ribbon.unwrap().depth - 0.05).abs() < 1e-6);
}
