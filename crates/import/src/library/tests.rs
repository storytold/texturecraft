//! The library offer of synthetic documents.

use std::collections::HashSet;

use texturecraft_import_reader::model::{
    BitmapSource, ChannelMask, EffectHeader, FillEffect, FillPlacement, LayerKind, PaintEffect, PaintTool, Procedural, ProceduralSource, ProjectBrush,
    ProjectDocument, ProjectEffect, ProjectLayer, ProjectSource, ProjectStroke, ProjectTextureSet, StrokeKey, StrokeView, Tweak, TweakValue,
};

use super::*;

fn header() -> EffectHeader {
    EffectHeader { uid: Some(1), enabled: true, label: String::new(), channels: ChannelMask(1), blending: vec![], unhandled: vec![] }
}

fn bitmap(url: &str) -> ProjectSource {
    ProjectSource::Bitmap(BitmapSource { uid: None, channels: ChannelMask(1), url: url.into() })
}

fn shape(name: &str, h: f32) -> ProjectSource {
    let p = Procedural {
        uid: None,
        url: format!("resource://shelf/{name}/{name}?version=1"),
        tweaks: vec![Tweak { identifier: "hardness".into(), value: TweakValue::Floats(vec![h]) }],
        outputs: vec![],
        inputs: vec![],
        alpha_output: String::new(),
        mono_output: 0,
    };
    ProjectSource::Procedural(ProceduralSource { uid: None, channels: ChannelMask(0), mode: 0, stamp_cycle: 0, stroke_wires: 0, procedural: Some(p) })
}

fn stroke() -> ProjectStroke {
    ProjectStroke {
        uid: None,
        points: vec![StrokeKey { position: [0.5, 0.5], time: 0.0 }],
        pressure: vec![],
        tilt_keys: 0,
        complete: true,
        view_matrix: [1.0; 16],
        projection_matrix: [1.0; 16],
        view: StrokeView::Mesh,
        duplicate_count: 0,
        duplicate_matrix: [0.0; 16],
        random_seed: 1,
        lazy_distance: 0.0,
    }
}

fn paint(alpha: Vec<ProjectSource>, size: f32, opacity: f32) -> ProjectEffect {
    ProjectEffect::Paint(PaintEffect {
        header: header(),
        tool: PaintTool::Brush,
        opacity,
        strokes: vec![stroke()],
        paths: vec![],
        brush_uid: None,
        brush: Some(ProjectBrush { size, flow: 0.5, ..ProjectBrush::default() }),
        stamp_blending: 0,
        colors: vec![bitmap("/Colour?version=c1")],
        alpha,
        stencil: None,
        stencil_matrix: None,
    })
}

/// A paint effect through a stencil, without strokes.
fn stencilled() -> ProjectEffect {
    let ProjectEffect::Paint(mut p) = paint(vec![], 0.1, 1.0) else { panic!("a paint effect") };
    p.strokes.clear();
    p.colors.clear();
    p.stencil = Some(bitmap("resource://starter_assets/Grid?version=s1"));
    ProjectEffect::Paint(p)
}

fn fill(sources: Vec<ProjectSource>) -> ProjectEffect {
    ProjectEffect::Fill(FillEffect { header: header(), sources, projection: 6, filtering: 2, uv_transform: None, placement: FillPlacement::default() })
}

fn document(content: Vec<ProjectEffect>) -> ProjectDocument {
    let layer = ProjectLayer {
        uid: Some(1),
        name: "L".into(),
        kind: LayerKind::Fill,
        visible: true,
        blending: vec![],
        mask: None,
        geometry: None,
        content,
        unhandled: vec![],
    };
    let set = ProjectTextureSet {
        uid: Some(1),
        material_uid: Some(7),
        name: "S".into(),
        resolution_log2: Some([10, 10]),
        channels: vec![],
        shader_model: None,
        layers: vec![layer],
        mesh_maps: vec![],
        uses_uv_tiles: false,
        uv_tile_entries: 0,
        unhandled: vec![],
    };
    ProjectDocument { format_version: 78, texture_sets: vec![set], unhandled: vec![] }
}

#[test]
fn only_what_the_file_itself_holds_is_shareable() {
    let of = Origin::of_url;
    assert_eq!(of("/Scratches?version=ab12"), Some(Origin::Project));
    assert_eq!(of("resource://shelf/Cutlery1?version=c51e.image"), Some(Origin::Shelf("shelf".into())));
    assert_eq!(of("resource://lib.1328-81/noise/noise?version=1"), Some(Origin::Shelf("lib".into())));
    assert_eq!(of("resource://starter_assets/Dirt?version=1"), Some(Origin::Shelf("starter_assets".into())));
    assert_eq!(of("resourcealias:Curvature"), None);
    assert_eq!(of(""), None);
    assert_eq!(of("resource:///x"), None);
    assert!(Origin::Project.shareable());
    assert!(!Origin::Shelf("shelf".into()).shareable() && !Origin::Shelf("x".into()).shareable());
    assert_eq!(Origin::Shelf("x".into()).id(), "shelf");
}

#[test]
fn brushes_images_and_the_environment_are_offered_once_each() {
    let doc = document(vec![
        paint(vec![shape("shape_v2", 0.9)], 0.2, 1.0),
        // the same brush at another size and opacity: one brush, used twice
        paint(vec![shape("shape_v2", 0.9)], 0.05, 0.4),
        paint(vec![shape("shape_v2", 0.1)], 0.2, 1.0),
        // a bitmap alpha: the bitmap is offered, its brush is not
        paint(vec![bitmap("/Scratch?version=a1")], 0.2, 1.0),
        stencilled(),
        fill(vec![bitmap("/Wood?version=w1"), bitmap("/Ambient Occlusion Map from Mesh S?version=b1"), bitmap("/NotStored?version=x")]),
    ]);
    let stored = |url: &str| !url.contains("NotStored");
    let facts = Facts { project: "dir/Thing.file".into(), baked: HashSet::new(), environment: Some("/Sky?version=e1".into()), stored: &stored };
    let offer = Collector::new(&facts).collect(&doc);
    let keys: Vec<(&str, ItemKind, usize, bool)> = offer.items.iter().map(|i| (i.key.as_str(), i.kind, i.uses, i.shareable())).collect();
    assert_eq!(
        keys,
        [
            ("brush:1", ItemKind::Brush, 2, true),
            ("brush:2", ItemKind::Brush, 1, true),
            ("alpha:/Scratch?version=a1", ItemKind::Alpha, 1, true),
            ("stencil:resource://starter_assets/Grid?version=s1", ItemKind::Stencil, 1, false),
            ("texture:/Colour?version=c1", ItemKind::Texture, 4, true),
            ("texture:/Wood?version=w1", ItemKind::Texture, 1, true),
            ("environment:/Sky?version=e1", ItemKind::Environment, 1, true),
        ]
    );
    let ItemSource::Brush(b) = &offer.get("brush:1").unwrap().source else { panic!() };
    assert_eq!(b.tip, BrushTip::Round { hardness: 0.9, exact: true });
    assert_eq!(offer.left_out.len(), 2, "{:?}", offer.left_out);
    assert!(offer.left_out.iter().any(|l| l.contains("bitmap alpha")));
    assert!(offer.left_out.iter().any(|l| l.contains("texture not stored")));
}

#[test]
fn the_older_round_shape_and_no_alpha_are_round_tips_approximated() {
    assert_eq!(tip::round_tip_hardness(&[]).map(|(_, exact)| exact), Some(false));
    let (h, exact) = tip::round_tip_hardness(&[shape("shape", 0.5)]).unwrap();
    assert!(!exact && (h - 0.6).abs() < 1e-6, "{h}");
    assert_eq!(tip::round_tip_hardness(&[shape("shape_v2", 0.5)]), Some((0.5, true)));
    assert_eq!(tip::round_tip_hardness(&[shape("dots", 0.5)]), None);
    assert_eq!(tip::round_tip_hardness(&[bitmap("/a?version=1")]), None);
}

#[test]
fn deep_nesting_is_walked_without_overflowing() {
    let mut content = vec![fill(vec![bitmap("/Deep?version=d")])];
    for _ in 0..200 {
        content = vec![ProjectEffect::Group(texturecraft_import_reader::model::EffectGroup { header: header(), effects: content })];
    }
    let stored = |_: &str| true;
    let facts = Facts { project: "p".into(), baked: HashSet::new(), environment: None, stored: &stored };
    let offer = Collector::new(&facts).collect(&document(content));
    assert!(offer.items.is_empty());
    assert!(offer.left_out.iter().any(|l| l.contains("nested too deeply")));
}
