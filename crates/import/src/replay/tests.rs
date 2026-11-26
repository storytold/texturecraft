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
    step(e, target, &channels(), &ProjectImages::none(), None, None)
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
    // projection painting: never without an image; neither it nor a smudge along 3D paths
    let mut e = effect(PaintTool::Projection);
    e.colors.clear();
    assert_eq!(key(&e), "brush:projection");
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
        step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), Some(&lib), None).unwrap()
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

#[test]
fn projection_painting_on_content_places_each_channels_source_on_the_view() {
    let mut e = effect(PaintTool::Projection);
    e.stencil_matrix = Some([2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, 0.0, 1.0]);
    let s = of(&e, StepTarget::Content).unwrap();
    // the channels' uniforms as values, placed by `projMatrix`; no grey image of its own
    assert_eq!(s.values.len(), 2);
    assert_eq!((s.placed, s.projection.is_none()), (Some([2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, 0.0, 1.0]), true));
    assert!(s.notes.iter().any(|n| n == brush::PROJECTED));
    e.colors = vec![material("")];
    let s = step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), None, Some(&Graphs)).unwrap();
    assert!(matches!(&s.sources[..], [(Channel::BaseColor, ReplaySource::Graph(_), _)]) && s.placed.is_some());
    assert!(of(&effect(PaintTool::Brush), StepTarget::Content).unwrap().placed.is_none());
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

/// The graphs a fill of a source runs, made up: `material` paints base colour and its grey output, nothing else.
struct Graphs;

impl Graphs {
    fn of(source: &ProceduralSource) -> Result<texturecraft_core::GraphRef, texturecraft_import_reader::blockers::Blocker> {
        let name = source.procedural.as_ref().map_or("?", |p| p.name());
        match name {
            "material" => Ok(texturecraft_core::GraphRef::new("k", name)),
            _ => Err(texturecraft_import_reader::blockers::Blocker::graph(name, "not in the project")),
        }
    }
}

impl SourceGraphs for Graphs {
    fn content(
        &self,
        source: &ProceduralSource,
        channels: &[Channel],
    ) -> Result<Vec<(Channel, texturecraft_core::GraphRef)>, texturecraft_import_reader::blockers::Blocker> {
        let g = Graphs::of(source)?;
        Ok(channels.iter().filter(|c| **c == Channel::BaseColor).map(|c| (*c, g.clone())).collect())
    }

    fn mask(&self, source: &ProceduralSource) -> Result<texturecraft_core::GraphRef, texturecraft_import_reader::blockers::Blocker> {
        Graphs::of(source)
    }
}

/// A material source on every channel, with a coverage output.
fn material(alpha_output: &str) -> ProjectSource {
    let ProjectSource::Procedural(mut p) = shape("material", vec![]) else { panic!("a procedural") };
    p.channels = ChannelMask(u64::MAX);
    p.mode = 1;
    if let Some(q) = &mut p.procedural {
        q.alpha_output = alpha_output.into();
    }
    ProjectSource::Procedural(p)
}

#[test]
fn a_brush_painting_a_graph_paints_what_a_fill_of_it_runs() {
    let mut e = effect(PaintTool::Brush);
    e.colors = vec![material("")];
    let s = step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), None, Some(&Graphs)).unwrap();
    // base colour from the graph; roughness, which it has no output for, not painted
    assert!(s.values.is_empty());
    assert_eq!(s.sources.len(), 1);
    assert!(matches!(&s.sources[0], (Channel::BaseColor, ReplaySource::Graph(g), o) if g.name == "material" && *o == 1.0));
    assert!(s.notes.iter().any(|n| n == brush::GRAPH_TEXELS), "{:?}", s.notes);
    // in a mask: its grey output
    let m = step(&e, StepTarget::Mask, &channels(), &ProjectImages::none(), None, Some(&Graphs)).unwrap();
    assert!(matches!(&m.mask_source, Some(ReplaySource::Graph(_))) && m.mask_value == 1.0);
    // without the project's graphs: refused as before; a graph that does not run: its own blocker
    assert_eq!(of(&e, StepTarget::Content).unwrap_err().key.to_string(), "brush:paints-procedural");
    e.colors = vec![shape("elsewhere", vec![])];
    if let ProjectSource::Procedural(p) = &mut e.colors[0] {
        p.channels = ChannelMask(1);
    }
    let b = step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), None, Some(&Graphs)).unwrap_err();
    assert_eq!((b.key.kind, b.key.name.as_str()), (BlockerKind::Graph, "elsewhere"));
    // an image in a mask stays refused; an image that cannot be read is refused with its own key
    let image =
        ProjectSource::Bitmap(texturecraft_import_reader::model::BitmapSource { uid: None, channels: ChannelMask(u64::MAX), url: "/x?version=1".into() });
    e.colors = vec![image];
    assert_eq!(of(&e, StepTarget::Mask).unwrap_err().key.to_string(), "brush:paints-bitmap");
    assert_eq!(of(&e, StepTarget::Content).unwrap_err().key.to_string(), "brush:bitmap-unreadable");
}

#[test]
fn a_material_with_its_own_coverage_is_the_tip_of_a_path_under_a_black_alpha() {
    let mut e = effect(PaintTool::Brush);
    e.colors = vec![material("opacity")];
    e.alpha = vec![uniform(0, [0.0; 3], 1.0)];
    e.paths.push(texturecraft_import_reader::model::StrokePath3d::line([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
    let s = step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), None, Some(&Graphs)).unwrap();
    assert_eq!(s.brush.tip, ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![1.0]) });
    assert!(s.notes.iter().any(|n| n == brush::SOURCE_ALONG), "{:?}", s.notes);
    // without paths, or without a coverage output, the black alpha stays
    let black = ReplayTip::Image { width: 1, height: 1, values: Arc::from(vec![0.0]) };
    let mut no_paths = e.clone();
    no_paths.paths.clear();
    assert_eq!(step(&no_paths, StepTarget::Content, &channels(), &ProjectImages::none(), None, Some(&Graphs)).unwrap().brush.tip, black);
    e.colors = vec![material("")];
    assert_eq!(step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), None, Some(&Graphs)).unwrap().brush.tip, black);
}

#[test]
fn the_legacy_gaussian_alpha_is_drawn_from_its_name() {
    let mut e = effect(PaintTool::Brush);
    let ProjectSource::Procedural(mut p) = shape("gaussian", vec![]) else { panic!("a procedural") };
    if let Some(q) = &mut p.procedural {
        q.url = format!("resource://shelf/gaussian/gaussian?version=1{}", texturecraft_import_reader::format_names::LEGACY_ALPHA_SUFFIX);
    }
    e.alpha = vec![ProjectSource::Procedural(p)];
    let s = of(&e, StepTarget::Content).unwrap();
    let ReplayTip::Image { width, values, .. } = &s.brush.tip else { panic!("an image") };
    let n = *width as usize;
    let centre = values[n / 2 * n + n / 2];
    // exp(−r²/2σ²): 1 in the middle, exp(−4.5) at the middle of a side
    assert!(centre > 0.99 && (values[n / 2 * n] - (-4.5f32).exp()).abs() < 0.01, "{centre}");
    assert!(s.notes.iter().any(|n| n == tip::GAUSSIAN_NOTE));
    // a gaussian of any other package is a graph like any other
    e.alpha = vec![shape("gaussian", vec![])];
    assert_eq!(of(&e, StepTarget::Content).unwrap_err().key.kind, BlockerKind::BrushAlpha);
}

#[test]
fn a_graph_tip_reads_what_feeds_its_image_inputs() {
    use crate::graph_library::GraphLibrary;
    use texturecraft_import_reader::decompile::Decompiled;
    use texturecraft_import_reader::model::ProceduralInput;
    let mut lib = GraphLibrary::default();
    // the tip passes its image input through; the input declares white
    let pass = Decompiled {
        text: "format 3\nsize 8\nnode 1 input idx=0\noutput grey 1\n".into(),
        images: vec!["shape".into()],
        image_defaults: vec![Some([1.0; 3])],
        outputs: vec![("grey".into(), 1)],
        warnings: vec![],
        colour_outputs: vec![],
    };
    lib.insert_as(&["pass".to_string()], "", pass);
    let grey = Decompiled {
        text: "format 3\nsize 8\nnode 1 color r=0.5 g=0.5 b=0.5\noutput grey 1\n".into(),
        images: vec![],
        image_defaults: vec![],
        outputs: vec![("grey".into(), 1)],
        warnings: vec![],
        colour_outputs: vec![],
    };
    lib.insert_as(&["half".to_string()], "", grey);
    let tip_fed = |source: Option<ProjectSource>| {
        let ProjectSource::Procedural(mut p) = shape("pass", vec![]) else { panic!("a procedural") };
        if let Some(q) = &mut p.procedural {
            q.inputs = vec![ProceduralInput {
                identifiers: vec![("shape".into(), texturecraft_import_reader::model::ProjectChannel::Bit(0))],
                source: source.map(Box::new),
            }];
        }
        let mut e = effect(PaintTool::Brush);
        e.alpha = vec![ProjectSource::Procedural(p)];
        let s = step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), Some(&lib), None).unwrap();
        let ReplayTip::Image { values, .. } = s.brush.tip else { panic!("an image") };
        values[values.len() / 2]
    };
    assert!((tip_fed(None) - 1.0).abs() < 0.01, "unfed: its declaration");
    assert!((tip_fed(Some(uniform(0, [0.25; 3], 1.0))) - 0.25).abs() < 0.01, "a uniform");
    assert!((tip_fed(Some(shape("half", vec![]))) - 0.5).abs() < 0.01, "another graph's grey");
}

#[test]
fn a_graph_tip_is_rendered_once_and_shared() {
    use crate::graph_library::GraphLibrary;
    use texturecraft_import_reader::decompile::Decompiled;
    let mut lib = GraphLibrary::default();
    let grey = Decompiled {
        text: "format 3\nsize 8\nnode 1 color r=0.5 g=0.5 b=0.5\noutput grey 1\n".into(),
        images: vec![],
        image_defaults: vec![],
        outputs: vec![("grey".into(), 1)],
        warnings: vec![],
        colour_outputs: vec![],
    };
    lib.insert_as(&["half".to_string()], "", grey);
    let tip = |seed: Option<i32>| {
        let mut e = effect(PaintTool::Brush);
        let tweaks = seed.map(|v| vec![Tweak { identifier: "$randomseed".into(), value: TweakValue::Int(v) }]).unwrap_or_default();
        e.alpha = vec![shape("half", tweaks)];
        let s = step(&e, StepTarget::Content, &channels(), &ProjectImages::none(), Some(&lib), None).unwrap();
        let ReplayTip::Image { values, .. } = s.brush.tip else { panic!("an image") };
        values
    };
    let (a, b) = (tip(None), tip(None));
    // the same graph, values and seed: one render, its values shared
    assert!(Arc::ptr_eq(&a, &b));
    // another seed is another render (of the same values here)
    let c = tip(Some(7));
    assert!(!Arc::ptr_eq(&a, &c) && a == c);
}

#[test]
fn a_particle_brush_replays_its_strokes_as_a_simulation() {
    let mut e = effect(PaintTool::Brush);
    let physics = texturecraft_import_reader::model::PhysicsEmitter {
        emitter: "shelf/EmitSingle?version=1".into(),
        receiver: "shelf/RcvLeak?version=1".into(),
        delta_time: Some(0.01),
        ..Default::default()
    };
    e.brush = Some(ProjectBrush { size: 0.2, flow: 0.8, emitter: StampEmitter::Physics(physics), ..ProjectBrush::default() });
    // a tip that cannot be made does not stop it: a soft round dab stands in
    e.alpha = vec![shape("not_in_the_project", vec![])];
    let s = of(&e, StepTarget::Content).unwrap();
    let p = s.particles.expect("particles");
    assert_eq!((p.per_point, p.gravity, p.step, p.file_units), (1.0, 0.5, 0.01, false));
    assert_eq!(s.strokes.len(), 2);
    assert_eq!(s.brush.tip, ReplayTip::Round { hardness: 0.5 });
    assert!(s.notes.iter().any(|n| n == "particles approximated: leak on single"), "{:?}", s.notes);
    assert!(s.notes.iter().any(|n| n.starts_with("particle tip laid as a soft round dab")), "{:?}", s.notes);
    // the same in a mask
    assert!(of(&e, StepTarget::Mask).unwrap().particles.is_some());
    // strokes without particles carry none
    assert!(of(&effect(PaintTool::Brush), StepTarget::Content).unwrap().particles.is_none());
}

#[test]
fn particle_speeds_in_the_imported_files_units_shrink_by_its_radius() {
    let mut e = effect(PaintTool::Brush);
    let physics = texturecraft_import_reader::model::PhysicsEmitter {
        emitter: "shelf/EmitSingle?version=1".into(),
        receiver: "shelf/RcvBrush?version=1".into(),
        ..Default::default()
    };
    e.brush = Some(ProjectBrush { emitter: StampEmitter::Physics(physics), ..ProjectBrush::default() });
    let s = of(&e, StepTarget::Content).unwrap();
    let job = |s: &ReplayStep| ReplayJob { target: ReplayTarget::Mask(texturecraft_core::LayerId(1)), label: String::new(), steps: vec![s.clone()] };
    let mut jobs = vec![job(&s)];
    particles::in_file_units(&mut jobs, Some(1730.0));
    let p = jobs[0].steps[0].particles.unwrap();
    assert!((p.speed - 0.25 / 1730.0).abs() < 1e-9 && !p.file_units);
    // once only, and nothing when the radius is not known
    particles::in_file_units(&mut jobs, Some(1730.0));
    assert_eq!(jobs[0].steps[0].particles.unwrap().speed, p.speed);
    let mut jobs = vec![job(&s)];
    particles::in_file_units(&mut jobs, None);
    assert_eq!(jobs[0].steps[0].particles.unwrap().speed, 0.25);
}

#[test]
fn a_brush_painting_images_only_places_them_on_the_view() {
    let image = || {
        let res = texturecraft_compose::ImageRes::new(1, 1, vec![[0.5; 4]], false);
        ReplaySource::Texels(Arc::new(ReplayTexels::Image(Arc::new(res))))
    };
    let m = [2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, 0.0, 1.0];
    let images = vec![(Channel::BaseColor, image(), 1.0), (Channel::Normal, image(), 1.0)];
    assert_eq!(brush::placement(PaintTool::Brush, StepTarget::Content, &images, m), Some(m));
    // a graph among them, no source, a mask or another tool: laid texel for texel as before
    let mixed = vec![(Channel::BaseColor, image(), 1.0), (Channel::Roughness, ReplaySource::Failed("x".into()), 1.0)];
    assert_eq!(brush::placement(PaintTool::Brush, StepTarget::Content, &mixed, m), None);
    assert_eq!(brush::placement(PaintTool::Brush, StepTarget::Content, &[], m), None);
    assert_eq!(brush::placement(PaintTool::Brush, StepTarget::Mask, &images, m), None);
    assert_eq!(brush::placement(PaintTool::Eraser, StepTarget::Content, &images, m), None);
    assert_eq!(brush::placement(PaintTool::Projection, StepTarget::Content, &[], m), Some(m));
}
