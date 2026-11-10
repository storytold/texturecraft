//! Graph sources and graph effects ([`texturecraft_core::GraphRef`]) evaluated through the graph runtime
//! (`texturecraft-graph`), and their cooked outputs.
//!
//! A document's [`GraphStore`] holds the graph texts by key and the cooked outputs. A cook is one render of a graph for
//! one texture set: every output it keeps, as 16-bit tiles like painted pixels ([`Cooked`]). Cooks are keyed by
//! everything that decides their pixels ([`GraphStore::cook_key`]), so a cook found under its key is used as it is,
//! with no render, and a reference whose key changed (another parameter value, a re-baked mesh map, an evaluator change
//! in a node kind the graph uses) is rendered again alone. A project file stores the cooks it uses
//! (`texturecraft-formats`), so a converted project opens without rendering its graphs.
//!
//! Renders run their parallel work on a pool of their own: the compositor's threads ask for cooks while they composite
//! in parallel, and a thread of its pool must never wait for a render running on its own pool ([`once`]). Values reach
//! the compositor at the boundary ([`graph_to_channel`], [`channel_to_graph`]): a graph's images hold display values
//! (colour sRGB, height 0..1 around 0.5, normals with green pointing down), TextureCraft's channels linear colour,
//! signed height and normals with green up.

pub mod once;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use texturecraft_core::{BakedMap, Channel, ChannelKind, GraphInput, GraphRef, LayerId, MAX_GRAPH_NESTING, StableHash};
use texturecraft_graph::context::{Cancel, EvalContext, Files};
use texturecraft_graph::graph::Graph;
pub use texturecraft_graph::graph::Graph as RuntimeGraph;
use texturecraft_graph::rayon::ThreadPool;
pub use texturecraft_graph::tex::Tex;
use texturecraft_texel::{TILE, TileImage, from_u16, linear_to_srgb, srgb_to_linear, to_u16};

use crate::store::{BakedMaps, ImageStore};
use once::Slots;

/// Largest side a graph is rendered at: renders of larger texture sets are sampled up. A guard of ours (memory and
/// time; a decompiled generator holds gigabytes at 4K), not a measured rule.
pub const MAX_SIDE: usize = 2048;

/// Parsed graphs kept for re-rendering with other values.
const PARSED_KEPT: usize = 32;

/// One use of a graph in a document: the texture set, the layer (a fill's source) or effect running it and the channel
/// it is read for (`None`: a mask or a nested read). The cook each use read last is remembered (and saved), so that a
/// use whose key changed and cannot be rendered again keeps its last pixels.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct GraphUse {
    pub set: usize,
    pub id: LayerId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<Channel>,
}

/// One render of a graph: its kept outputs as 16-bit tiles (one component for grey outputs, else RGBA), and which of
/// them the graph declares linear (`meta working`).
#[derive(Clone, Debug, PartialEq)]
pub struct Cooked {
    pub outputs: BTreeMap<String, TileImage>,
    pub linear: BTreeSet<String>,
}

impl Cooked {
    /// Every output stored as it is rendered: the same data whether rendered here or loaded from a file.
    pub fn from_textures(outs: BTreeMap<String, Tex>, linear: BTreeSet<String>) -> Cooked {
        let outputs = outs.into_iter().map(|(name, t)| (name, quantize(&t))).collect();
        Cooked { outputs, linear }
    }

    pub fn bytes(&self) -> usize {
        self.outputs.values().map(TileImage::bytes).sum()
    }

    /// Output `name` (`None`: the first).
    pub fn output(&self, name: Option<&str>) -> Option<&TileImage> {
        match name {
            Some(n) => self.outputs.get(n),
            None => self.outputs.values().next(),
        }
    }

    /// Whether output `name` holds linear colour.
    pub fn is_linear(&self, name: Option<&str>) -> bool {
        let first = || self.outputs.keys().next().map(String::as_str);
        name.or_else(first).is_some_and(|n| self.linear.contains(n))
    }
}

/// A float image as tiles of 16-bit values (0..1; values outside are clamped). Grey opaque images keep one component.
fn quantize(t: &Tex) -> TileImage {
    let grey = t.px.iter().all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 1.0);
    let comps = if grey { 1 } else { 4 };
    let (w, h) = (t.w as u32, t.h as u32);
    let mut img = TileImage::new(w, h, comps);
    for ty in 0..img.tiles_y() {
        for tx in 0..img.tiles_x() {
            let mut data = vec![0u16; TILE as usize * TILE as usize * comps];
            for ly in 0..TILE {
                for lx in 0..TILE {
                    let (x, y) = (tx * TILE + lx, ty * TILE + ly);
                    if x >= w || y >= h {
                        continue;
                    }
                    let p = t.px.get((y * w + x) as usize).copied().unwrap_or([0.0; 4]);
                    let o = (ly * TILE + lx) as usize * comps;
                    for (k, slot) in data.iter_mut().skip(o).take(comps).enumerate() {
                        *slot = to_u16(p.get(k).copied().unwrap_or(0.0));
                    }
                }
            }
            img.set_tile(tx, ty, Some(Arc::new(data)));
        }
    }
    img
}

/// Texel (x, y) of an image as RGBA (wrapping around).
fn texel(img: &TileImage, x: i64, y: i64) -> [f32; 4] {
    let (w, h) = (i64::from(img.width().max(1)), i64::from(img.height().max(1)));
    let v = img.get(x.rem_euclid(w) as u32, y.rem_euclid(h) as u32);
    match img.comps() {
        1 => {
            let g = from_u16(v[0]);
            [g, g, g, 1.0]
        }
        _ => v.map(from_u16),
    }
}

/// Bilinear sample with wrap-around at texture coordinates (v down).
pub fn sample(img: &TileImage, u: f32, v: f32) -> [f32; 4] {
    if !u.is_finite() || !v.is_finite() {
        return [0.0; 4];
    }
    let fx = u * img.width() as f32 - 0.5;
    let fy = v * img.height() as f32 - 0.5;
    let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
    let (a, b, c, d) = (texel(img, x0, y0), texel(img, x0 + 1, y0), texel(img, x0, y0 + 1), texel(img, x0 + 1, y0 + 1));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        top + (bot - top) * ty
    })
}

/// The value at texel (x, y) of an image covering `size`: the very texel when the sizes agree, else sampled.
pub fn at_texel(img: &TileImage, x: u32, y: u32, size: (u32, u32)) -> [f32; 4] {
    if (img.width(), img.height()) == size {
        return texel(img, i64::from(x), i64::from(y));
    }
    sample(img, (x as f32 + 0.5) / size.0.max(1) as f32, (y as f32 + 0.5) / size.1.max(1) as f32)
}

/// A graph's value as channel `kind` holds it. Colour: the graph's display (sRGB) values made linear, unless the output
/// is declared linear. Height: the graph's 0..1 over the signed −1..1, 0.5 the flat surface. Normal: green flipped (a
/// graph's normals point green down) and blue rebuilt from red and green as a unit normal's.
pub fn graph_to_channel(kind: ChannelKind, v: [f32; 4], linear: bool) -> [f32; 3] {
    match kind {
        ChannelKind::Color if linear => [v[0], v[1], v[2]],
        ChannelKind::Color => [srgb_to_linear(v[0]), srgb_to_linear(v[1]), srgb_to_linear(v[2])],
        ChannelKind::Signed => [2.0 * v[0] - 1.0; 3],
        ChannelKind::NormalMap => {
            let (x, y) = (2.0 * v[0] - 1.0, 2.0 * (1.0 - v[1]) - 1.0);
            [v[0], 1.0 - v[1], 0.5 + 0.5 * (1.0 - x * x - y * y).max(0.0).sqrt()]
        }
        ChannelKind::Scalar => [v[0]; 3],
    }
}

/// [`graph_to_channel`] undone: a channel value as a graph's input reads it (`linear`: the input is declared linear).
pub fn channel_to_graph(kind: ChannelKind, v: [f32; 3], linear: bool) -> [f32; 3] {
    match kind {
        ChannelKind::Color if linear => v,
        ChannelKind::Color => v.map(linear_to_srgb),
        ChannelKind::Signed => [(v[0] + 1.0) * 0.5; 3],
        ChannelKind::NormalMap => [v[0], 1.0 - v[1], v[2]],
        ChannelKind::Scalar => [v[0]; 3],
    }
}

/// What a render reads of the texture set besides the graph.
#[derive(Clone, Copy)]
pub struct RenderEnv<'a> {
    pub baked: &'a BakedMaps,
    pub images: &'a ImageStore,
    /// The side the graph is rendered at.
    pub side: usize,
}

impl RenderEnv<'_> {
    /// The render side for a texture set of `size`.
    pub fn side_for(size: (u32, u32)) -> usize {
        (size.0.max(size.1) as usize).clamp(1, MAX_SIDE)
    }
}

/// What went wrong with one use, for `project.inspect` and the import report.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct UseProblem {
    pub graph: String,
    /// True when the use shows the pixels it was cooked with before (its key changed and it could not be rendered
    /// again); false when it shows nothing.
    pub stale: bool,
    pub reason: String,
}

/// Cooked outputs and what is known about them, shared by every snapshot of a document (a cook's key decides its
/// pixels, so snapshots never disagree about one).
#[derive(Default)]
pub struct CookCache {
    cooks: Slots<String, Result<Arc<Cooked>, String>>,
    /// The cook each use read last.
    uses: Mutex<BTreeMap<GraphUse, String>>,
    problems: Mutex<BTreeMap<GraphUse, UseProblem>>,
    renders: AtomicUsize,
    parsed: Mutex<HashMap<String, Arc<Graph>>>,
    kinds: Mutex<HashMap<String, u128>>,
    /// Fingerprints of baked maps and image resources by the address and length of their data.
    fingerprints: Mutex<HashMap<(usize, usize), u128>>,
    pool: OnceLock<Option<Arc<ThreadPool>>>,
    /// Stops renders running for this document.
    pub cancel: Cancel,
}

impl std::fmt::Debug for CookCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookCache").field("cooks", &self.cooks.values().len()).field("renders", &self.renders.load(Ordering::Relaxed)).finish_non_exhaustive()
    }
}

/// A document's graphs: texts by key and the cooks (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct GraphStore {
    /// Graph texts by text key ([`texturecraft_core::text_key`]).
    pub texts: BTreeMap<String, Arc<str>>,
    pub cooks: Arc<CookCache>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The kinds of the nodes a graph text has (the third word of its `node` lines, nested graphs included).
fn node_kinds(text: &str) -> BTreeSet<&str> {
    text.lines().filter_map(|l| l.trim_start().strip_prefix("node ")).filter_map(|rest| rest.split_whitespace().nth(1)).collect()
}

impl GraphStore {
    /// A store with no graphs, for composites of documents without any.
    pub fn empty() -> &'static GraphStore {
        static EMPTY: OnceLock<GraphStore> = OnceLock::new();
        EMPTY.get_or_init(GraphStore::default)
    }

    /// Adds a graph text, under its key.
    pub fn add_text(&mut self, text: &str) -> String {
        let key = texturecraft_core::text_key(text);
        self.texts.entry(key.clone()).or_insert_with(|| Arc::from(text));
        key
    }

    /// Renders made since the store was made (by every snapshot sharing its cooks).
    pub fn renders(&self) -> usize {
        self.cooks.renders.load(Ordering::Relaxed)
    }

    /// Uses whose graph showed nothing, or showed stale pixels, the last time they were composited.
    pub fn problems(&self) -> Vec<(GraphUse, UseProblem)> {
        lock(&self.cooks.problems).iter().map(|(u, p)| (u.clone(), p.clone())).collect()
    }

    /// The cook each use read last.
    pub fn uses(&self) -> BTreeMap<GraphUse, String> {
        lock(&self.cooks.uses).clone()
    }

    /// Sets what each use read last (from a project file).
    pub fn set_uses(&self, uses: BTreeMap<GraphUse, String>) {
        *lock(&self.cooks.uses) = uses;
    }

    /// The cook under `key`, if one is held (rendered or loaded).
    pub fn cooked(&self, key: &str) -> Option<Arc<Cooked>> {
        self.cooks.cooks.get(&key.to_string()).and_then(Result::ok)
    }

    /// Puts a cook in under `key` (loading a project file).
    pub fn insert_cooked(&self, key: String, cooked: Cooked) {
        self.cooks.cooks.insert(key, Ok(Arc::new(cooked)));
    }

    /// The cooks the uses read, by key (what a project file saves), sorted.
    pub fn used_cooks(&self) -> Vec<(String, Arc<Cooked>)> {
        let keys: BTreeSet<String> = self.uses().into_values().collect();
        keys.into_iter().filter_map(|k| self.cooked(&k).map(|c| (k, c))).collect()
    }

    /// Forgets uses of layers and effects that no longer exist, and cooks no use reads (failed renders are kept, so that
    /// they are not tried again on every composite).
    pub fn prune(&self, alive: impl Fn(LayerId) -> bool) {
        lock(&self.cooks.uses).retain(|u, _| alive(u.id));
        lock(&self.cooks.problems).retain(|u, _| alive(u.id));
        let used: BTreeSet<String> = self.uses().into_values().collect();
        let failed: BTreeSet<String> = self.cooks.cooks.values().into_iter().filter(|(_, v)| v.is_err()).map(|(k, _)| k).collect();
        self.cooks.cooks.retain(|k| used.contains(k) || failed.contains(k));
    }

    fn pool(&self) -> Option<Arc<ThreadPool>> {
        self.cooks.pool.get_or_init(|| texturecraft_graph::context::own_pool("graph")).clone()
    }

    /// A digest of the evaluator versions of the node kinds text `key` uses ([`texturecraft_graph::versions`]).
    fn kind_digest(&self, key: &str) -> u128 {
        if let Some(d) = lock(&self.cooks.kinds).get(key) {
            return *d;
        }
        let mut h = StableHash::new();
        if let Some(text) = self.texts.get(key) {
            for kind in node_kinds(text) {
                h.str(kind).str(texturecraft_graph::versions::of_kind(kind));
            }
        }
        let d = h.finish();
        lock(&self.cooks.kinds).insert(key.to_string(), d);
        d
    }

    /// A fingerprint of float data, by its address and length while it lives (baked maps are replaced, not changed in
    /// place, when they are baked again).
    fn fingerprint_floats(&self, data: &[f32]) -> u128 {
        let at = (data.as_ptr() as usize, data.len());
        if let Some(f) = lock(&self.cooks.fingerprints).get(&at) {
            return *f;
        }
        let mut h = StableHash::new();
        for v in data {
            h.f32(*v);
        }
        let f = h.finish();
        lock(&self.cooks.fingerprints).insert(at, f);
        f
    }

    fn fingerprint_pixels(&self, data: &[[f32; 4]]) -> u128 {
        self.fingerprint_floats(data.as_flattened())
    }

    /// Hashes what decides `g`'s render besides the pixels a filter is given: the text by its key, the evaluator
    /// versions of the node kinds it uses, the parameters, seed and rendered outputs, and what feeds each input (baked
    /// maps and images by their contents, nested graphs by the same, recursively). False when the nesting is too deep.
    fn digest(&self, h: &mut StableHash, g: &GraphRef, env: &RenderEnv, depth: usize) -> bool {
        if depth > MAX_GRAPH_NESTING {
            return false;
        }
        h.str(&g.key).bytes(&self.kind_digest(&g.key).to_le_bytes());
        h.u64(g.params.len() as u64);
        for (name, value) in &g.params {
            h.str(name);
            let lanes = value.lanes();
            h.u64(lanes.len() as u64);
            lanes.iter().for_each(|v| {
                h.f32(*v);
            });
        }
        h.u64(u64::from(g.seed));
        let rendered = g.rendered();
        h.u64(rendered.len() as u64);
        rendered.iter().for_each(|o| {
            h.str(o);
        });
        for (k, input) in &g.inputs {
            h.u64(u64::from(*k));
            match input {
                GraphInput::Baked(m) => {
                    h.str("baked").str(m.id());
                    let fp = env.baked.maps.get(m).map_or(0, |d| self.fingerprint_floats(d));
                    h.u64(u64::from(env.baked.resolution)).bytes(&fp.to_le_bytes());
                }
                GraphInput::Constant(c) => {
                    h.str("constant").f32(c[0]).f32(c[1]).f32(c[2]);
                }
                GraphInput::Image(name) => {
                    h.str("image").str(name);
                    let fp = env.images.get(name).map_or(0, |img| self.fingerprint_pixels(&img.rgba));
                    h.bytes(&fp.to_le_bytes());
                }
                GraphInput::Nested(inner) => {
                    h.str("nested").str(inner.output.as_deref().unwrap_or(""));
                    if !self.digest(h, inner, env, depth + 1) {
                        return false;
                    }
                }
                GraphInput::Stack { channel } => {
                    h.str("stack").str(channel.map_or("", Channel::id));
                }
                GraphInput::StackAlpha { channel } => {
                    h.str("stack-alpha").str(channel.map_or("", Channel::id));
                }
                GraphInput::Anchor { anchor, channel, levels } => {
                    h.str("anchor").u64(anchor.0).str(channel.map_or("", Channel::id));
                    if let Some(l) = levels {
                        h.f32(l.in_low).f32(l.in_high).f32(l.gamma).f32(l.out_low).f32(l.out_high);
                    }
                }
            }
        }
        true
    }

    /// The key of `g`'s cook at `env.side`, given the images a filter reads (`given`, by input index). It holds the
    /// graph runtime's [`texturecraft_graph::versions::EPOCH`] and the versions of the node kinds the graph uses, but
    /// not [`texturecraft_graph::versions::TOOLCHAIN`]: renders are the same bits on every machine and with every
    /// compiler (the runtime takes its maths from one library), so a compiler update must not throw away every cook
    /// stored in every project. `None` for graphs nested too deeply.
    pub fn cook_key(&self, g: &GraphRef, env: &RenderEnv, given: &[(u32, Tex)]) -> Option<String> {
        let mut h = StableHash::new();
        h.str("cook").u64(u64::from(texturecraft_graph::versions::EPOCH)).u64(env.side as u64);
        if !self.digest(&mut h, g, env, 0) {
            return None;
        }
        for (k, t) in given {
            h.u64(u64::from(*k)).u64(t.w as u64).u64(t.h as u64);
            for p in &t.px {
                for v in p {
                    // what the graph is given, at the precision a cook keeps: the same stack read on another machine
                    // finds the same cook
                    h.bytes(&to_u16(*v).to_le_bytes());
                }
            }
        }
        Some(h.key())
    }

    /// The image inputs graph `key` declares linear (`input … working=1`): they take a colour channel's values as they
    /// are, the others as display (sRGB) values.
    pub fn working_inputs(&self, key: &str) -> BTreeSet<u32> {
        let Ok(g) = self.parsed(key) else { return BTreeSet::new() };
        g.nodes
            .values()
            .filter(|n| n.kind == "input" && n.nums.get("working").is_some_and(|w| *w > 0.5))
            .map(|n| n.nums.get("idx").copied().unwrap_or(0.0) as u32)
            .collect()
    }

    fn parsed(&self, key: &str) -> Result<Arc<Graph>, String> {
        if let Some(g) = lock(&self.cooks.parsed).get(key) {
            return Ok(g.clone());
        }
        let text = self.texts.get(key).ok_or_else(|| format!("the text of graph {key} is not in the project"))?;
        let g = Arc::new(Graph::parse(text).map_err(|e| e.to_string())?);
        let mut parsed = lock(&self.cooks.parsed);
        if parsed.len() >= PARSED_KEPT {
            parsed.clear();
        }
        parsed.insert(key.to_string(), g.clone());
        Ok(g)
    }

    /// The cook of `g` (no stack inputs, or with them in `given`), from the cache or rendered now. Errors say why
    /// there is none.
    pub fn cook(&self, g: &GraphRef, env: &RenderEnv, given: &[(u32, Tex)]) -> Result<(String, Arc<Cooked>), String> {
        self.cook_at(g, env, given, 0)
    }

    fn cook_at(&self, g: &GraphRef, env: &RenderEnv, given: &[(u32, Tex)], depth: usize) -> Result<(String, Arc<Cooked>), String> {
        let key = self.cook_key(g, env, given).ok_or_else(|| format!("graphs nested deeper than {MAX_GRAPH_NESTING}"))?;
        let got = self.cooks.cooks.get_or_fill_own_pool(&key, || {
            if self.cooks.cancel.is_cancelled() {
                return Err("cancelled".to_string());
            }
            self.cooks.renders.fetch_add(1, Ordering::Relaxed);
            self.render(g, env, given, depth).map(Arc::new)
        });
        // a cancelled render is not remembered as a failure
        if matches!(&got, Err(e) if e == "cancelled") {
            self.cooks.cooks.remove(&key);
        }
        got.map(|c| (key, c))
    }

    /// The cook `use_` reads for `g` (see [`GraphStore::cook`]), remembered as the use's; when there is none now, the one
    /// the use read last, flagged stale. `None` when there is neither.
    pub fn cook_for(&self, use_: GraphUse, g: &GraphRef, env: &RenderEnv, given: &[(u32, Tex)]) -> Option<Arc<Cooked>> {
        match self.cook(g, env, given) {
            Ok((key, cooked)) => {
                let mut uses = lock(&self.cooks.uses);
                if uses.get(&use_) != Some(&key) {
                    uses.insert(use_.clone(), key);
                }
                drop(uses);
                let mut problems = lock(&self.cooks.problems);
                if problems.contains_key(&use_) {
                    problems.remove(&use_);
                }
                Some(cooked)
            }
            Err(reason) => {
                let last = lock(&self.cooks.uses).get(&use_).cloned().and_then(|k| self.cooked(&k));
                let problem = UseProblem { graph: g.name.clone(), stale: last.is_some(), reason };
                let mut problems = lock(&self.cooks.problems);
                if problems.get(&use_) != Some(&problem) {
                    problems.insert(use_, problem);
                }
                last
            }
        }
    }

    /// Renders `g` (see the module docs).
    fn render(&self, g: &GraphRef, env: &RenderEnv, given: &[(u32, Tex)], depth: usize) -> Result<Cooked, String> {
        let side = env.side;
        let mut graph = (*self.parsed(&g.key)?).clone();
        apply_values(&mut graph, &g.params);
        graph.seed_offset = g.seed;
        let have = graph.output_idents();
        let mut names: Vec<&str> = g.rendered().into_iter().filter(|n| have.iter().any(|h| h == n)).collect();
        if names.is_empty() {
            names.extend(have.first().map(String::as_str));
        }
        graph.keep_outputs(&names)?;
        let mut images: Vec<Option<Tex>> = Vec::new();
        let mut put = |k: u32, t: Tex| {
            let k = k as usize;
            if images.len() <= k {
                images.resize(k + 1, None);
            }
            if let Some(slot) = images.get_mut(k) {
                *slot = Some(t);
            }
        };
        for (k, input) in &g.inputs {
            match input {
                GraphInput::Baked(m) => put(*k, baked_tex(env.baked, *m, side)),
                GraphInput::Constant(c) => put(*k, Tex::solid(side, side, [c[0], c[1], c[2], 1.0])),
                GraphInput::Image(name) => {
                    if let Some(img) = env.images.get(name) {
                        put(*k, Tex::from_fn(side, side, |u, v| img.sample(u, v)));
                    }
                }
                GraphInput::Nested(inner) if depth < MAX_GRAPH_NESTING && !inner.reads_stack() => {
                    let (_, cooked) = self.cook_at(inner, env, &[], depth + 1)?;
                    let out = cooked.output(inner.output.as_deref()).ok_or_else(|| format!("graph {} has no output {:?}", inner.name, inner.output))?;
                    put(*k, Tex::from_fn(side, side, |u, v| sample(out, u, v)));
                }
                // read black: what a stack input reads is given by the compositor
                GraphInput::Nested(_) | GraphInput::Stack { .. } | GraphInput::StackAlpha { .. } | GraphInput::Anchor { .. } => {}
            }
        }
        for (k, t) in given {
            put(*k, t.clone());
        }
        let ec = EvalContext { cancel: self.cooks.cancel.clone(), pool: self.pool(), files: Files::none(), ..EvalContext::default() };
        let outs = evaluate(&graph, &ec, side, images, given)?;
        let linear: BTreeSet<String> =
            graph.meta.get("working").map(|w| w.trim_matches('"').split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default();
        Ok(Cooked::from_textures(outs, linear))
    }
}

/// Sets the graph's parameters to `values`. A parameter is every `input_value` node of its name, set to the value as it
/// is stored, whatever range a slider exposing it offers (a decompiled graph exposes each parameter with its slider
/// range, and files hold values outside it); a graph without such nodes takes it through its exposed parameter of that
/// label. A value for a parameter the graph does not have changes nothing.
pub fn apply_values(graph: &mut Graph, values: &BTreeMap<String, texturecraft_core::GraphValue>) {
    for (name, value) in values {
        let lanes = value.lanes();
        let mut found = false;
        for n in graph.nodes.values_mut().filter(|n| n.kind == "input_value" && n.strs.get("name").is_some_and(|s| s == name)) {
            found = true;
            if let Some(v) = lanes.first() {
                n.nums.insert("v".to_string(), *v);
            }
            for (lane, v) in ["x", "y", "z", "w"].iter().zip(&lanes) {
                n.nums.insert((*lane).to_string(), *v);
            }
        }
        if !found {
            let _ = graph.set_param(name, &lanes);
        }
    }
}

/// Whether the graph's input `idx` is one its package reads as grey.
fn reads_grey(graph: &Graph, idx: u32) -> bool {
    graph
        .nodes
        .values()
        .any(|n| n.kind == "input" && n.nums.get("idx").copied().unwrap_or(0.0) as u32 == idx && n.nums.get("gray").copied().unwrap_or(0.0) > 0.5)
}

/// Evaluates `graph` with `images`. A graph that reads a given input as grey and is given colour there runs once for
/// each of red, green and blue, each as a grey image, and the results make up the colour (measured against the
/// reference app with filters on base colour: 14 of the 16 filters that came out grey matched this way).
fn evaluate(graph: &Graph, ec: &EvalContext, side: usize, images: Vec<Option<Tex>>, given: &[(u32, Tex)]) -> Result<BTreeMap<String, Tex>, String> {
    let colour: Vec<u32> = given.iter().filter(|(k, t)| reads_grey(graph, *k) && !t.px.iter().all(|p| p[0] == p[1] && p[1] == p[2])).map(|(k, _)| *k).collect();
    if colour.is_empty() {
        return graph.evaluate_in(ec, Some(side), &images).map_err(|e| e.to_string());
    }
    let mut merged: BTreeMap<String, Tex> = BTreeMap::new();
    for c in 0..3 {
        let mut one = images.clone();
        for k in &colour {
            if let Some(Some(t)) = one.get_mut(*k as usize) {
                t.px.iter_mut().for_each(|p| *p = [p[c], p[c], p[c], p[3]]);
            }
        }
        for (name, t) in graph.evaluate_in(ec, Some(side), &one).map_err(|e| e.to_string())? {
            match merged.get_mut(&name) {
                Some(m) if (m.w, m.h) == (t.w, t.h) => m.px.iter_mut().zip(&t.px).for_each(|(a, b)| {
                    if let (Some(slot), Some(v)) = (a.get_mut(c), b.get(c)) {
                        *slot = *v;
                    }
                }),
                Some(_) => return Err(format!("output {name} changes size between channels")),
                None => {
                    merged.insert(name, t);
                }
            }
        }
    }
    Ok(merged)
}

/// A baked map as a graph reads it, at `side` (normal maps with green pointing down, as the graph's own normals).
fn baked_tex(baked: &BakedMaps, m: BakedMap, side: usize) -> Tex {
    let s = side as u32;
    let flip = m == BakedMap::Normal;
    let neutral = m.neutral();
    let mut t = Tex::solid(side, side, [neutral[0], neutral[1], neutral[2], 1.0]);
    if !baked.maps.contains_key(&m) {
        return t;
    }
    for (i, p) in t.px.iter_mut().enumerate() {
        let (x, y) = ((i % side) as u32, (i / side) as u32);
        if let Some(v) = baked.sample(m, x, y, (s, s)) {
            *p = [v[0], if flip { 1.0 - v[1] } else { v[1] }, v[2], 1.0];
        }
    }
    t
}

#[cfg(test)]
pub(crate) mod tests;
