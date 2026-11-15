//! Stacks built from synthetic models.

use texturecraft_core::{BlendMode, Channel, ChannelFormat, EffectKind, Fidelity, GeometryMask, LayerKind, MaskBackground, SafetyFlagKind, Source};
use texturecraft_import_reader::model::{
    BitmapSource, ChannelBlending, ChannelMask, EffectGroup, EffectHeader, FillEffect, FillPlacement, GeometryMaskKind, LayerKind as ProjectLayerKind,
    Levels as ProjectLevels, LevelsEffect, MaskInitial, PaintEffect, PaintTool, Procedural, ProceduralSource, ProjectBlendMode, ProjectEffect,
    ProjectGeometryMask, ProjectLayer, ProjectMask, ProjectSource, ProjectStroke, ProjectTextureSet, RawEntry, ReferenceSource, StrokeView, UniformSource,
};

use super::*;
use crate::channels::SetChannels;
use crate::options::ImportOptions;
use crate::resources::ProjectImages;

const BASE: u64 = 1;
const HEIGHT: u64 = 2;
const ROUGH: u64 = 128;

fn channels() -> SetChannels {
    let mut c = SetChannels::default();
    c.declare(Channel::BaseColor, ChannelFormat::Srgb8);
    c.declare(Channel::Height, ChannelFormat::L16F);
    c.declare(Channel::Roughness, ChannelFormat::L8);
    c
}

fn header(uid: u64, channels: u64, blending: Vec<ChannelBlending>) -> EffectHeader {
    EffectHeader { uid: Some(uid), enabled: true, label: String::new(), channels: ChannelMask(channels), blending, unhandled: Vec::new() }
}

fn blend(channels: u64, mode: u64, opacity: f32) -> ChannelBlending {
    ChannelBlending { channels: ChannelMask(channels), mode: ProjectBlendMode::from_raw(mode), opacity }
}

fn uniform(channel: u64, v: [f32; 3]) -> ProjectSource {
    ProjectSource::Uniform(UniformSource { uid: None, channels: ChannelMask(channel), color: v, opacity: 1.0 })
}

fn procedural(channels: u64, name: &str) -> ProjectSource {
    let p = Procedural {
        uid: None,
        url: format!("resource://shelf/{name}/{name}?version=1"),
        tweaks: vec![],
        outputs: vec![],
        inputs: vec![],
        alpha_output: String::new(),
        mono_output: 0,
    };
    ProjectSource::Procedural(ProceduralSource { uid: None, channels: ChannelMask(channels), mode: 0, stamp_cycle: 0, stroke_wires: 0, procedural: Some(p) })
}

fn fill(uid: u64, channels: u64, sources: Vec<ProjectSource>, blending: Vec<ChannelBlending>) -> ProjectEffect {
    ProjectEffect::Fill(FillEffect {
        header: header(uid, channels, blending),
        sources,
        projection: 6,
        filtering: 2,
        uv_transform: None,
        placement: FillPlacement::default(),
    })
}

fn layer(uid: u64, name: &str, kind: ProjectLayerKind, content: Vec<ProjectEffect>) -> ProjectLayer {
    ProjectLayer { uid: Some(uid), name: name.into(), kind, visible: true, blending: Vec::new(), mask: None, geometry: None, content, unhandled: Vec::new() }
}

fn stroke() -> ProjectStroke {
    ProjectStroke {
        uid: None,
        points: Vec::new(),
        pressure: Vec::new(),
        tilt_keys: 0,
        complete: true,
        view_matrix: [0.0; 16],
        projection_matrix: [0.0; 16],
        view: StrokeView::Uv,
        duplicate_count: 0,
        duplicate_matrix: [0.0; 16],
        random_seed: 0,
        lazy_distance: 0.0,
    }
}

fn paint(uid: u64, channels: u64) -> ProjectEffect {
    ProjectEffect::Paint(PaintEffect {
        header: header(uid, channels, vec![]),
        tool: PaintTool::Brush,
        opacity: 1.0,
        strokes: vec![stroke(), stroke()],
        paths: vec![],
        brush_uid: None,
        brush: None,
        stamp_blending: 0,
        colors: vec![],
        alpha: vec![],
        stencil: None,
        stencil_matrix: None,
    })
}

fn set(layers: Vec<ProjectLayer>) -> ProjectTextureSet {
    ProjectTextureSet {
        uid: Some(1),
        material_uid: Some(7),
        name: "S".into(),
        resolution_log2: Some([10, 10]),
        channels: vec![],
        shader_model: None,
        layers,
        mesh_maps: vec![],
        uses_uv_tiles: false,
        uv_tile_entries: 0,
        unhandled: vec![],
    }
}

fn build(layers: Vec<ProjectLayer>) -> ImportedStack {
    let channels = channels();
    let images = ProjectImages::none();
    let options = ImportOptions::default();
    let parts = ["Body".to_string(), "Eyes".to_string()];
    let input = SetInput {
        channels: &channels,
        images: &images,
        mesh_maps: &[],
        parts: &parts,
        options: &options,
        replay: false,
        graphs: None,
        geometry: None,
        layers_done: None,
    };
    import(&set(layers), &input, 10)
}

fn by_name<'s>(stack: &'s ImportedStack, name: &str) -> &'s texturecraft_core::Layer {
    stack.layers.iter().find(|l| l.name == name).unwrap()
}

#[test]
fn a_full_uniform_fill_is_a_fill_layer_with_its_blending_and_exact() {
    let mut l =
        layer(1, "Paint it", ProjectLayerKind::Fill, vec![fill(2, BASE | ROUGH, vec![uniform(BASE, [0.2, 0.4, 0.6]), uniform(ROUGH, [0.3; 3])], vec![])]);
    l.blending = vec![blend(BASE, 4, 0.5)];
    let stack = build(vec![l]);
    let ours = by_name(&stack, "Paint it");
    let LayerKind::Fill(f) = &ours.kind else { panic!("fill layer expected: {:?}", ours.kind) };
    assert_eq!(f.sources.get(&Channel::BaseColor), Some(&Source::Uniform([0.2, 0.4, 0.6])));
    assert_eq!(f.sources.get(&Channel::Roughness), Some(&Source::Uniform([0.3; 3])));
    let b = ours.blend(Channel::BaseColor).unwrap();
    assert_eq!((b.mode, b.opacity), (BlendMode::Multiply, 0.5), "layer opacity lives in each channel's opacity");
    assert_eq!(ours.blend(Channel::Roughness).unwrap().mode, BlendMode::Normal);
    assert!(ours.blend(Channel::Height).is_none(), "only written channels are enabled");
    assert!(!ours.is_locked() && stack.verdict.all_exact());
    assert_eq!(stack.reports[&ours.id].fidelity, Fidelity::Exact);
    assert_eq!(stack.roots, vec![ours.id]);
    assert!(stack.next_id > ours.id.0);
}

#[test]
fn folders_keep_children_order_paths_and_visibility() {
    let a = layer(2, "A", ProjectLayerKind::Fill, vec![fill(3, ROUGH, vec![uniform(ROUGH, [0.1; 3])], vec![])]);
    let mut b = layer(4, "B", ProjectLayerKind::Fill, vec![fill(5, ROUGH, vec![uniform(ROUGH, [0.9; 3])], vec![])]);
    b.visible = false;
    let mut folder = layer(1, "Folder", ProjectLayerKind::Group(vec![a, b]), vec![]);
    folder.blending = vec![blend(ROUGH, 2, 0.25)];
    let stack = build(vec![folder]);
    let f = by_name(&stack, "Folder");
    let kids = f.children().to_vec();
    let names: Vec<&str> = kids.iter().map(|id| stack.layers.iter().find(|l| l.id == *id).unwrap().name.as_str()).collect();
    assert_eq!(names, ["A", "B"]);
    assert_eq!(f.blend(Channel::Roughness).unwrap().opacity, 0.25);
    assert!(f.blend(Channel::BaseColor).is_some(), "folders blend every declared channel");
    assert!(!by_name(&stack, "B").visible);
    assert_eq!(stack.reports[&kids[1]].path, "Folder / B");
    assert_eq!(stack.verdict.layer_count, 3);
}

#[test]
fn a_procedural_fill_is_a_locked_placeholder_keyed_by_its_graph() {
    let l =
        layer(1, "Rust", ProjectLayerKind::Fill, vec![fill(2, BASE | ROUGH, vec![procedural(BASE | ROUGH, "rust_fine"), uniform(HEIGHT, [0.0; 3])], vec![])]);
    let stack = build(vec![l]);
    let ours = by_name(&stack, "Rust");
    assert!(matches!(ours.kind, LayerKind::Paint), "not a full fill: an empty base with effects");
    let ph: Vec<_> = ours.placeholders().collect();
    assert_eq!(ph.len(), 1);
    assert!(matches!(&ph[0].kind, EffectKind::Placeholder { label, .. } if label.contains("rust_fine")));
    assert_eq!(ours.lock.as_ref().unwrap().keys(), ["graph:rust_fine"]);
    let report = &stack.reports[&ours.id];
    assert_eq!(report.fidelity, Fidelity::Baked);
    assert!(report.flags.iter().any(|f| f.kind == SafetyFlagKind::ContentNotComputable && f.key == "graph:rust_fine"));
    assert!(!stack.verdict.channels[&Channel::BaseColor].is_computable());
    assert!(stack.graphs.contains("rust_fine"));
    assert!(ours.blend(Channel::BaseColor).is_some(), "the graph's channels keep their blending for when graphs compute");
}

#[test]
fn a_mask_generator_hides_and_locks_the_layer() {
    let mut l = layer(1, "Dirt", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [0.8; 3])], vec![])]);
    l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::Black, effects: vec![fill(3, 0, vec![procedural(0, "mg_dirt")], vec![])] });
    let stack = build(vec![l]);
    let ours = by_name(&stack, "Dirt");
    assert!(!ours.visible, "without its mask it would cover the whole set");
    let m = ours.mask.as_ref().unwrap();
    assert_eq!((m.background, m.effects.len()), (MaskBackground::Black, 1));
    assert!(m.effects[0].is_placeholder());
    assert_eq!(ours.lock.as_ref().unwrap().keys(), ["graph:mg_dirt"]);
    assert!(stack.reports[&ours.id].flags.iter().any(|f| f.kind == SafetyFlagKind::MaskNotComputable));
    assert_eq!(stack.verdict.mask_failures.len(), 1);
}

#[test]
fn brush_strokes_stay_in_the_stored_result_and_lock_the_layer() {
    let group = ProjectEffect::Group(EffectGroup { header: header(2, u64::MAX, vec![]), effects: vec![paint(3, BASE), paint(9, BASE)] });
    let stack = build(vec![layer(1, "Paint", ProjectLayerKind::Paint, vec![group])]);
    let ours = by_name(&stack, "Paint");
    assert!(matches!(ours.kind, LayerKind::Paint));
    assert_eq!(ours.lock.as_ref().unwrap().keys(), ["strokes:brush"]);
    let ph: Vec<_> = ours.placeholders().collect();
    assert_eq!(ph.len(), 1, "consecutive paint effects share one placeholder");
    assert_eq!(ph[0].name, "brush strokes (4 strokes in 2 paint effects)");
    assert_eq!(stack.reports[&ours.id].fidelity, Fidelity::Baked);
}

fn reference(anchor: Option<u64>) -> ReferenceSource {
    ReferenceSource { uid: None, channels: ChannelMask(0), anchor_uid: anchor, anchor_label: String::new(), levels: None, bindings: vec![(HEIGHT, 0)], mode: 2 }
}

#[test]
fn references_read_the_anchor_effect_by_its_id() {
    let mut anchor = layer(1, "Base", ProjectLayerKind::Fill, vec![fill(2, HEIGHT, vec![uniform(HEIGHT, [0.5; 3])], vec![])]);
    anchor.content.push(ProjectEffect::Anchor(EffectHeader { uid: Some(99), enabled: true, ..EffectHeader::default() }));
    let mut reader = layer(3, "Reader", ProjectLayerKind::Fill, vec![fill(4, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
    reader.mask = Some(ProjectMask {
        enabled: true,
        initial: MaskInitial::Black,
        effects: vec![fill(5, 0, vec![ProjectSource::Reference(reference(Some(99)))], vec![])],
    });
    let stack = build(vec![anchor, reader]);
    let base = by_name(&stack, "Base");
    let anchor_id = base.content_effects.iter().find(|e| matches!(e.kind, EffectKind::Anchor)).unwrap().id;
    let m = by_name(&stack, "Reader").mask.as_ref().unwrap();
    assert!(matches!(&m.effects[0].kind, EffectKind::Fill { source: Source::Anchor { anchor, channel: Channel::Height }, .. } if *anchor == anchor_id));
    assert!(stack.verdict.all_exact(), "{:?}", stack.verdict);
    // a missing anchor is not computable
    let mut lost = layer(6, "Lost", ProjectLayerKind::Fill, vec![fill(7, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
    lost.mask =
        Some(ProjectMask { enabled: true, initial: MaskInitial::White, effects: vec![fill(8, 0, vec![ProjectSource::Reference(reference(Some(5)))], vec![])] });
    let stack = build(vec![lost]);
    assert_eq!(by_name(&stack, "Lost").lock.as_ref().unwrap().keys(), ["feature:missing-anchor"]);
}

#[test]
fn anchors_inside_masks_are_recorded_and_read() {
    let mark = || ProjectEffect::Anchor(EffectHeader { uid: Some(42), enabled: true, ..EffectHeader::default() });
    let read = |uid: u64| fill(uid, 0, vec![ProjectSource::Reference(reference(Some(42)))], vec![]);
    // the anchor and a fill reading it in the same mask, and a layer above reading it
    let mut low = layer(1, "Low", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
    low.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::Black, effects: vec![fill(3, 0, vec![uniform(0, [0.4; 3])], vec![]), mark(), read(4)] });
    let mut top = layer(5, "Top", ProjectLayerKind::Fill, vec![fill(6, ROUGH, vec![uniform(ROUGH, [0.5; 3])], vec![])]);
    top.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::Black, effects: vec![read(7)] });
    let stack = build(vec![low, top]);
    assert!(stack.verdict.all_exact(), "{:?}", stack.verdict);
    let low = by_name(&stack, "Low");
    assert!(low.lock.is_none(), "{:?}", low.lock);
    let m = low.mask.as_ref().unwrap();
    let anchor_id = m.effects.iter().find(|e| matches!(e.kind, EffectKind::Anchor)).unwrap().id;
    assert!(matches!(&m.effects[2].kind, EffectKind::Fill { source: Source::Anchor { anchor, .. }, .. } if *anchor == anchor_id));
    let t = by_name(&stack, "Top");
    assert!(
        t.lock.is_none()
            && matches!(&t.mask.as_ref().unwrap().effects[0].kind, EffectKind::Fill { source: Source::Anchor { anchor, .. }, .. } if *anchor == anchor_id)
    );
}

#[test]
fn fills_split_by_blend_and_levels_take_the_reciprocal_gamma() {
    let fx = fill(3, BASE | ROUGH, vec![uniform(BASE, [1.0, 0.0, 0.0]), uniform(ROUGH, [0.5; 3])], vec![blend(BASE, 4, 1.0), blend(ROUGH, 2, 0.5)]);
    let levels = ProjectEffect::Levels(LevelsEffect {
        header: header(4, ROUGH | BASE, vec![]),
        levels: Some(ProjectLevels {
            input_minimum: [0.0; 3],
            input_maximum: [1.0; 3],
            gamma: [0.5; 3],
            output_minimum: [0.0; 3],
            output_maximum: [1.0; 3],
            clamp: 1,
        }),
    });
    let base = fill(2, ROUGH | BASE, vec![uniform(ROUGH, [0.2; 3]), uniform(BASE, [0.2; 3])], vec![]);
    let stack = build(vec![layer(1, "L", ProjectLayerKind::Fill, vec![base, fx, levels])]);
    let ours = by_name(&stack, "L");
    assert!(matches!(ours.kind, LayerKind::Fill(_)));
    let materials: Vec<_> = ours.content_effects.iter().filter(|e| matches!(e.kind, EffectKind::Material(_))).collect();
    assert_eq!(materials.len(), 2, "one material fill per blend group");
    assert_eq!((materials[0].blend, materials[1].blend, materials[1].opacity), (BlendMode::Multiply, BlendMode::Normal, 0.5));
    let lv = ours.content_effects.iter().find_map(|e| match &e.kind {
        EffectKind::Levels(l) => Some(*l),
        _ => None,
    });
    assert_eq!(lv.map(|l| l.gamma), Some(2.0));
    assert!(stack.verdict.all_exact(), "{:?}", stack.verdict.channels);
}

#[test]
fn geometry_masks_list_included_parts_and_unknown_blends_are_flagged() {
    let mut l = layer(1, "Eyes only", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [0.4; 3])], vec![])]);
    l.geometry = Some(ProjectGeometryMask {
        enabled: true,
        kind: GeometryMaskKind::Meshes,
        mesh_default: true,
        mesh_list: vec!["Body".into()],
        uv_tile_default: true,
        uv_tile_entries: 0,
    });
    l.blending = vec![blend(ROUGH, 77, 1.0)];
    let stack = build(vec![l]);
    let ours = by_name(&stack, "Eyes only");
    assert_eq!(ours.geometry_mask, Some(GeometryMask::Parts(vec!["Eyes".into()])));
    let r = &stack.reports[&ours.id];
    assert!(r.flags.iter().any(|f| f.kind == SafetyFlagKind::Guessed), "{:?}", r.flags);
    assert_eq!(r.fidelity, Fidelity::Approximated);
}

#[test]
fn unreadable_bitmaps_and_unknown_classes_are_not_computable_and_ids_are_unique() {
    let bitmap = ProjectSource::Bitmap(BitmapSource { uid: None, channels: ChannelMask(BASE), url: "/Wood?version=abc".into() });
    let other = ProjectEffect::Other(RawEntry { class: "DataActionNew".into(), uid: None });
    let w = layer(1, "W", ProjectLayerKind::Fill, vec![fill(2, BASE, vec![bitmap], vec![]), other]);
    let stack = build(vec![w, layer(3, "X", ProjectLayerKind::Other("DataLayerNew".into()), vec![])]);
    assert_eq!(by_name(&stack, "W").lock.as_ref().unwrap().keys(), ["feature:unreadable-bitmap", "class:DataActionNew"]);
    assert_eq!(by_name(&stack, "X").lock.as_ref().unwrap().keys(), ["class:DataLayerNew"]);
    let mut ids: Vec<u64> = stack.layers.iter().map(|l| l.id.0).collect();
    ids.extend(stack.layers.iter().flat_map(|l| l.content_effects.iter().chain(l.mask.iter().flat_map(|m| m.effects.iter())).map(|e| e.id.0)));
    let unique: std::collections::BTreeSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
    assert!(ids.iter().all(|id| *id >= 10 && *id < stack.next_id));
}

fn build_replaying(layers: Vec<ProjectLayer>) -> ImportedStack {
    let channels = channels();
    let images = ProjectImages::none();
    let options = ImportOptions::default();
    let parts = ["Body".to_string()];
    let input = SetInput {
        channels: &channels,
        images: &images,
        mesh_maps: &[],
        parts: &parts,
        options: &options,
        replay: true,
        graphs: None,
        geometry: None,
        layers_done: None,
    };
    import(&set(layers), &input, 10)
}

fn painting(uid: u64, channels: u64, tool: PaintTool, colors: Vec<ProjectSource>) -> ProjectEffect {
    match paint(uid, channels) {
        ProjectEffect::Paint(mut p) => {
            p.tool = tool;
            p.colors = colors;
            ProjectEffect::Paint(p)
        }
        other => other,
    }
}

#[test]
fn with_a_replayer_brush_work_becomes_the_layers_pixels_unlocked() {
    let group = ProjectEffect::Group(EffectGroup {
        header: header(2, u64::MAX, vec![]),
        effects: vec![
            painting(3, BASE | ROUGH, PaintTool::Brush, vec![uniform(BASE, [1.0, 0.0, 0.0]), uniform(ROUGH, [0.25; 3])]),
            painting(9, BASE, PaintTool::Eraser, vec![]),
        ],
    });
    let stack = build_replaying(vec![layer(1, "Paint", ProjectLayerKind::Paint, vec![group])]);
    let ours = by_name(&stack, "Paint");
    assert!(ours.lock.is_none() && ours.placeholders().count() == 0 && ours.content_effects.is_empty(), "{ours:?}");
    assert_eq!(stack.jobs.len(), 1);
    let job = &stack.jobs[0];
    assert_eq!(job.target, crate::replay::ReplayTarget::Content(ours.id));
    assert_eq!(job.steps.len(), 2);
    assert_eq!(job.steps[0].values, vec![(Channel::BaseColor, [1.0, 0.0, 0.0], 1.0), (Channel::Roughness, [0.25; 3], 1.0)]);
    assert_eq!(job.steps[1].mode, crate::replay::PaintMode::Erase);
    assert_eq!(stack.reports[&ours.id].fidelity, Fidelity::Approximated);
    assert!(matches!(stack.verdict.channels[&Channel::BaseColor], Status::Approximated(_)));
    assert!(ours.channels.contains_key(&Channel::Roughness));
}

#[test]
fn with_a_replayer_painted_masks_become_paint_effects() {
    let mut l = layer(1, "Masked", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
    l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::Black, effects: vec![painting(5, 0, PaintTool::Brush, vec![uniform(0, [0.5; 3])])] });
    let stack = build_replaying(vec![l]);
    let ours = by_name(&stack, "Masked");
    assert!(ours.lock.is_none() && ours.visible, "{:?}", ours.lock);
    let m = ours.mask.as_ref().unwrap();
    assert!(matches!(m.effects[0].kind, EffectKind::Paint));
    assert_eq!(stack.jobs.len(), 1);
    assert_eq!(stack.jobs[0].target, crate::replay::ReplayTarget::Mask(m.effects[0].id));
    assert_eq!(stack.jobs[0].steps[0].mask_value, 0.5);
}

#[test]
fn strokes_that_cannot_be_replayed_stay_placeholders_with_their_reason() {
    // projection painting without an image among the brush work keeps all of it a placeholder: they paint one image in
    // order
    let group = ProjectEffect::Group(EffectGroup {
        header: header(2, u64::MAX, vec![]),
        effects: vec![painting(3, BASE, PaintTool::Brush, vec![uniform(BASE, [1.0; 3])]), painting(9, BASE, PaintTool::Projection, vec![])],
    });
    let mut l = layer(1, "Paint", ProjectLayerKind::Paint, vec![group]);
    let mut emitterless = paint(5, 0);
    if let ProjectEffect::Paint(p) = &mut emitterless {
        p.brush = Some(texturecraft_import_reader::model::ProjectBrush {
            emitter: texturecraft_import_reader::model::StampEmitter::Other(String::new()),
            ..Default::default()
        });
    }
    l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::White, effects: vec![emitterless] });
    let stack = build_replaying(vec![l]);
    let ours = by_name(&stack, "Paint");
    let keys = ours.lock.as_ref().unwrap().keys();
    assert!(keys.contains(&"brush:projection") && keys.contains(&"strokes:brush") && keys.contains(&"brush:no-emitter"), "{keys:?}");
    assert!(stack.jobs.is_empty());
    let reasons: Vec<String> = ours
        .placeholders()
        .chain(ours.mask.iter().flat_map(|m| m.effects.iter()).filter(|e| matches!(e.kind, EffectKind::Placeholder { .. })))
        .filter_map(|e| match &e.kind {
            EffectKind::Placeholder { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .collect();
    assert!(reasons.iter().any(|r| r.contains("without an emitter")), "{reasons:?}");
}

/// With the project's graphs decompiled, a procedural fill whose channels blend differently becomes one graph source
/// per channel, each in the fill effect of its own blend (a material laid on whole with its first channel's blend
/// added every channel's output), and the layer is not locked.
#[test]
fn a_decompiled_fill_lays_each_channel_with_its_own_blend() {
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
    lib.insert_as(&["strata".to_string()], "", d);
    let (channels, images, options) = (channels(), ProjectImages::none(), ImportOptions::default());
    let parts = ["Body".to_string()];
    let input = SetInput {
        channels: &channels,
        images: &images,
        mesh_maps: &[],
        parts: &parts,
        options: &options,
        replay: false,
        graphs: Some(&lib),
        geometry: None,
        layers_done: None,
    };
    let blending = vec![blend(BASE, 2, 1.0), blend(HEIGHT, 4, 1.0)];
    let l = layer(1, "Strata", ProjectLayerKind::Paint, vec![fill(2, BASE | HEIGHT, vec![procedural(BASE | HEIGHT, "strata")], blending)]);
    let stack = import(&set(vec![l]), &input, 10);
    let strata = by_name(&stack, "Strata");
    assert!(!strata.is_locked() && strata.placeholders().next().is_none());
    let by_channel = |c: Channel| {
        strata.content_effects.iter().find_map(|e| match &e.kind {
            texturecraft_core::EffectKind::Material(f) => f.sources.get(&c).map(|s| (e.blend, s.graph().is_some())),
            _ => None,
        })
    };
    let (base, height) = (by_channel(Channel::BaseColor).unwrap_or_else(|| panic!("{:#?}", strata.content_effects)), by_channel(Channel::Height).unwrap());
    assert!(base.1 && height.1, "graph sources");
    assert_ne!(base.0, height.0, "each channel keeps its own blend");
}

fn mask_group(uid: u64, mode: texturecraft_import_reader::model::ProjectBlendMode, opacity: f32, effects: Vec<ProjectEffect>) -> ProjectEffect {
    ProjectEffect::Group(EffectGroup { header: header(uid, u64::MAX, vec![blend(0, mode.raw(), opacity)]), effects })
}

#[test]
fn a_mask_group_with_its_own_blend_is_one_paint_effect_with_that_blend() {
    use texturecraft_import_reader::model::ProjectBlendMode;
    let group = mask_group(
        4,
        ProjectBlendMode::Subtract,
        0.5,
        vec![painting(5, 0, PaintTool::Brush, vec![uniform(0, [1.0; 3])]), painting(6, 0, PaintTool::Brush, vec![uniform(0, [0.0; 3])])],
    );
    let mut l = layer(1, "Masked", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
    l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::White, effects: vec![group] });
    let stack = build_replaying(vec![l]);
    let m = by_name(&stack, "Masked").mask.as_ref().unwrap();
    // not one effect per brush laid normally over the mask: one layer of the group's strokes, subtracted at half strength
    assert_eq!(m.effects.len(), 1, "{:?}", m.effects);
    assert!(matches!(m.effects[0].kind, EffectKind::Paint));
    assert_eq!(m.effects[0].blend, texturecraft_core::BlendMode::Subtract);
    assert!((m.effects[0].opacity - 0.5).abs() < 1e-6);
    assert_eq!(stack.jobs.len(), 1);
    let job = &stack.jobs[0];
    assert_eq!(job.target, crate::replay::ReplayTarget::Mask(m.effects[0].id));
    assert_eq!(job.steps.iter().map(|s| s.mask_value).collect::<Vec<_>>(), vec![1.0, 0.0]);
    assert!(job.steps.iter().all(|s| s.mode == crate::replay::PaintMode::Paint));
}

#[test]
fn an_eraser_in_a_mask_group_lowers_the_groups_coverage_and_outside_one_paints_black() {
    use texturecraft_import_reader::model::ProjectBlendMode;
    let erasing = mask_group(
        4,
        ProjectBlendMode::Normal,
        1.0,
        vec![painting(5, 0, PaintTool::Brush, vec![uniform(0, [1.0; 3])]), painting(6, 0, PaintTool::Eraser, vec![])],
    );
    let mut l = layer(1, "Masked", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
    l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::Black, effects: vec![erasing, painting(7, 0, PaintTool::Eraser, vec![])] });
    let stack = build_replaying(vec![l]);
    let m = by_name(&stack, "Masked").mask.as_ref().unwrap();
    assert_eq!(m.effects.len(), 2, "{:?}", m.effects);
    assert_eq!(stack.jobs.len(), 2);
    assert_eq!(stack.jobs[0].steps.len(), 2);
    assert_eq!(stack.jobs[0].steps[1].mode, crate::replay::PaintMode::Erase);
    // the lone eraser paints black over the whole mask
    assert_eq!(stack.jobs[1].steps[0].mode, crate::replay::PaintMode::Paint);
    assert_eq!(stack.jobs[1].steps[0].mask_value, 0.0);
}

#[test]
fn a_mask_group_holding_more_than_brush_work_or_without_replay_stays_flat() {
    use texturecraft_import_reader::model::ProjectBlendMode;
    let mixed = || {
        mask_group(
            4,
            ProjectBlendMode::Subtract,
            1.0,
            vec![painting(5, 0, PaintTool::Brush, vec![uniform(0, [1.0; 3])]), fill(6, 0, vec![uniform(0, [0.5; 3])], vec![])],
        )
    };
    let masked = |group: ProjectEffect| {
        let mut l = layer(1, "Masked", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
        l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::White, effects: vec![group] });
        l
    };
    let stack = build_replaying(vec![masked(mixed())]);
    assert_eq!(by_name(&stack, "Masked").mask.as_ref().unwrap().effects.len(), 2);
    let only_brush = mask_group(4, ProjectBlendMode::Subtract, 1.0, vec![painting(5, 0, PaintTool::Brush, vec![uniform(0, [1.0; 3])])]);
    let stack = build(vec![masked(only_brush)]);
    let m = by_name(&stack, "Masked").mask.as_ref().unwrap();
    assert!(m.effects.iter().all(|e| matches!(e.kind, EffectKind::Placeholder { .. })), "{m:?}");
}

/// One quad over the whole UV square as part "Body" of material 0 (two triangles, one polygon).
fn quad_mesh() -> texturecraft_geom::Mesh {
    use glam::{Vec2, Vec3};
    texturecraft_geom::Mesh {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y],
        normals: vec![Vec3::Z; 4],
        uvs: vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)],
        indices: vec![0, 1, 2, 0, 2, 3],
        submeshes: vec![texturecraft_geom::SubMesh { name: "Body".into(), material: 0, start: 0, count: 6 }],
        materials: vec!["m".into()],
        ..texturecraft_geom::Mesh::default()
    }
}

fn polygon_fill(uid: u64, grey: f32, q: [f32; 2], blending: Vec<ChannelBlending>) -> ProjectEffect {
    use texturecraft_import_reader::model::{DecalHit, HitGranularity, HitShape, PolygonFillEffect};
    let hit = DecalHit {
        uid: None,
        shape: HitShape::Point(q),
        color: [grey; 3],
        granularity: HitGranularity::Triangle,
        view: StrokeView::Uv,
        proj_view_inv: [0.0; 16],
        duplicate_count: 0,
        duplicate_matrix: [0.0; 16],
        paint_through: false,
        collision: None,
        occlusion_culling: None,
    };
    ProjectEffect::PolygonFill(PolygonFillEffect { header: header(uid, u64::MAX, blending), hits: vec![hit], unread: 0 })
}

fn build_with_mesh(layers: Vec<ProjectLayer>, mesh: &texturecraft_geom::Mesh) -> ImportedStack {
    let channels = channels();
    let images = ProjectImages::none();
    let options = ImportOptions::default();
    let parts = ["Body".to_string()];
    let geometry = crate::polygon_fill::geometry::SetGeometry::new(mesh, 0, &[vec![true, false]]);
    let input = SetInput {
        channels: &channels,
        images: &images,
        mesh_maps: &[],
        parts: &parts,
        options: &options,
        replay: false,
        graphs: None,
        geometry: Some(&geometry),
        layers_done: None,
    };
    import(&set(layers), &input, 10)
}

#[test]
fn a_mask_group_of_strokes_with_polygon_fills_above_them_is_one_paint_effect() {
    use texturecraft_import_reader::model::ProjectBlendMode;
    let mesh = quad_mesh();
    let build = |group: ProjectEffect| {
        let mut l = layer(1, "Masked", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
        l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::White, effects: vec![group] });
        let (channels, images, options, parts) = (channels(), ProjectImages::none(), ImportOptions::default(), ["Body".to_string()]);
        let geometry = crate::polygon_fill::geometry::SetGeometry::new(&mesh, 0, &[vec![true, false]]);
        let input = SetInput {
            channels: &channels,
            images: &images,
            mesh_maps: &[],
            parts: &parts,
            options: &options,
            replay: true,
            graphs: None,
            geometry: Some(&geometry),
            layers_done: None,
        };
        import(&set(vec![l]), &input, 10)
    };
    let brush = || painting(5, 0, PaintTool::Brush, vec![uniform(0, [1.0; 3])]);
    let stack = build(mask_group(4, ProjectBlendMode::Subtract, 1.0, vec![brush(), polygon_fill(6, 1.0, [0.9, 0.1], vec![])]));
    let m = by_name(&stack, "Masked").mask.as_ref().unwrap();
    assert_eq!(m.effects.len(), 1, "{:?}", m.effects);
    assert_eq!((m.effects[0].kind.clone(), m.effects[0].blend), (EffectKind::Paint, BlendMode::Subtract));
    assert_eq!(stack.jobs.len(), 1);
    assert_eq!(stack.fills.len(), 1);
    assert!(stack.fills[0].over_strokes && stack.fills[0].effect == m.effects[0].id);
    // strokes above a polygon fill: still one paint effect with the group's blending (laid flat, a subtracting group
    // would add its strokes and picks), the picks laid over the strokes
    let stack = build(mask_group(4, ProjectBlendMode::Subtract, 1.0, vec![polygon_fill(6, 1.0, [0.9, 0.1], vec![]), brush()]));
    let m = by_name(&stack, "Masked").mask.as_ref().unwrap();
    assert_eq!(m.effects.len(), 1, "{:?}", m.effects);
    assert_eq!((m.effects[0].kind.clone(), m.effects[0].blend), (EffectKind::Paint, BlendMode::Subtract));
    assert!(stack.fills.len() == 1 && stack.fills[0].over_strokes);
}

#[test]
fn mask_polygon_fills_become_paint_effects_with_their_picks() {
    use texturecraft_import_reader::model::ProjectBlendMode;
    let mesh = quad_mesh();
    let masked = |effects: Vec<ProjectEffect>| {
        let mut l = layer(1, "Masked", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
        l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::Black, effects });
        l
    };
    // a white click on the lower-right triangle (the file's UV, V up), then a click beside the mesh
    let stack = build_with_mesh(vec![masked(vec![polygon_fill(3, 1.0, [0.9, 0.1], vec![]), polygon_fill(4, 1.0, [3.0, 3.0], vec![])])], &mesh);
    let l = by_name(&stack, "Masked");
    let m = l.mask.as_ref().unwrap();
    assert_eq!(m.effects.len(), 2);
    assert!(m.effects.iter().all(|e| matches!(e.kind, EffectKind::Paint)), "{m:?}");
    assert!(!l.is_locked() && stack.verdict.all_exact(), "{:?}", stack.verdict);
    assert_eq!(stack.fills.len(), 1, "the fill picking nothing paints nothing");
    assert_eq!((stack.fills[0].effect, stack.fills[0].runs[0].triangles.iter().copied().collect::<Vec<_>>()), (m.effects[0].id, vec![0]));
    // the pixels: the picked triangle white at full coverage
    let gbuf = texturecraft_geom::UvGBuffer::new(&mesh, 0, 16, 16);
    let img = stack.fills[0].paint(&gbuf).unwrap();
    assert_eq!(img.get(14, 14)[..2], [u16::MAX, u16::MAX]);
    assert_eq!(img.get(1, 1)[..2], [0, 0]);
    // a group of polygon fills with its own blending is one paint effect with the group's blend
    let group = mask_group(5, ProjectBlendMode::Subtract, 1.0, vec![polygon_fill(6, 1.0, [0.9, 0.1], vec![]), polygon_fill(7, 0.5, [0.1, 0.9], vec![])]);
    let stack = build_with_mesh(vec![masked(vec![group])], &mesh);
    let m = by_name(&stack, "Masked").mask.as_ref().unwrap();
    assert_eq!(m.effects.len(), 1);
    assert_eq!((m.effects[0].blend, stack.fills[0].runs.len()), (BlendMode::Subtract, 2));
    assert!(stack.verdict.all_exact(), "{:?}", stack.verdict);
    // without the mesh it stays a locked placeholder
    let stack = build(vec![masked(vec![polygon_fill(3, 1.0, [0.9, 0.1], vec![])])]);
    let l = by_name(&stack, "Masked");
    assert!(l.is_locked() && matches!(l.mask.as_ref().unwrap().effects[0].kind, EffectKind::Placeholder { .. }));
}

#[test]
fn content_polygon_fills_paint_base_colour_on_their_picks() {
    let mesh = quad_mesh();
    let painted = |effects: Vec<ProjectEffect>| layer(1, "Painted", ProjectLayerKind::Paint, effects);
    // a red click on the lower-right triangle, writing every channel: only base colour gets pixels
    let mut red = polygon_fill(3, 0.0, [0.9, 0.1], vec![]);
    if let ProjectEffect::PolygonFill(p) = &mut red {
        p.hits[0].color = [1.0, 0.0, 0.0];
    }
    let stack = build_with_mesh(vec![painted(vec![red])], &mesh);
    let l = by_name(&stack, "Painted");
    assert_eq!(l.content_effects.len(), 1);
    assert!(matches!(l.content_effects[0].kind, EffectKind::Paint) && !l.is_locked(), "{l:?}");
    assert_eq!(stack.fills.len(), 1);
    assert_eq!((stack.fills[0].effect, stack.fills[0].channel), (l.content_effects[0].id, Some(Channel::BaseColor)));
    let gbuf = texturecraft_geom::UvGBuffer::new(&mesh, 0, 16, 16);
    let img = stack.fills[0].paint(&gbuf).unwrap();
    assert_eq!(img.get(14, 14)[..4], [u16::MAX, 0, 0, u16::MAX], "red at full coverage on the picked triangle");
    assert_eq!(img.get(1, 1)[..4], [0, 0, 0, 0], "nothing on the other triangle");
    // a fill picking nothing writes nothing (no pixels, no lock)
    let stack = build_with_mesh(vec![painted(vec![polygon_fill(3, 1.0, [3.0, 3.0], vec![])])], &mesh);
    assert!(stack.fills.is_empty() && !by_name(&stack, "Painted").is_locked());
    // without the mesh it stays a locked placeholder
    let stack = build(vec![painted(vec![polygon_fill(3, 1.0, [0.9, 0.1], vec![])])]);
    let l = by_name(&stack, "Painted");
    assert!(l.is_locked() && matches!(l.content_effects[0].kind, EffectKind::Placeholder { .. }), "{l:?}");
}

fn filter(uid: u64, channels: u64, name: &str) -> ProjectEffect {
    let procedural = Procedural {
        uid: None,
        url: format!("resource://shelf/{name}/{name}?version=1"),
        tweaks: vec![],
        outputs: vec![],
        inputs: vec![],
        alpha_output: String::new(),
        mono_output: 0,
    };
    ProjectEffect::Filter(texturecraft_import_reader::model::FilterEffect {
        header: header(uid, channels, vec![]),
        procedural: Some(procedural),
        inputs: vec![],
        input_alphas: vec![],
    })
}

#[test]
fn a_fill_layer_whose_filter_writes_channels_the_fill_does_not_hold_lays_its_fill_as_an_effect() {
    // the filter works on every channel; the fill holds base colour only: on height the original's fill covers nothing
    let l = layer(1, "Filtered", ProjectLayerKind::Fill, vec![fill(2, BASE, vec![uniform(BASE, [0.2, 0.4, 0.6])], vec![]), filter(3, 0, "warp")]);
    let stack = build(vec![l]);
    let ours = by_name(&stack, "Filtered");
    assert!(matches!(ours.kind, LayerKind::Paint), "{:?}", ours.kind);
    let EffectKind::Material(f) = &ours.content_effects[0].kind else { panic!("the fill first: {:?}", ours.content_effects) };
    assert_eq!(f.sources.get(&Channel::BaseColor), Some(&Source::Uniform([0.2, 0.4, 0.6])));
    assert!(!f.sources.contains_key(&Channel::Height));
    assert!(ours.blend(Channel::Height).is_some(), "the filter still writes height");
    assert_eq!(ours.content_effects.len(), 2, "the fill, then the filter");

    // a filter on the fill's own channels only keeps the fill layer
    let l = layer(1, "Own", ProjectLayerKind::Fill, vec![fill(2, BASE, vec![uniform(BASE, [0.2, 0.4, 0.6])], vec![]), filter(3, BASE, "warp")]);
    let stack = build(vec![l]);
    assert!(matches!(by_name(&stack, "Own").kind, LayerKind::Fill(_)));
}

#[test]
fn levels_act_on_the_channels_they_list_only() {
    let invert =
        ProjectLevels { input_minimum: [0.0; 3], input_maximum: [1.0; 3], gamma: [1.0; 3], output_minimum: [1.0; 3], output_maximum: [0.0; 3], clamp: 1 };
    let levels = ProjectEffect::Levels(LevelsEffect { header: header(3, ROUGH, vec![]), levels: Some(invert.clone()) });
    let base = fill(2, ROUGH | BASE, vec![uniform(ROUGH, [0.2; 3]), uniform(BASE, [0.2; 3])], vec![]);
    let stack = build(vec![layer(1, "L", ProjectLayerKind::Fill, vec![base, levels])]);
    let ours = by_name(&stack, "L");
    let lv = ours.content_effects.iter().find(|e| matches!(e.kind, EffectKind::Levels(_))).unwrap();
    assert_eq!(lv.channels, [Channel::Roughness], "base colour is left as it is");
    assert!(lv.visible);
    assert!(stack.verdict.all_exact(), "{:?}", stack.verdict.channels);
    // levels on a channel the set does not hold (64: glossiness) change nothing
    let gloss = ProjectEffect::Levels(LevelsEffect { header: header(3, 64, vec![]), levels: Some(invert) });
    let base = fill(2, ROUGH | BASE, vec![uniform(ROUGH, [0.2; 3]), uniform(BASE, [0.2; 3])], vec![]);
    let stack = build(vec![layer(1, "L", ProjectLayerKind::Fill, vec![base, gloss])]);
    let lv = by_name(&stack, "L").content_effects.iter().find(|e| matches!(e.kind, EffectKind::Levels(_))).unwrap();
    assert!(!lv.visible, "switched off");
}

#[test]
fn a_smudge_moves_the_brush_work_before_it_and_alone_changes_nothing() {
    use crate::replay::PaintMode;
    use texturecraft_import_reader::model::ProjectBlendMode;
    // in a paint layer's brush work: one job, the smudge after the brush
    let group = ProjectEffect::Group(EffectGroup {
        header: header(2, u64::MAX, vec![]),
        effects: vec![painting(3, BASE, PaintTool::Brush, vec![uniform(BASE, [1.0; 3])]), painting(9, BASE, PaintTool::Smudge, vec![])],
    });
    let stack = build_replaying(vec![layer(1, "Paint", ProjectLayerKind::Paint, vec![group])]);
    let ours = by_name(&stack, "Paint");
    assert!(ours.lock.is_none() && ours.placeholders().count() == 0, "{ours:?}");
    assert_eq!(stack.jobs.len(), 1);
    assert_eq!(stack.jobs[0].steps.iter().map(|s| s.mode).collect::<Vec<_>>(), [PaintMode::Paint, PaintMode::Smudge]);
    assert_eq!(stack.jobs[0].steps[1].values.iter().map(|v| v.0).collect::<Vec<_>>(), [Channel::BaseColor]);
    // in a mask: a group holding a smudge is one paint effect even when it blends normally; a smudge on its own is not
    // replayed (what it moves lies below its effect)
    let smudging = mask_group(4, ProjectBlendMode::Normal, 1.0, vec![painting(5, 0, PaintTool::Brush, vec![]), painting(6, 0, PaintTool::Smudge, vec![])]);
    let mut l = layer(1, "Masked", ProjectLayerKind::Fill, vec![fill(2, ROUGH, vec![uniform(ROUGH, [1.0; 3])], vec![])]);
    l.mask = Some(ProjectMask { enabled: true, initial: MaskInitial::Black, effects: vec![smudging, painting(7, 0, PaintTool::Smudge, vec![])] });
    let stack = build_replaying(vec![l]);
    let ours = by_name(&stack, "Masked");
    let m = ours.mask.as_ref().unwrap();
    assert_eq!(m.effects.len(), 2, "{:?}", m.effects);
    assert!(matches!(m.effects[0].kind, EffectKind::Paint));
    assert_eq!(stack.jobs.len(), 1);
    assert_eq!(stack.jobs[0].steps.iter().map(|s| s.mode).collect::<Vec<_>>(), [PaintMode::Paint, PaintMode::Smudge]);
    assert!(matches!(m.effects[1].kind, EffectKind::Paint), "an empty paint effect: it changes nothing");
    assert!(ours.lock.is_none() && ours.placeholders().count() == 0, "{:?}", ours.lock);
    // in layer content, above other content effects: an empty paint effect as well, and so is an eraser there
    let base = fill(2, BASE, vec![uniform(BASE, [0.5; 3])], vec![]);
    let effects = vec![base, painting(3, BASE, PaintTool::Smudge, vec![]), painting(4, BASE, PaintTool::Eraser, vec![])];
    let stack = build_replaying(vec![layer(1, "Content", ProjectLayerKind::Fill, effects)]);
    let ours = by_name(&stack, "Content");
    assert!(ours.lock.is_none() && ours.placeholders().count() == 0, "{:?}", ours.lock);
    assert!(stack.jobs.is_empty(), "nothing replayed into them");
    assert_eq!(ours.content_effects.iter().filter(|e| matches!(e.kind, EffectKind::Paint)).count(), 2, "{:?}", ours.content_effects);
}
