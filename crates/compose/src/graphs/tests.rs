use super::*;
use texturecraft_core::{GraphValue, text_key};

/// A colour and a grey output; parameter `gain` sets the colour's red.
pub(crate) const COLOUR: &str =
    "format 3\nsize 16\nnode 1 color r=0.25 g=0.5 b=0.75\nnode 2 color r=0.5 g=0.5 b=0.5\noutput colour 1\noutput grey 2\nexpose gain 1 r 0 1\n";

/// Inverts its input 0.
pub(crate) const INVERT: &str = "format 3\nsize 16\nnode 1 input idx=0\nnode 2 invert in=@1\noutput out 2\n";

fn store(texts: &[&str]) -> GraphStore {
    let mut s = GraphStore::default();
    for t in texts {
        s.add_text(t);
    }
    s
}

fn env<'a>(baked: &'a BakedMaps, images: &'a ImageStore) -> RenderEnv<'a> {
    RenderEnv { baked, images, side: 8 }
}

#[test]
fn a_render_keeps_its_outputs_and_is_found_again_without_rendering() {
    let s = store(&[COLOUR]);
    let (baked, images) = (BakedMaps::default(), ImageStore::default());
    let mut g = GraphRef::new(text_key(COLOUR), "colour");
    g.output = Some("colour".into());
    g.keep.insert("grey".into());
    let (key, cooked) = s.cook(&g, &env(&baked, &images), &[]).unwrap();
    assert_eq!(cooked.outputs.keys().collect::<Vec<_>>(), ["colour", "grey"]);
    assert_eq!(cooked.outputs["grey"].comps(), 1, "grey outputs keep one component");
    let v = at_texel(&cooked.outputs["colour"], 3, 3, (8, 8));
    assert!((v[0] - 0.25).abs() < 1e-4 && (v[2] - 0.75).abs() < 1e-4, "{v:?}");
    assert_eq!(s.renders(), 1);
    // the other channel of the same fill reads the other output of the same render
    let mut grey = g.clone();
    grey.output = Some("grey".into());
    grey.keep = ["colour".to_string()].into();
    assert_eq!(s.cook(&grey, &env(&baked, &images), &[]).unwrap().0, key, "the output read is not part of the key");
    assert_eq!(s.renders(), 1);
}

#[test]
fn parameters_seed_and_size_change_the_key_and_the_pixels() {
    let s = store(&[COLOUR]);
    let (baked, images) = (BakedMaps::default(), ImageStore::default());
    let e = env(&baked, &images);
    let g = GraphRef::new(text_key(COLOUR), "colour");
    let base = s.cook_key(&g, &e, &[]).unwrap();
    let mut gain = g.clone();
    gain.params.insert("gain".into(), GraphValue::Float(0.9));
    assert_ne!(s.cook_key(&gain, &e, &[]).unwrap(), base);
    let (_, cooked) = s.cook(&gain, &e, &[]).unwrap();
    assert!((at_texel(&cooked.outputs["colour"], 0, 0, (8, 8))[0] - 0.9).abs() < 1e-4);
    let mut seeded = g.clone();
    seeded.seed = 3;
    assert_ne!(s.cook_key(&seeded, &e, &[]).unwrap(), base);
    assert_ne!(s.cook_key(&g, &RenderEnv { side: 16, ..e }, &[]).unwrap(), base);
    // a parameter the graph does not have renders as if it were not given
    let mut unknown = g.clone();
    unknown.params.insert("nothing".into(), GraphValue::Vector(vec![f32::NAN, 2.0]));
    assert!(s.cook(&unknown, &e, &[]).is_ok());
}

#[test]
fn keys_are_stable_across_stores() {
    let (baked, images) = (BakedMaps::default(), ImageStore::default());
    let g = GraphRef::new(text_key(COLOUR), "colour");
    let a = store(&[COLOUR]).cook_key(&g, &env(&baked, &images), &[]);
    let b = store(&[COLOUR, INVERT]).cook_key(&g, &env(&baked, &images), &[]);
    assert_eq!(a, b);
}

#[test]
fn a_filter_renders_what_it_is_given_and_inputs_take_part_in_the_key() {
    let s = store(&[INVERT]);
    let (baked, images) = (BakedMaps::default(), ImageStore::default());
    let e = env(&baked, &images);
    let mut g = GraphRef::new(text_key(INVERT), "invert");
    g.inputs.insert(0, GraphInput::Stack { channel: None });
    let given = |v: f32| vec![(0u32, Tex::solid(8, 8, [v, v, v, 1.0]))];
    let (k1, c1) = s.cook(&g, &e, &given(0.25)).unwrap();
    let (k2, _) = s.cook(&g, &e, &given(0.5)).unwrap();
    assert_ne!(k1, k2);
    assert!((at_texel(&c1.outputs["out"], 1, 1, (8, 8))[0] - 0.75).abs() < 1e-4);
    // a nested graph feeds an input
    let mut nested = GraphRef::new(text_key(INVERT), "invert");
    let s2 = store(&[INVERT, COLOUR]);
    let mut inner = GraphRef::new(text_key(COLOUR), "colour");
    inner.output = Some("grey".into());
    nested.inputs.insert(0, GraphInput::Nested(Box::new(inner)));
    let (_, c) = s2.cook(&nested, &e, &[]).unwrap();
    assert!((at_texel(&c.outputs["out"], 0, 0, (8, 8))[0] - 0.5).abs() < 1e-4);
    nested.inputs.insert(0, GraphInput::Constant([0.2, 0.2, 0.2]));
    let (_, c) = s2.cook(&nested, &e, &[]).unwrap();
    assert!((at_texel(&c.outputs["out"], 0, 0, (8, 8))[0] - 0.8).abs() < 1e-4);
}

#[test]
fn hostile_references_fail_without_panicking() {
    let s = store(&[COLOUR, "format 3\nnode 1 nonsense_kind\noutput x 1\n", "node 1 invert in=@7\noutput y 1\n"]);
    let (baked, images) = (BakedMaps::default(), ImageStore::default());
    let e = env(&baked, &images);
    assert!(s.cook(&GraphRef::new(text_key("not stored"), "missing"), &e, &[]).is_err());
    assert!(s.cook(&GraphRef::new(text_key("format 3\nnode 1 nonsense_kind\noutput x 1\n"), "bad"), &e, &[]).is_err());
    assert!(s.cook(&GraphRef::new(text_key("node 1 invert in=@7\noutput y 1\n"), "dangling"), &e, &[]).is_err());
    // nesting past the bound
    let mut deep = GraphRef::new(text_key(COLOUR), "colour");
    for _ in 0..(MAX_GRAPH_NESTING + 3) {
        let mut outer = GraphRef::new(text_key(COLOUR), "colour");
        outer.inputs.insert(0, GraphInput::Nested(Box::new(deep)));
        deep = outer;
    }
    assert!(s.cook_key(&deep, &e, &[]).is_none() && s.cook(&deep, &e, &[]).is_err());
    // a huge side is refused by the runtime, not allocated
    assert!(s.cook(&GraphRef::new(text_key(COLOUR), "colour"), &RenderEnv { side: 1 << 20, ..e }, &[]).is_err());
    // cancelled: nothing rendered, nothing remembered
    s.cooks.cancel.cancel();
    let mut other = GraphRef::new(text_key(COLOUR), "colour");
    other.seed = 99;
    assert_eq!(s.cook(&other, &e, &[]).unwrap_err(), "cancelled");
}

#[test]
fn a_use_that_cannot_render_again_keeps_its_last_pixels_flagged() {
    let s = store(&[COLOUR]);
    let (baked, images) = (BakedMaps::default(), ImageStore::default());
    let e = env(&baked, &images);
    let use_ = GraphUse { set: 0, id: LayerId(4), channel: Some(Channel::BaseColor) };
    let g = GraphRef::new(text_key(COLOUR), "colour");
    assert!(s.cook_for(use_.clone(), &g, &e, &[]).is_some());
    assert!(s.problems().is_empty());
    // a project file holding the cook and the use but not the text (a resource that is missing): another value
    // changes the key, there is nothing to render it from, and the last pixels stay, flagged stale
    let loaded = GraphStore::default();
    for (key, cooked) in s.used_cooks() {
        loaded.insert_cooked(key, (*cooked).clone());
    }
    loaded.set_uses(s.uses());
    let mut changed = g.clone();
    changed.seed = 5;
    assert!(loaded.cook_for(use_.clone(), &changed, &e, &[]).is_some());
    let problems = loaded.problems();
    assert!(problems.len() == 1 && problems[0].1.stale, "{problems:?}");
    assert_eq!(loaded.renders(), 1, "one attempt, with no text to render from");
    // with the text (as a project file holds it) the unchanged key is found without a render
    let mut reopened = GraphStore::default();
    reopened.add_text(COLOUR);
    for (key, cooked) in s.used_cooks() {
        reopened.insert_cooked(key, (*cooked).clone());
    }
    assert!(reopened.cook_for(use_.clone(), &g, &e, &[]).is_some() && reopened.problems().is_empty());
    assert_eq!(reopened.renders(), 0);
    // a use with no earlier cook shows nothing, flagged
    let other = GraphUse { id: LayerId(5), ..use_ };
    assert!(loaded.cook_for(other, &changed, &e, &[]).is_none());
    assert_eq!(loaded.problems().iter().filter(|(_, p)| !p.stale).count(), 1);
    loaded.prune(|id| id == LayerId(4));
    assert_eq!(loaded.uses().len(), 1);
    assert_eq!(loaded.used_cooks().len(), 1);
}

#[test]
fn values_cross_the_boundary_both_ways() {
    let v = [0.75, 0.25, 1.0, 1.0];
    assert_eq!(graph_to_channel(ChannelKind::Signed, v, false), [0.5; 3]);
    assert_eq!(graph_to_channel(ChannelKind::Scalar, v, false), [0.75; 3]);
    let lin = graph_to_channel(ChannelKind::Color, [0.5, 0.5, 0.5, 1.0], false);
    assert!((lin[0] - 0.214).abs() < 1e-3);
    assert_eq!(graph_to_channel(ChannelKind::Color, v, true), [0.75, 0.25, 1.0]);
    let n = graph_to_channel(ChannelKind::NormalMap, [0.5, 0.5, 1.0, 1.0], false);
    assert_eq!(n, [0.5, 0.5, 1.0], "a flat normal stays flat");
    for kind in [ChannelKind::Color, ChannelKind::Signed, ChannelKind::Scalar] {
        let there = graph_to_channel(kind, v, false);
        let back = channel_to_graph(kind, there, false);
        let again = graph_to_channel(kind, [back[0], back[1], back[2], 1.0], false);
        assert!(there.iter().zip(again).all(|(a, b)| (a - b).abs() < 1e-5), "{kind:?}");
    }
}

#[test]
fn quantized_cooks_sample_their_texels() {
    let t = Tex { w: 2, h: 1, px: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 0.5, 0.25, 1.0]] };
    let c = Cooked::from_textures([("o".to_string(), t)].into(), BTreeSet::new());
    let img = &c.outputs["o"];
    assert_eq!(img.comps(), 4);
    assert_eq!(at_texel(img, 1, 0, (2, 1))[0], 1.0);
    assert!((sample(img, 0.5, 0.5)[0] - 0.5).abs() < 1e-3, "halfway between the texels");
    assert_eq!(sample(img, f32::NAN, 0.0), [0.0; 4]);
    assert_eq!(c.output(None).map(TileImage::width), Some(2));
}
