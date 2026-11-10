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
    assert_eq!(key(&effect(PaintTool::Smudge)), "brush:smudge");
    assert_eq!(key(&effect(PaintTool::Projection)), "brush:projection");
    let mut e = effect(PaintTool::Brush);
    e.brush = Some(ProjectBrush { emitter: StampEmitter::Physics(Default::default()), ..ProjectBrush::default() });
    assert_eq!(key(&e), "brush:particle");
    let mut e = effect(PaintTool::Brush);
    e.stencil = Some(uniform(0, [1.0; 3], 1.0));
    assert_eq!(key(&e), "brush:stencil");
    let mut e = effect(PaintTool::Brush);
    e.paths.push(texturecraft_import_reader::model::StrokePath3d::line([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
    assert_eq!(key(&e), "brush:3d-path");
    let mut e = effect(PaintTool::Brush);
    e.strokes[0].complete = false;
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
