//! Every place the file runs a compiled graph, one function per use: fills of content channels (procedural and
//! material fills), mask generators, filters and graphs feeding other graphs' inputs.
//!
//! With the project's graphs decompiled ([`GraphLibrary`], the default), a use becomes a graph source
//! ([`Source::Graph`]) or a graph effect ([`EffectKind::Graph`]) running the project's own graph: its tweaks as parameter
//! values, `$randomseed` as the seed, each channel reading the output the file wires to it (a mask its
//! `monoChannelOutputIndex`), and every image input fed as the file feeds it ([`recipe`]): a constant, another
//! procedural (nested, rendered the same way), a bitmap (an image resource, laid on in texture space), an anchor (for
//! mask generators and filters, read through the reference's levels), a filter's stack below in the channel its
//! `inputs` name, or else the baked mesh map of the input's name, or else the colour its declaration asks for.
//!
//! What cannot be fed, a graph the project does not hold, or graphs kept as placeholders ([`GraphMode::Keep`]) give a
//! placeholder effect (it composites as identity) with the blocker `graph:<name>` (or the input's blocker) that locks
//! the layer. Callers place [`GraphUse::effect`] where the use sits, merge [`GraphUse::status`] into the layer's status
//! and record [`GraphUse::graph`] in the report.
//!
//! A few filters have built-in counterparts. Known simple conversions are selected by their graph identity.
//! Approximate conversions require [`crate::ImportOptions::native_filters`].

use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_core::{Channel, ChannelKind, EffectKind, Filter, GraphInput, GraphRef, Levels, MAX_GRAPH_NESTING, Projection, Source};
use texturecraft_import_reader::blockers::Blocker;
use texturecraft_import_reader::decompile::Decompiled;
use texturecraft_import_reader::model::{ChannelMask, FilterEffect, Procedural, ProceduralSource, ProjectChannel, ProjectSource, ReferenceSource, TweakValue};

use crate::graph_library::{GraphLibrary, LibraryGraph, baked_map_for, tweak_value};
use crate::layers::content::{is_dangling, levels};
use crate::layers::{Anchors, SetInput};
use crate::resources::{MeshMapAlias, mesh_map_alias};
use crate::status::Status;

/// Why graph uses are placeholders when graphs are kept, for the effect's `reason`.
pub const NOT_COMPUTED: &str = "compiled graphs are kept as placeholders";

/// File data: the parameter that is a graph's random seed, and the parameters graphs take from their host, not from a
/// value (they start with this).
const SEED_INPUT: &str = "$randomseed";
const SYSTEM_PREFIX: char = '$';

/// Whether the import runs the project's graphs or keeps them as placeholders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphMode {
    /// Decompile the project's graphs and run them (the default: a placeholder stands for nothing and locks its layer).
    #[default]
    Decompile,
    /// Keep every graph use a locked placeholder (the original's pixels only in its stored result).
    Keep,
}

/// One use of a compiled graph in the imported stack.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphUse {
    /// What stands where the graph is used: a graph effect, or a placeholder.
    pub effect: EffectKind,
    /// Its status: approximated when the graph runs, not computable by its blockers when it cannot.
    pub status: Status,
    /// The graph's name (`?` when the file names none).
    pub graph: String,
}

/// A procedural fill running its graph: one source per channel it writes.
#[derive(Debug, Clone, PartialEq)]
pub struct FillGraph {
    pub sources: BTreeMap<Channel, Source>,
    pub status: Status,
    pub graph: String,
}

fn name_of(p: Option<&Procedural>) -> String {
    p.map_or("?", Procedural::name).to_string()
}

fn placeholder(label: String, graph: String, why: Status) -> GraphUse {
    let reason = match &why {
        Status::NotComputable(b) if !b.is_empty() => b.iter().map(|b| b.text.clone()).collect::<Vec<_>>().join("; "),
        _ => NOT_COMPUTED.to_string(),
    };
    GraphUse { effect: EffectKind::Placeholder { label, reason }, status: why, graph }
}

/// The graph a procedural runs, when the import decompiles graphs and the project holds it; else why not.
fn library_graph<'l>(input: &SetInput<'l>, p: &Procedural, label: &str) -> Result<(&'l GraphLibrary, &'l LibraryGraph), Status> {
    let keep = || Status::not_computable(Blocker::graph(p.name(), label.to_string()));
    let lib = input.graphs.ok_or_else(keep)?;
    let g = lib.of(p).ok_or_else(|| Status::not_computable(Blocker::graph(p.name(), format!("{label}: the graph is not in the project"))))?;
    Ok((lib, g))
}

/// What a recipe may read besides constants, baked maps, images and other graphs.
#[derive(Clone, Copy)]
struct Reads<'s> {
    /// Anchors feeding the graph itself (mask generators and filters read them; fills and nested graphs cannot).
    anchors: bool,
    /// A filter's image inputs fed by the stack, by index.
    stack: &'s BTreeMap<u32, GraphInput>,
}

/// Everything a recipe takes.
struct Cook<'a, 'l> {
    input: &'a SetInput<'l>,
    anchors: &'a Anchors,
    lib: &'l GraphLibrary,
    status: Status,
}

impl Cook<'_, '_> {
    fn block(&mut self, b: Blocker) {
        self.status.and(&Status::not_computable(b));
    }

    /// The reference app's graph `d` run for procedural `p` as a [`GraphRef`]; problems go into `self.status`.
    fn recipe(&mut self, p: &Procedural, g: &LibraryGraph, reads: Reads, depth: usize) -> GraphRef {
        let d = &g.graph;
        let mut r = GraphRef::new(g.key.clone(), p.name());
        for t in &p.tweaks {
            match (t.identifier.as_str(), &t.value) {
                (SEED_INPUT, TweakValue::Int(v)) => r.seed = *v as u32,
                (SEED_INPUT, TweakValue::Floats(v)) => r.seed = v.first().copied().filter(|x| x.is_finite()).map_or(0, |x| x as i64 as u32),
                (id, _) if id.starts_with(SYSTEM_PREFIX) => {}
                (id, value) => {
                    if let Some(v) = tweak_value(value) {
                        r.params.insert(id.to_string(), v);
                    }
                }
            }
        }
        r.colour_outputs = d.colour_outputs.iter().cloned().collect();
        for (k, name) in d.images.iter().enumerate() {
            let k = k as u32;
            if let Some(fed) = reads.stack.get(&k) {
                r.inputs.insert(k, fed.clone());
                continue;
            }
            let source = p.inputs.iter().find(|i| i.identifiers.iter().any(|(id, _)| id == name)).and_then(|i| i.source.as_deref());
            let fed = match source {
                Some(ProjectSource::Uniform(u)) => Some(GraphInput::Constant(u.color.map(|v| if v.is_finite() { v } else { 0.0 }))),
                Some(ProjectSource::Procedural(inner)) => match inner.procedural.as_ref() {
                    Some(i) if depth < MAX_GRAPH_NESTING => match self.lib.of(i) {
                        Some(ig) => {
                            let none = BTreeMap::new();
                            let mut nested = self.recipe(i, ig, Reads { anchors: false, stack: &none }, depth + 1);
                            let o = mono_output(i, &ig.graph);
                            nested.keep.extend(o.clone());
                            nested.output = o;
                            Some(GraphInput::Nested(Box::new(nested)))
                        }
                        None => {
                            self.block(Blocker::graph(i.name(), format!("procedural \"{}\" feeding an input: not in the project", i.name())));
                            None
                        }
                    },
                    Some(i) => {
                        self.block(Blocker::feature("graph-input-depth", format!("procedurals nested too deeply at \"{}\"", i.name())));
                        None
                    }
                    None => None,
                },
                Some(ProjectSource::Bitmap(b)) if !b.url.is_empty() && baked_map_for(name).is_none() => self.bitmap(&b.url),
                Some(ProjectSource::Reference(rs)) if !is_dangling(rs) => match reads.anchors && depth == 0 {
                    true => self.anchor(rs),
                    false => {
                        self.block(Blocker::feature("graph-input-anchor", format!("\"{}\" reads an anchor in input {name}", p.name())));
                        None
                    }
                },
                Some(ProjectSource::Other(raw)) => {
                    self.block(Blocker::class(&raw.class, format!("input {name} of \"{}\"", p.name())));
                    None
                }
                _ => None,
            };
            let fed = fed.or_else(|| baked_map_for(name).map(GraphInput::Baked)).or_else(|| {
                // nothing feeds it: the colour the input declares
                d.image_defaults.get(k as usize).copied().flatten().map(GraphInput::Constant)
            });
            if let Some(f) = fed {
                r.inputs.insert(k, f);
            }
        }
        r
    }

    /// A bitmap feeding an image input: the image as a resource of the imported project (approximated: laid on in
    /// texture space).
    fn bitmap(&mut self, url: &str) -> Option<GraphInput> {
        let url = match mesh_map_alias(url, self.input.mesh_maps) {
            MeshMapAlias::Unassigned(_) => return Some(GraphInput::Constant([0.0; 3])),
            MeshMapAlias::Assigned(u) => u,
            MeshMapAlias::Unknown => {
                self.block(Blocker::feature("mesh-map-alias", "graph input from an unknown mesh map slot"));
                return None;
            }
            MeshMapAlias::NotAlias => url,
        };
        match self.input.images.load(url) {
            Ok(loaded) => {
                self.status.and(&Status::approximated("a bitmap feeding a graph input, laid on in texture space"));
                Some(GraphInput::Image(loaded.name))
            }
            Err(why) => {
                self.block(Blocker::feature("unreadable-bitmap", why));
                None
            }
        }
    }

    /// An anchor feeding an image input, read in the channel the reference's first binding names, through its levels.
    /// An anchor inside a mask gives the mask's grey there, whatever the channel (the compositor records it while it
    /// evaluates the mask).
    fn anchor(&mut self, r: &ReferenceSource) -> Option<GraphInput> {
        let Some(info) = r.anchor_uid.and_then(|u| self.anchors.get(&u)) else {
            self.block(Blocker::feature("missing-anchor", "graph input reads an anchor that is not in the set"));
            return None;
        };
        if !info.status.is_computable() {
            for b in info.status.blockers() {
                self.status.and(&Status::not_computable(b.through("reads an anchor")));
            }
            return None;
        }
        self.status.and(&info.status);
        let channel = r.bindings.first().and_then(|(source, _)| crate::channels::channel_for_bit(*source));
        let (lv, s) = levels(r.levels.as_ref());
        self.status.and(&s);
        let levels = (lv != Levels::default()).then_some(lv);
        Some(GraphInput::Anchor { anchor: info.id, channel, levels })
    }
}

/// The output a grey use of `p` takes: the one `monoChannelOutputIndex` names, else the graph's first.
fn mono_output(p: &Procedural, d: &Decompiled) -> Option<String> {
    let has = |n: &str| d.outputs.iter().any(|(o, _)| o == n);
    let wired = p.outputs.get(p.mono_output as usize).map(|o| o.identifier.clone()).filter(|n| has(n));
    wired.or_else(|| d.outputs.first().map(|(n, _)| n.clone()))
}

/// The output `p` wires to `channel`. A graph whose outputs name no channel at all has its grey output everywhere.
fn output_for(p: &Procedural, d: &Decompiled, channel: ProjectChannel) -> Option<String> {
    let has = |n: &str| d.outputs.iter().any(|(o, _)| o == n);
    let wired = |m: ChannelMask| !m.is_empty() && m.contains(channel);
    if let Some(o) = p.outputs.iter().find(|o| wired(o.channels) && has(&o.identifier)) {
        return Some(o.identifier.clone());
    }
    let no_wiring = p.outputs.iter().all(|o| o.channels.is_empty());
    no_wiring.then(|| mono_output(p, d)).flatten()
}

/// The output that is the coverage of a fill.
fn alpha_output(p: &Procedural, d: &Decompiled) -> Option<String> {
    Some(p.alpha_output.clone()).filter(|a| !a.is_empty() && d.outputs.iter().any(|(o, _)| o == a))
}

/// What running a decompiled graph is: approximated, as a reproduction rather than the reference app's own engine.
fn decompiled(what: &str, name: &str) -> Status {
    Status::approximated(format!("{what} \"{name}\" decompiled"))
}

/// A procedural source of a fill in layer content writing `channels`: one graph source per channel the graph has an
/// output for (sharing one render), or the placeholder that stands for it.
pub fn fill_sources(input: &SetInput, anchors: &Anchors, source: &ProceduralSource, channels: &[Channel]) -> Result<FillGraph, Box<GraphUse>> {
    let graph = name_of(source.procedural.as_ref());
    let what = if source.mode == 1 { "material fill" } else { "procedural fill" };
    let names: Vec<String> = channels.iter().map(|c| input.channels.label(*c)).collect();
    let label = if names.is_empty() { format!("{what} \"{graph}\"") } else { format!("{what} \"{graph}\" ({})", names.join(", ")) };
    let Some(p) = source.procedural.as_ref() else {
        return Err(Box::new(placeholder(label.clone(), graph.clone(), Status::not_computable(Blocker::graph(&graph, label)))));
    };
    let (lib, g) = library_graph(input, p, &label).map_err(|why| Box::new(placeholder(label.clone(), graph.clone(), why)))?;
    let mut cook = Cook { input, anchors, lib, status: decompiled(what, &graph) };
    let none = BTreeMap::new();
    let mut base = cook.recipe(p, g, Reads { anchors: false, stack: &none }, 0);
    if !cook.status.is_computable() {
        return Err(Box::new(placeholder(label, graph, cook.status)));
    }
    let wired: Vec<(Channel, String)> = channels.iter().filter_map(|c| Some((*c, output_for(p, &g.graph, input.channels.project_channel(*c))?))).collect();
    base.alpha_output = alpha_output(p, &g.graph);
    base.keep = wired.iter().map(|(_, o)| o.clone()).chain(base.alpha_output.clone()).collect();
    let sources = wired
        .into_iter()
        .map(|(c, o)| {
            let mut r = base.clone();
            r.output = Some(o);
            (c, Source::Graph(Box::new(r)))
        })
        .collect();
    Ok(FillGraph { sources, status: cook.status, graph })
}

/// A procedural source of a fill in a mask (a mask generator), laid on through `projection`.
pub fn mask_generator(input: &SetInput, anchors: &Anchors, source: &ProceduralSource, projection: Projection) -> GraphUse {
    let graph = name_of(source.procedural.as_ref());
    let label = format!("mask generator \"{graph}\"");
    let Some(p) = source.procedural.as_ref() else { return placeholder(label.clone(), graph.clone(), Status::not_computable(Blocker::graph(&graph, label))) };
    let (lib, g) = match library_graph(input, p, &label) {
        Ok(found) => found,
        Err(why) => return placeholder(label, graph, why),
    };
    let mut cook = Cook { input, anchors, lib, status: decompiled("mask generator", &graph) };
    let none = BTreeMap::new();
    let mut r = cook.recipe(p, g, Reads { anchors: true, stack: &none }, 0);
    if !cook.status.is_computable() {
        return placeholder(label, graph, cook.status);
    }
    r.output = mono_output(p, &g.graph);
    r.keep.extend(r.output.clone());
    GraphUse { effect: EffectKind::Graph { graph: r, projection }, status: cook.status, graph }
}

/// A filter effect, in content (`channels`: those it writes) or in a mask (`None`): the graph reading the stack below
/// it in the inputs the file names (input 0 is the channel it runs on; a filter in a mask reads the mask), writing each
/// of its channels from the output wired to it and leaving the others as they are.
pub fn filter(input: &SetInput, anchors: &Anchors, f: &FilterEffect, channels: Option<&[Channel]>) -> GraphUse {
    let in_mask = channels.is_none();
    let graph = name_of(f.procedural.as_ref());
    let label = if in_mask { format!("mask filter \"{graph}\"") } else { format!("filter \"{graph}\"") };
    let Some(p) = f.procedural.as_ref() else { return placeholder(label.clone(), graph.clone(), Status::not_computable(Blocker::graph(&graph, label))) };
    let (lib, g) = match library_graph(input, p, &label) {
        Ok(found) => found,
        Err(why) => return placeholder(label, graph, why),
    };
    let d = &g.graph;
    let mut cook = Cook { input, anchors, lib, status: decompiled(if in_mask { "mask filter" } else { "filter" }, &graph) };
    let mut stack: BTreeMap<u32, GraphInput> = BTreeMap::new();
    for (k, name) in d.images.iter().enumerate() {
        let read = match f.inputs.iter().find(|(i, _)| i == name).map(|(_, c)| *c) {
            Some(c) => Some(c),
            // a filter with an input it doesn't list reads the stack there, if nothing else feeds it
            None if name == "input" && !p.inputs.iter().any(|i| i.identifiers.iter().any(|(id, _)| id == name)) => Some(ProjectChannel::Bit(0)),
            None => None,
        };
        match read {
            None => {}
            Some(ProjectChannel::Bit(0)) => {
                stack.insert(k as u32, GraphInput::Stack { channel: None });
            }
            Some(_) if in_mask => cook.block(Blocker::feature("graph-filter-channel-in-mask", format!("mask filter \"{graph}\" reads another channel"))),
            Some(c) => {
                if let Some(ours) = input.channels.ours(c) {
                    stack.insert(k as u32, GraphInput::Stack { channel: Some(ours) });
                }
            }
        }
    }
    // an input's `identifierAlpha` (`input_Alpha`): the graph input fed with the coverage of what that input reads
    for (fed, alpha) in &f.input_alphas {
        let Some(k) = d.images.iter().position(|n| n == alpha) else { continue };
        let read = d.images.iter().position(|n| n == fed).and_then(|i| stack.get(&(i as u32)));
        if let Some(GraphInput::Stack { channel }) = read {
            let channel = *channel;
            stack.insert(k as u32, GraphInput::StackAlpha { channel });
        }
    }
    let mut r = cook.recipe(p, g, Reads { anchors: true, stack: &stack }, 0);
    if !cook.status.is_computable() {
        return placeholder(label, graph, cook.status);
    }
    if let Some(channels) = channels {
        for c in channels.iter().copied() {
            let Some(o) = output_for(p, d, input.channels.project_channel(c)) else { continue };
            // the output's `identifierAlpha` (`output_Alpha`): its coverage
            if let Some(a) = p.outputs.iter().find(|x| x.identifier == o).map(|x| x.alpha.clone()).filter(|a| d.outputs.iter().any(|(n, _)| n == a)) {
                r.output_alphas.insert(c, a.clone());
                r.keep.insert(a);
            }
            r.keep.insert(o.clone());
            r.outputs.insert(c, o);
        }
        if r.outputs.is_empty() {
            // no output for any channel it lists: it changes nothing
            let none = EffectKind::Placeholder { label, reason: "the filter's graph has no output for the channels it runs on".into() };
            return GraphUse { effect: none, status: Status::Exact, graph };
        }
    }
    r.output = mono_output(p, d);
    r.keep.extend(r.output.clone());
    if is_standard_scalar_invert(p, &r, channels) {
        let status = Status::approximated("standard scalar invert converted to TextureCraft's native Invert");
        return GraphUse { effect: EffectKind::Filter(Filter::Invert), status, graph };
    }
    GraphUse { effect: EffectKind::Graph { graph: r, projection: Projection::default() }, status: cook.status, graph }
}

fn is_standard_scalar_invert(p: &Procedural, graph: &GraphRef, channels: Option<&[Channel]>) -> bool {
    // This packaged graph applies 1 - x to its own scalar stack and passes its coverage through unmodified.
    const STANDARD_INVERT: &str = "56985cc99137f7944bfdbd8447cb14af";
    graph.key == STANDARD_INVERT
        && tweak(p, "invert").is_none_or(|value| value == 1.0)
        && graph.output.as_deref() == Some("output")
        && graph.inputs.get(&0) == Some(&GraphInput::StackAlpha { channel: None })
        && graph.inputs.get(&1) == Some(&GraphInput::Stack { channel: None })
        && graph.inputs.len() == 2
        && channels.is_none_or(|channels| {
            channels.iter().all(|channel| {
                matches!(channel.kind(), ChannelKind::Scalar | ChannelKind::Signed)
                    && graph.outputs.get(channel).is_some_and(|output| output == "output")
                    && graph.output_alphas.get(channel).is_some_and(|output| output == "output_Alpha")
            })
        })
}

/// The grey output of procedural `p`'s graph rendered at `side` with its tweaks (red, row by row), for a brush tip or a
/// stencil. Its image inputs read what the file feeds them: a uniform's colour, another procedural (its grey output,
/// rendered the same way, nested at most [`GREY_NESTING`] deep), a bitmap (laid on in texture space); the others what
/// their declaration asks for. `None` when the project does not hold the graph or it does not render. Rendered once per
/// cook key: the library keeps the values and every later call with the same graph, values, seed, inputs and side
/// shares them ([`GraphLibrary::shared_grey`]).
pub fn render_grey(lib: &GraphLibrary, p: &Procedural, side: u32, images: &crate::resources::ProjectImages) -> Option<Arc<[f32]>> {
    use texturecraft_compose::graphs::{RenderEnv, at_texel};
    use texturecraft_compose::{BakedMaps, GraphStore, ImageStore};
    let mut store = GraphStore::default();
    let mut bitmaps = ImageStore::default();
    let r = grey_recipe(lib, p, images, &mut store, &mut bitmaps, 0)?;
    let baked = BakedMaps::default();
    let env = RenderEnv { baked: &baked, images: &bitmaps, side: side as usize };
    let render = || {
        let (_, cooked) = store.cook(&r, &env, &[]).ok()?;
        let out = cooked.output(r.output.as_deref())?;
        let n = side as usize;
        Some((0..n * n).map(|i| at_texel(out, (i % n) as u32, (i / n) as u32, (side, side))[0].clamp(0.0, 1.0)).collect::<Arc<[f32]>>())
    };
    match store.cook_key(&r, &env, &[]) {
        Some(key) => lib.shared_grey(format!("{key}/{side}/{}", r.output.as_deref().unwrap_or_default()), render),
        None => render(),
    }
}

/// Deepest nesting of procedurals feeding a brush tip's graph that is rendered (deeper ones read their declaration).
pub const GREY_NESTING: usize = 4;

/// The recipe of `p`'s grey output for [`render_grey`]: its graph's text added to `store`, bitmaps feeding it to
/// `bitmaps` (under their URL).
fn grey_recipe(
    lib: &GraphLibrary,
    p: &Procedural,
    images: &crate::resources::ProjectImages,
    store: &mut texturecraft_compose::GraphStore,
    bitmaps: &mut texturecraft_compose::ImageStore,
    depth: usize,
) -> Option<GraphRef> {
    let g = lib.of(p)?;
    store.texts.insert(g.key.clone(), g.graph.text.clone());
    let mut r = GraphRef::new(g.key.clone(), p.name());
    for t in &p.tweaks {
        match (t.identifier.as_str(), &t.value) {
            (SEED_INPUT, TweakValue::Int(v)) => r.seed = *v as u32,
            (id, _) if id.starts_with(SYSTEM_PREFIX) => {}
            (id, value) => {
                if let Some(v) = tweak_value(value) {
                    r.params.insert(id.to_string(), v);
                }
            }
        }
    }
    for (k, name) in g.graph.images.iter().enumerate() {
        let source = p.inputs.iter().find(|i| i.identifiers.iter().any(|(id, _)| id == name)).and_then(|i| i.source.as_deref());
        let fed = match source {
            Some(ProjectSource::Uniform(u)) => Some(GraphInput::Constant(u.color.map(|v| if v.is_finite() { v } else { 0.0 }))),
            Some(ProjectSource::Procedural(inner)) if depth < GREY_NESTING => inner
                .procedural
                .as_ref()
                .and_then(|i| grey_recipe(lib, i, images, store, bitmaps, depth + 1))
                .map(|nested| GraphInput::Nested(Box::new(nested))),
            Some(ProjectSource::Bitmap(b)) if !b.url.is_empty() => images.peek(&b.url).ok().map(|read| {
                if !bitmaps.has_slot(&b.url) {
                    bitmaps.insert(b.url.clone(), std::sync::Arc::new(read.image.image_res(!read.float)));
                }
                GraphInput::Image(b.url.clone())
            }),
            _ => None,
        };
        let fed = fed.or_else(|| g.graph.image_defaults.get(k).copied().flatten().map(GraphInput::Constant));
        if let Some(f) = fed {
            r.inputs.insert(k as u32, f);
        }
    }
    r.output = mono_output(p, &g.graph);
    if depth > 0 {
        r.keep.extend(r.output.clone());
    }
    Some(r)
}

/// A float tweak of a procedural (its first component).
fn tweak(p: &Procedural, id: &str) -> Option<f32> {
    p.tweaks.iter().find(|t| t.identifier == id).and_then(|t| match &t.value {
        TweakValue::Floats(v) => v.first().copied().filter(|x| x.is_finite()),
        TweakValue::Int(i) => Some(*i as f32),
        _ => None,
    })
}

/// A built-in filter close to a compiled filter graph, with why it is only close; `None` for graphs without one. Sizes
/// convert by first-guess factors (blur), and parameters the file does not store keep their usual defaults.
pub fn native_filter(f: &FilterEffect) -> Option<(Filter, String)> {
    let p = f.procedural.as_ref()?;
    let (filter, note) = match p.name() {
        "invert" => {
            if tweak(p, "invert").is_some_and(|v| v < 0.5) {
                return None;
            }
            (Filter::Invert, "built-in invert")
        }
        "clamp" => (Filter::Clamp { min: tweak(p, "min").unwrap_or(0.0), max: tweak(p, "max").unwrap_or(1.0) }, "built-in clamp"),
        "blur" | "Blur" | "fx_blur" => {
            (Filter::Blur { radius: tweak(p, "intensity").unwrap_or(10.0) * 0.004 * 2048.0 }, "built-in blur (size converted by a first-guess factor)")
        }
        "sharpen" => (Filter::Sharpen { amount: tweak(p, "intensity").unwrap_or(1.0) }, "built-in sharpen"),
        "hsl_perceptive" => {
            let centred = |id: &str, k: f32| tweak(p, id).map_or(0.0, |v| (v - 0.5) * k);
            (
                Filter::Hsl { hue: centred("hue", 1.0), saturation: centred("saturation", 2.0), lightness: centred("luminosity", 2.0) },
                "built-in HSL (perceptive mode left out)",
            )
        }
        _ => return None,
    };
    Some((filter, format!("filter \"{}\" replaced by the {note}", p.name())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::{SetChannels, bit_of};
    use crate::layers::AnchorInfo;
    use crate::options::ImportOptions;
    use crate::resources::ProjectImages;
    use texturecraft_core::{ChannelFormat, GraphValue, LayerId};
    use texturecraft_import_reader::model::{EffectHeader, ProceduralInput, ProceduralOutput, Tweak, UniformSource};

    fn graph(text: &str, images: &[&str], outputs: &[&str]) -> Decompiled {
        Decompiled {
            text: text.into(),
            images: images.iter().map(|s| s.to_string()).collect(),
            image_defaults: images.iter().map(|_| None).collect(),
            outputs: outputs.iter().enumerate().map(|(k, s)| (s.to_string(), k + 1)).collect(),
            warnings: Vec::new(),
            colour_outputs: Vec::new(),
        }
    }

    fn procedural(name: &str) -> Procedural {
        Procedural {
            uid: None,
            url: format!("resource://shelf/{name}/{name}?version=1"),
            tweaks: Vec::new(),
            outputs: Vec::new(),
            inputs: Vec::new(),
            alpha_output: String::new(),
            mono_output: 0,
        }
    }

    fn source(p: Procedural) -> ProceduralSource {
        ProceduralSource { uid: None, channels: ChannelMask::default(), mode: 1, stamp_cycle: 0, stroke_wires: 0, procedural: Some(p) }
    }

    fn output(id: &str, c: Channel) -> ProceduralOutput {
        ProceduralOutput { identifier: id.into(), channels: ChannelMask(bit_of(c)), alpha: String::new() }
    }

    fn input(id: &str, s: ProjectSource) -> ProceduralInput {
        ProceduralInput { identifiers: vec![(id.into(), ProjectChannel::Bit(0))], source: Some(Box::new(s)) }
    }

    fn channels() -> SetChannels {
        let mut c = SetChannels::default();
        c.declare(Channel::BaseColor, ChannelFormat::Srgb8);
        c.declare(Channel::Roughness, ChannelFormat::L8);
        c.declare(Channel::Metallic, ChannelFormat::L8);
        c
    }

    fn library(graphs: Vec<(&str, Decompiled)>) -> GraphLibrary {
        let mut lib = GraphLibrary::default();
        for (name, d) in graphs {
            lib.insert_as(&[name.to_string()], "", d);
        }
        lib
    }

    /// Runs `f` with a set input over `lib` (`None`: graphs kept).
    fn with<R>(lib: Option<&GraphLibrary>, f: impl FnOnce(&SetInput, &Anchors) -> R) -> R {
        let (ch, images, options) = (channels(), ProjectImages::none(), ImportOptions::default());
        let input = SetInput {
            channels: &ch,
            images: &images,
            mesh_maps: &[],
            parts: &[],
            options: &options,
            replay: false,
            graphs: lib,
            geometry: None,
            layers_done: None,
        };
        let mut anchors = Anchors::new();
        anchors.insert(7, AnchorInfo { id: LayerId(70), in_mask: false, status: Status::Exact });
        anchors.insert(8, AnchorInfo { id: LayerId(80), in_mask: true, status: Status::Exact });
        f(&input, &anchors)
    }

    fn graph_of(s: &Source) -> &GraphRef {
        s.graph().unwrap()
    }

    #[test]
    fn each_channel_takes_its_output_with_tweaks_and_seed_sharing_one_render() {
        let mut p = procedural("wood");
        p.outputs = vec![output("basecolor", Channel::BaseColor), output("rough", Channel::Roughness), output("opacity", Channel::Opacity)];
        p.alpha_output = "opacity".into();
        p.tweaks = vec![
            Tweak { identifier: "$randomseed".into(), value: TweakValue::Int(-3) },
            Tweak { identifier: "$outputsize".into(), value: TweakValue::Int2([8, 8]) },
            Tweak { identifier: "tint".into(), value: TweakValue::Floats(vec![0.25, 0.5, 0.75]) },
            Tweak { identifier: "on".into(), value: TweakValue::Int(1) },
            Tweak { identifier: "label".into(), value: TweakValue::Text("x".into()) },
        ];
        let lib = library(vec![("wood", graph("wood text", &[], &["basecolor", "rough", "opacity"]))]);
        let fill = with(Some(&lib), |i, a| fill_sources(i, a, &source(p), &[Channel::BaseColor, Channel::Roughness, Channel::Metallic])).unwrap();
        assert_eq!(fill.sources.keys().copied().collect::<Vec<_>>(), [Channel::BaseColor, Channel::Roughness], "no output: metallic is not written");
        let rough = graph_of(&fill.sources[&Channel::Roughness]);
        assert_eq!(rough.key, texturecraft_core::text_key("wood text"));
        assert_eq!(rough.output.as_deref(), Some("rough"));
        assert_eq!(rough.alpha_output.as_deref(), Some("opacity"));
        assert_eq!(rough.keep.iter().map(String::as_str).collect::<Vec<_>>(), ["basecolor", "opacity", "rough"]);
        assert_eq!(rough.seed, (-3i32) as u32);
        assert_eq!(rough.params.get("tint"), Some(&GraphValue::Vector(vec![0.25, 0.5, 0.75])));
        assert_eq!(rough.params.get("on"), Some(&GraphValue::Int(1)));
        assert!(!rough.params.contains_key("$outputsize"), "the size is the texture set's");
        assert_eq!(rough.params.get("label"), Some(&GraphValue::Text("x".into())), "text tweaks pass through");
        let base = graph_of(&fill.sources[&Channel::BaseColor]);
        assert_eq!(GraphRef { output: None, ..base.clone() }, GraphRef { output: None, ..rough.clone() }, "the channels share one render");
        assert!(fill.status.is_computable() && !fill.status.is_exact());
    }

    #[test]
    fn image_inputs_take_the_projects_source_then_the_map_of_their_name_then_their_declared_colour() {
        let mut d = graph("wear", &["ambient_occlusion", "micro_height", "texture", "normaWS", "detail"], &["output"]);
        d.image_defaults[1] = Some([0.5, 0.5, 0.5]);
        let mut inner = procedural("noise");
        inner.tweaks = vec![Tweak { identifier: "scale".into(), value: TweakValue::Floats(vec![4.0]) }];
        let uniform = ProjectSource::Uniform(UniformSource { uid: None, channels: ChannelMask::default(), color: [0.0, 0.25, f32::NAN], opacity: 1.0 });
        let mut p = procedural("wear");
        p.inputs = vec![input("texture", uniform), input("detail", ProjectSource::Procedural(source(inner)))];
        let lib = library(vec![("wear", d), ("noise", graph("noise", &[], &["output"]))]);
        let g = with(Some(&lib), |i, a| mask_generator(i, a, &source(p), Projection::default()));
        let EffectKind::Graph { graph: r, .. } = &g.effect else { panic!("a graph effect: {g:?}") };
        assert_eq!(r.inputs[&0], GraphInput::Baked(texturecraft_core::BakedMap::AmbientOcclusion));
        assert_eq!(r.inputs[&1], GraphInput::Constant([0.5; 3]), "nothing fed and no map of that name: the declared colour");
        assert_eq!(r.inputs[&2], GraphInput::Constant([0.0, 0.25, 0.0]));
        assert_eq!(r.inputs[&3], GraphInput::Baked(texturecraft_core::BakedMap::WorldNormal), "the misspelt channel tag some graphs use");
        let GraphInput::Nested(n) = &r.inputs[&4] else { panic!("nested") };
        assert_eq!((n.name.as_str(), n.output.as_deref()), ("noise", Some("output")));
        assert_eq!(n.params.get("scale"), Some(&GraphValue::Float(4.0)));
        assert_eq!(r.output.as_deref(), Some("output"));
    }

    #[test]
    fn anchors_feed_generators_and_filters_but_block_fills() {
        let anchor = ReferenceSource {
            uid: None,
            channels: ChannelMask::default(),
            anchor_uid: Some(7),
            anchor_label: String::new(),
            levels: None,
            bindings: vec![(bit_of(Channel::Height), 0)],
            mode: 2,
        };
        let mut p = procedural("ao");
        p.inputs = vec![input("micro_height", ProjectSource::Reference(anchor))];
        let lib = library(vec![("ao", graph("ao", &["micro_height"], &["output"]))]);
        let g = with(Some(&lib), |i, a| mask_generator(i, a, &source(p.clone()), Projection::default()));
        let EffectKind::Graph { graph: r, .. } = &g.effect else { panic!("a graph effect") };
        assert_eq!(r.inputs[&0], GraphInput::Anchor { anchor: LayerId(70), channel: Some(Channel::Height), levels: None });
        let fill = with(Some(&lib), |i, a| fill_sources(i, a, &source(p), &[Channel::Roughness])).unwrap_err();
        assert_eq!(fill.status.blockers()[0].key.to_string(), "feature:graph-input-anchor");
        assert!(matches!(fill.effect, EffectKind::Placeholder { .. }));
    }

    #[test]
    fn an_anchor_inside_a_mask_feeds_a_graph_input() {
        let anchor = ReferenceSource {
            uid: None,
            channels: ChannelMask::default(),
            anchor_uid: Some(8),
            anchor_label: String::new(),
            levels: None,
            bindings: vec![(bit_of(Channel::Height), 0)],
            mode: 2,
        };
        let mut p = procedural("mask_editor");
        p.inputs = vec![input("mask", ProjectSource::Reference(anchor))];
        let lib = library(vec![("mask_editor", graph("mask_editor", &["mask"], &["output"]))]);
        let g = with(Some(&lib), |i, a| mask_generator(i, a, &source(p), Projection::default()));
        let EffectKind::Graph { graph: r, .. } = &g.effect else { panic!("a graph effect, not locked: {g:?}") };
        assert_eq!(r.inputs[&0], GraphInput::Anchor { anchor: LayerId(80), channel: Some(Channel::Height), levels: None });
        assert!(g.status.is_computable());
    }

    #[test]
    fn filters_read_the_stack_and_channels_and_write_by_channel() {
        let mut p = procedural("finish");
        p.outputs = vec![output("rough_out", Channel::Roughness), output("metal_out", Channel::Metallic)];
        p.outputs[0].alpha = "rough_alpha".into();
        let f = FilterEffect {
            header: EffectHeader::default(),
            procedural: Some(p),
            inputs: vec![("input".into(), ProjectChannel::Bit(0)), ("rough".into(), ProjectChannel::Roughness), ("gloss".into(), ProjectChannel::Glossiness)],
            input_alphas: vec![("input".into(), "input_Alpha".into())],
        };
        let lib = library(vec![("finish", graph("finish", &["input", "rough", "gloss", "input_Alpha"], &["rough_out", "metal_out", "rough_alpha"]))]);
        let g = with(Some(&lib), |i, a| filter(i, a, &f, Some(&[Channel::Roughness, Channel::Metallic])));
        let EffectKind::Graph { graph: r, .. } = &g.effect else { panic!("a graph effect: {g:?}") };
        assert!(r.reads_stack());
        assert_eq!(r.inputs[&0], GraphInput::Stack { channel: None });
        assert_eq!(r.inputs[&1], GraphInput::Stack { channel: Some(Channel::Roughness) });
        assert!(!r.inputs.contains_key(&2), "a channel the set doesn't have");
        assert_eq!(r.inputs[&3], GraphInput::StackAlpha { channel: None });
        assert_eq!(r.outputs[&Channel::Roughness], "rough_out");
        assert_eq!(r.outputs[&Channel::Metallic], "metal_out");
        assert_eq!(r.output_alphas[&Channel::Roughness], "rough_alpha");
        // in a mask a filter reading another channel cannot run
        let blocked = with(Some(&lib), |i, a| filter(i, a, &f, None));
        assert_eq!(blocked.status.blockers()[0].key.to_string(), "feature:graph-filter-channel-in-mask");
    }

    #[test]
    fn without_the_graph_or_with_graphs_kept_the_use_is_a_locked_placeholder() {
        let p = procedural("mg_dirt");
        let kept = with(None, |i, a| mask_generator(i, a, &source(p.clone()), Projection::default()));
        assert!(matches!(kept.effect, EffectKind::Placeholder { .. }));
        assert_eq!(kept.status.lock_reasons()[0].key, "graph:mg_dirt");
        let lib = library(vec![("other", graph("o", &[], &["o"]))]);
        let missing = with(Some(&lib), |i, a| fill_sources(i, a, &source(p), &[Channel::BaseColor])).unwrap_err();
        assert_eq!(missing.graph, "mg_dirt");
        assert_eq!(missing.status.lock_reasons()[0].key, "graph:mg_dirt");
        let none = ProceduralSource { procedural: None, ..source(procedural("x")) };
        assert_eq!(with(Some(&lib), |i, a| mask_generator(i, a, &none, Projection::default())).graph, "?");
    }

    #[test]
    fn native_filters_follow_their_switches() {
        let with_tweaks = |name: &str, tweaks| FilterEffect {
            header: EffectHeader::default(),
            procedural: Some(Procedural { tweaks, ..procedural(name) }),
            inputs: vec![],
            input_alphas: vec![],
        };
        assert_eq!(native_filter(&with_tweaks("invert", vec![])).map(|f| f.0), Some(Filter::Invert));
        let off = Tweak { identifier: "invert".into(), value: TweakValue::Int(0) };
        assert!(native_filter(&with_tweaks("invert", vec![off])).is_none(), "a switched-off invert changes nothing");
    }
}
