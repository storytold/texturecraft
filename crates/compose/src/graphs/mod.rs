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

pub mod budget;
pub mod kept;
pub mod once;
pub mod spill;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use rayon::prelude::*;

use texturecraft_core::{BakedMap, Channel, ChannelKind, GraphInput, GraphRef, LayerId, MAX_GRAPH_NESTING, StableHash};
pub use texturecraft_graph::context::Cancel;
use texturecraft_graph::context::{EvalContext, Files};
use texturecraft_graph::graph::Graph;
pub use texturecraft_graph::graph::Graph as RuntimeGraph;
use texturecraft_graph::rayon::ThreadPool;
pub use texturecraft_graph::tex::Tex;
use texturecraft_texel::{TILE, TileImage, from_u16, linear_to_srgb, srgb_to_linear, to_u16};

use crate::store::{BakedMaps, ImageStore};
pub use kept::KeptCooks;
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
    /// The outputs' mip levels, built when a projection first reads one smaller than it is rendered.
    mips: Mips,
}

impl Cooked {
    pub fn new(outputs: BTreeMap<String, TileImage>, linear: BTreeSet<String>) -> Cooked {
        let mips = Mips(outputs.keys().map(|k| (k.clone(), OnceLock::new())).collect());
        Cooked { outputs, linear, mips }
    }

    /// Every output stored as it is rendered: the same data whether rendered here or loaded from a file.
    pub fn from_textures(outs: BTreeMap<String, Tex>, linear: BTreeSet<String>) -> Cooked {
        let outputs = outs.into_iter().map(|(name, t)| (name, quantize(&t))).collect();
        Cooked::new(outputs, linear)
    }

    /// Output `name` (`None`: the first) at texture coordinates (u, v), prefiltered over `footprint` (the texel's size in
    /// texture coordinates): bilinear where a texel covers at most one pixel of the output, else trilinear between the
    /// mip levels around the footprint, as a GPU reads a minified texture (a pattern repeated many times over the
    /// texture set averages out instead of aliasing).
    pub fn sample_filtered(&self, name: Option<&str>, u: f32, v: f32, footprint: f32) -> Option<[f32; 4]> {
        let (key, image) = match name {
            Some(n) => self.outputs.get_key_value(n)?,
            None => self.outputs.iter().next()?,
        };
        let size = image.width().max(image.height()) as f32;
        let lod = (footprint * size).log2();
        if !lod.is_finite() || lod <= 0.0 {
            return Some(sample(image, u, v));
        }
        let mut spare = Vec::new();
        let levels = self.mips.of(key, image, &mut spare);
        let at =
            |k: usize| if k == 0 { sample(image, u, v) } else { levels.get(k - 1).or(levels.last()).map_or_else(|| sample(image, u, v), |m| m.sample(u, v)) };
        let lo = lod.floor() as usize;
        let t = lod - lod.floor();
        let (a, b) = (at(lo), at(lo + 1));
        Some(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t))
    }

    /// [`Cooked::sample_filtered`] repeating only along the axes `wrap` names (u, v): along the others the edge pixels
    /// extend and coordinates outside 0..1 have no value (`None`, as for a missing output).
    pub fn sample_wrapped(&self, name: Option<&str>, u: f32, v: f32, footprint: f32, wrap: [bool; 2]) -> Option<[f32; 4]> {
        if wrap == [true, true] {
            return self.sample_filtered(name, u, v, footprint);
        }
        if !u.is_finite() || !v.is_finite() || !(wrap[0] || (0.0..=1.0).contains(&u)) || !(wrap[1] || (0.0..=1.0).contains(&v)) {
            return None;
        }
        let (key, image) = match name {
            Some(n) => self.outputs.get_key_value(n)?,
            None => self.outputs.iter().next()?,
        };
        let size = image.width().max(image.height()) as f32;
        let lod = (footprint * size).log2();
        if !lod.is_finite() || lod <= 0.0 {
            return Some(sample_wrap(image, u, v, wrap));
        }
        let mut spare = Vec::new();
        let levels = self.mips.of(key, image, &mut spare);
        let at = |k: usize| {
            if k == 0 {
                sample_wrap(image, u, v, wrap)
            } else {
                levels.get(k - 1).or(levels.last()).map_or_else(|| sample_wrap(image, u, v, wrap), |m| m.sample_wrap(u, v, wrap))
            }
        };
        let lo = lod.floor() as usize;
        let t = lod - lod.floor();
        let (a, b) = (at(lo), at(lo + 1));
        Some(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t))
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

/// The mip levels of a cook's outputs, by output, each built when that output is first read smaller than it is
/// rendered (a cache: cooks holding the same outputs compare equal whatever it holds).
#[derive(Clone, Debug, Default)]
struct Mips(BTreeMap<String, OnceLock<Arc<Vec<Mip>>>>);

impl Mips {
    /// The levels of output `key` (`image`), built now if they are not yet.
    fn of<'a>(&'a self, key: &str, image: &TileImage, spare: &'a mut Vec<Mip>) -> &'a [Mip] {
        match self.0.get(key) {
            Some(slot) => slot.get_or_init(|| Arc::new(mip_levels(image))),
            // an output put in after the cook was made: built for this read only
            None => {
                *spare = mip_levels(image);
                spare
            }
        }
    }
}

impl PartialEq for Mips {
    fn eq(&self, _: &Mips) -> bool {
        true
    }
}

/// One mip level, row by row.
#[derive(Debug)]
struct Mip {
    w: usize,
    h: usize,
    px: MipPixels,
}

/// A level's pixels: RGBA, or one value for a grey opaque output (read as that value three times and alpha 1, which is
/// what the RGBA form of such an output holds bit for bit: the mean of four equal channels is the same in each, and
/// the mean of four ones is one).
#[derive(Debug)]
enum MipPixels {
    Rgba(Vec<[f32; 4]>),
    Grey(Vec<f32>),
}

impl Mip {
    fn at(&self, x: i64, y: i64) -> [f32; 4] {
        let (w, h) = (self.w.max(1) as i64, self.h.max(1) as i64);
        let i = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize;
        match &self.px {
            MipPixels::Rgba(px) => px.get(i).copied().unwrap_or([0.0; 4]),
            MipPixels::Grey(px) => px.get(i).map_or([0.0; 4], |g| [*g, *g, *g, 1.0]),
        }
    }

    /// Bilinear sample repeating along the axes `wrap` names, the edge extended along the others.
    fn sample_wrap(&self, u: f32, v: f32, wrap: [bool; 2]) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let (w, h) = (self.w.max(1) as i64, self.h.max(1) as i64);
        let at = |x: i64, y: i64| self.at(if wrap[0] { x } else { x.clamp(0, w - 1) }, if wrap[1] { y } else { y.clamp(0, h - 1) });
        bilinear_with(u * self.w as f32 - 0.5, v * self.h as f32 - 0.5, at)
    }

    /// Bilinear sample with wrap-around (v down), as [`sample`].
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let fx = u * self.w as f32 - 0.5;
        let fy = v * self.h as f32 - 0.5;
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let (a, b, c, d) = (self.at(x0, y0), self.at(x0 + 1, y0), self.at(x0, y0 + 1), self.at(x0 + 1, y0 + 1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            top + (bot - top) * ty
        })
    }

    /// Half the size (at least one pixel): each pixel the mean of the two by two it covers.
    fn halved(&self) -> Mip {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mean = |i: usize| -> [f32; 4] {
            let (x, y) = ((i % w) as i64 * 2, (i / w) as i64 * 2);
            let q = [self.at(x, y), self.at(x + 1, y), self.at(x, y + 1), self.at(x + 1, y + 1)];
            std::array::from_fn(|k| (q[0][k] + q[1][k] + q[2][k] + q[3][k]) * 0.25)
        };
        let px = match &self.px {
            MipPixels::Rgba(_) => MipPixels::Rgba((0..w * h).map(mean).collect()),
            MipPixels::Grey(_) => MipPixels::Grey((0..w * h).map(|i| mean(i)[0]).collect()),
        };
        Mip { w, h, px }
    }
}

/// The mip levels of `img` below itself, halving down to one pixel.
fn mip_levels(img: &TileImage) -> Vec<Mip> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let at = |i: usize| texel(img, (i % w.max(1)) as i64, (i / w.max(1)) as i64);
    // a one-component output reads as its value three times and alpha 1
    let px = if img.comps() == 1 { MipPixels::Grey((0..w * h).map(|i| at(i)[0]).collect()) } else { MipPixels::Rgba((0..w * h).map(at).collect()) };
    let base = Mip { w, h, px };
    let mut levels = vec![base];
    while let Some(last) = levels.last()
        && (last.w > 1 || last.h > 1)
    {
        let next = last.halved();
        levels.push(next);
    }
    levels.remove(0);
    levels
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

/// Bilinear interpolation around pixel coordinate (`fx`, `fy`) (pixel centres at whole numbers) of `at`'s pixels.
fn bilinear_with(fx: f32, fy: f32, at: impl Fn(i64, i64) -> [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        top + (bot - top) * ty
    })
}

/// [`sample`] repeating along the axes `wrap` names (u, v), the edge pixels extended along the others.
pub fn sample_wrap(img: &TileImage, u: f32, v: f32, wrap: [bool; 2]) -> [f32; 4] {
    if !u.is_finite() || !v.is_finite() {
        return [0.0; 4];
    }
    let (w, h) = (i64::from(img.width().max(1)), i64::from(img.height().max(1)));
    let at = |x: i64, y: i64| texel(img, if wrap[0] { x } else { x.clamp(0, w - 1) }, if wrap[1] { y } else { y.clamp(0, h - 1) });
    bilinear_with(u * img.width() as f32 - 0.5, v * img.height() as f32 - 0.5, at)
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
/// is declared linear. Data (user channels): as the graph gives it. Height: the graph's 0..1 over the signed −1..1, 0.5 the flat surface. Normal: green flipped (a
/// graph's normals point green down) and blue rebuilt from red and green as a unit normal's.
pub fn graph_to_channel(kind: ChannelKind, v: [f32; 4], linear: bool) -> [f32; 3] {
    match kind {
        ChannelKind::Color if linear => [v[0], v[1], v[2]],
        ChannelKind::Data => [v[0], v[1], v[2]],
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
        ChannelKind::Data => v,
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

/// Listener for cold graph cook and node diagnostics.
pub type CookTrace = Arc<dyn Fn(String) + Send + Sync>;
pub use texturecraft_graph::accelerator::Accelerator as GraphAccelerator;

/// Cooked outputs and what is known about them, shared by every snapshot of a document (a cook's key decides its
/// pixels, so snapshots never disagree about one).
#[derive(Default)]
pub struct CookCache {
    cooks: Slots<String, Result<Arc<Cooked>, String>>,
    /// Results a background resize can read without changing the live document's graph uses.
    resize_source: Option<Arc<CookCache>>,
    resize_cache_hits: AtomicUsize,
    /// The cook each use read last.
    uses: Mutex<BTreeMap<GraphUse, String>>,
    problems: Mutex<BTreeMap<GraphUse, UseProblem>>,
    renders: AtomicUsize,
    parsed: Mutex<HashMap<String, Arc<Graph>>>,
    kinds: Mutex<HashMap<String, u128>>,
    /// Fingerprints of baked maps and image resources by the address and length of their data.
    fingerprints: Mutex<HashMap<(usize, usize), u128>>,
    pool: OnceLock<Option<Arc<ThreadPool>>>,
    trace: Mutex<Option<CookTrace>>,
    accelerator: Mutex<Option<Arc<dyn GraphAccelerator>>>,
    /// Where cooks are also kept between documents and runs ([`kept`]), if anywhere.
    kept: Mutex<Option<Arc<dyn KeptCooks>>>,
    /// Cooks found there instead of rendered.
    kept_found: AtomicUsize,
    /// Cooks moved out of memory ([`GraphStore::spill_cooks`]).
    spilled: spill::Spill,
    /// Cooks made available since the store was made: rendered, found where kept or read back from a spill.
    filled: AtomicUsize,
    /// Renders running now: their finished nodes and the nodes they run (see [`GraphStore::cook_progress`]).
    running: Mutex<Vec<RunningCook>>,
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

fn trace_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or_default()
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

    /// An empty store whose renders stop when `cancel` is set (a render in progress stops at its next node; later ones
    /// do not start). The flag stays set: once it is, the store renders nothing more.
    pub fn with_cancel(cancel: Cancel) -> GraphStore {
        GraphStore { texts: BTreeMap::new(), cooks: Arc::new(CookCache { cancel, ..CookCache::default() }) }
    }

    /// Evaluate a candidate resolution without updating the current view's graph usage or cancellation state.
    pub fn for_resize(&self, cancel: Cancel, set: usize) -> GraphStore {
        let store = GraphStore {
            texts: self.texts.clone(),
            cooks: Arc::new(CookCache {
                resize_source: Some(self.cooks.clone()),
                uses: Mutex::new(self.uses().into_iter().filter(|(use_, _)| use_.set == set).collect()),
                cancel,
                ..CookCache::default()
            }),
        };
        store.set_accelerator(self.accelerator());
        store
    }

    /// Preserve completed results and install the new uses only after the resize has been accepted.
    pub fn accept_resize(&self, prepared: &GraphStore, set: usize) {
        for (key, result) in prepared.cooks.cooks.values() {
            self.cooks.cooks.insert(key, result);
        }
        let uses = prepared.uses();
        let problems = prepared.problems();
        {
            let mut previous = lock(&self.cooks.uses);
            previous.retain(|use_, _| use_.set != set);
            previous.extend(uses);
        }
        {
            let mut previous = lock(&self.cooks.problems);
            previous.retain(|use_, _| use_.set != set);
            previous.extend(problems);
        }
        self.cooks.renders.fetch_add(prepared.renders(), Ordering::Relaxed);
        self.cooks.kept_found.fetch_add(prepared.kept_found(), Ordering::Relaxed);
    }

    /// Park older resolutions on disk when preparing another view would keep too much of their output in memory.
    /// The currently shown and incoming results stay resident so neither view reads them back tile by tile.
    pub fn park_old_resize_results(&self, limit: usize) {
        let Some(source) = self.cooks.resize_source.as_ref() else { return };
        let incoming: HashMap<_, _> = self.cooks.cooks.values().into_iter().filter_map(|(key, result)| result.ok().map(|cook| (key, cook.bytes()))).collect();
        let previous: Vec<_> = source.cooks.values().into_iter().filter_map(|(key, result)| result.ok().map(|cook| (key, cook))).collect();
        let held: usize = previous.iter().filter(|(key, _)| !incoming.contains_key(key)).map(|(_, cook)| cook.bytes()).sum();
        let mut occupied = held.saturating_add(incoming.values().sum());
        if occupied <= limit {
            return;
        }
        let active: BTreeSet<_> = lock(&source.uses).values().cloned().collect();
        for (key, cook) in previous {
            if !active.contains(&key) && !incoming.contains_key(&key) && (source.spilled.outline(&key).is_some() || source.spilled.put(&key, &cook)) {
                source.cooks.remove(&key);
                occupied = occupied.saturating_sub(cook.bytes());
                if occupied <= limit {
                    break;
                }
            }
        }
    }

    /// Adds a graph text, under its key.
    pub fn add_text(&mut self, text: &str) -> String {
        let key = texturecraft_core::text_key(text);
        self.texts.entry(key.clone()).or_insert_with(|| Arc::from(text));
        key
    }

    /// Also keeps cooks in `kept` (and looks there before rendering), for every snapshot sharing this store's cooks.
    pub fn keep_cooks_in(&self, kept: Option<Arc<dyn KeptCooks>>) {
        *lock(&self.cooks.kept) = kept;
    }

    /// Report diagnostics from uncached graph renders until the listener is removed.
    pub fn trace_cooks(&self, trace: Option<CookTrace>) {
        *lock(&self.cooks.trace) = trace;
    }

    pub fn accelerator(&self) -> Option<Arc<dyn GraphAccelerator>> {
        lock(&self.cooks.accelerator).clone()
    }

    pub fn set_accelerator(&self, accelerator: Option<Arc<dyn GraphAccelerator>>) {
        *lock(&self.cooks.accelerator) = accelerator;
    }

    /// Cooks found where they are kept ([`GraphStore::keep_cooks_in`]) instead of rendered.
    pub fn kept_found(&self) -> usize {
        self.cooks.kept_found.load(Ordering::Relaxed)
    }

    /// Matching results read from the live cache or disk during a background resize.
    pub fn resize_cache_hits(&self) -> usize {
        self.cooks.resize_cache_hits.load(Ordering::Relaxed) + self.kept_found()
    }

    /// Renders made since the store was made (by every snapshot sharing its cooks).
    pub fn renders(&self) -> usize {
        self.cooks.renders.load(Ordering::Relaxed)
    }

    /// How far cooking has come since the store was made, in cooks: one per cook made available (rendered, found where
    /// kept or read back), plus each running render's share of its nodes finished (at most 0.95). For a progress
    /// display; it only grows while nothing is cancelled.
    pub fn cook_progress(&self) -> f32 {
        let running: f32 = lock(&self.cooks.running).iter().map(|cook| (cook.done.load(Ordering::Relaxed) as f32 / cook.nodes.max(1) as f32).min(0.95)).sum();
        self.cooks.filled.load(Ordering::Relaxed) as f32 + running
    }

    /// Name and node progress of the latest graph currently evaluating.
    pub fn active_cook(&self) -> Option<(String, usize, usize)> {
        let running = lock(&self.cooks.running);
        let cook = running.last()?;
        Some((cook.name.clone(), cook.done.load(Ordering::Relaxed).min(cook.nodes), cook.nodes))
    }

    /// Uses whose graph showed nothing, or showed stale pixels, the last time they were composited.
    pub fn problems(&self) -> Vec<(GraphUse, UseProblem)> {
        lock(&self.cooks.problems).iter().map(|(u, p)| (u.clone(), p.clone())).collect()
    }

    /// Whether the last cook of `use_` failed (see [`GraphStore::cook_for`]).
    pub fn has_problem(&self, use_: &GraphUse) -> bool {
        lock(&self.cooks.problems).contains_key(use_)
    }

    /// The cook each use read last.
    pub fn uses(&self) -> BTreeMap<GraphUse, String> {
        lock(&self.cooks.uses).clone()
    }

    /// Sets what each use read last (from a project file).
    pub fn set_uses(&self, uses: BTreeMap<GraphUse, String>) {
        *lock(&self.cooks.uses) = uses;
    }

    /// The last complete cook read by an effect or source, including a stack-dependent effect.
    pub fn use_cook_key(&self, use_: &GraphUse) -> Option<String> {
        lock(&self.cooks.uses).get(use_).cloned()
    }

    /// Record a resolved source as used for this document, including when the GPU binds a cached result.
    pub fn note_use(&self, use_: GraphUse, key: &str) {
        let mut uses = lock(&self.cooks.uses);
        if uses.get(&use_).is_none_or(|previous| previous != key) {
            uses.insert(use_.clone(), key.to_owned());
        }
        drop(uses);
        lock(&self.cooks.problems).remove(&use_);
    }

    /// Look up a result without paging in a disk cook on the UI thread.
    pub fn resident_cook(&self, key: &str) -> Option<Arc<Cooked>> {
        let key = key.to_owned();
        self.cooks.cooks.get(&key).or_else(|| self.cooks.resize_source.as_ref()?.cooks.get(&key)).and_then(Result::ok)
    }

    pub fn failed_cook(&self, key: &str) -> bool {
        self.cooks.cooks.get(&key.to_owned()).is_some_and(|cook| cook.is_err())
    }

    /// The cook under `key`, if one is held (rendered or loaded; a spilled one is read back, not held again).
    pub fn cooked(&self, key: &str) -> Option<Arc<Cooked>> {
        match self.cooks.cooks.get(&key.to_string()) {
            Some(held) => held.ok(),
            None => self.cooks.spilled.get(key).map(Arc::new).or_else(|| {
                let source = self.cooks.resize_source.as_ref()?;
                source.cooks.get(&key.to_owned()).and_then(Result::ok).or_else(|| source.spilled.get(key).map(Arc::new))
            }),
        }
    }

    /// Read a saved set's recorded graph results on the preparation worker. A matching
    /// archive snapshot can use these results without evaluating its stack again.
    pub fn warm_saved_uses(&self, set: usize) {
        let keys: BTreeSet<String> = self.uses().into_iter().filter_map(|(use_, key)| (use_.set == set).then_some(key)).collect();
        for key in keys {
            if self.cooks.cancel.is_cancelled() {
                break;
            }
            if self.cooks.cooks.get(&key).is_none()
                && let Some(cooked) = self.cooked(&key)
            {
                self.cooks.cooks.insert(key, Ok(cooked));
            }
        }
    }

    /// What the cook under `key` holds (outputs, their tiles, linear ones), without reading a spilled one back.
    pub fn cook_outline(&self, key: &str) -> Option<spill::CookOutline> {
        match self.cooks.cooks.get(&key.to_string()) {
            Some(held) => held.ok().map(|c| spill::CookOutline::of(&c)),
            None => self.cooks.spilled.outline(key),
        }
    }

    /// Moves every cook held in memory out to a file of its own (read back when something needs it), so that what
    /// was cooked so far stops taking memory: an import does this after each texture set. Cooks that cannot be written
    /// out stay. Returns how many moved; none in the web build.
    pub fn spill_cooks(&self) -> usize {
        let mut moved = 0;
        for (key, held) in self.cooks.cooks.values() {
            let Ok(cooked) = held else { continue };
            // (one read back from its file is there already)
            if self.cooks.spilled.outline(&key).is_some() || self.cooks.spilled.put(&key, &cooked) {
                self.cooks.cooks.remove(&key);
                moved += 1;
            }
        }
        moved
    }

    /// The cooks `source` holds (their outlines by key) are read from it when something needs them, instead of being
    /// held from the start: what a document opened from a project file does with the file's cooks.
    pub fn read_cooks_from(&self, source: Arc<dyn spill::CookSource>, outlines: HashMap<String, spill::CookOutline>) {
        self.cooks.spilled.set_source(source, outlines);
    }

    /// Bytes of the cooks held in memory.
    pub fn held_bytes(&self) -> usize {
        self.cooks.cooks.values().into_iter().filter_map(|(_, v)| v.ok()).map(|c| c.bytes()).sum()
    }

    /// When the cooks held in memory take more than `limit` bytes, lets go of those that can be read again (spilled,
    /// or kept by the project file); they come back when a composite or a save needs them. Returns how many went.
    pub fn release_reloadable(&self, limit: usize) -> usize {
        if self.held_bytes() <= limit {
            return 0;
        }
        let mut gone = 0;
        for (key, held) in self.cooks.cooks.values() {
            if held.is_ok() && self.cooks.spilled.outline(&key).is_some() {
                self.cooks.cooks.remove(&key);
                gone += 1;
            }
        }
        gone
    }

    /// The keys of the cooks the uses read that are held or spilled (what a project file saves), sorted.
    pub fn used_cook_keys(&self) -> Vec<String> {
        let keys: BTreeSet<String> = self.uses().into_values().collect();
        keys.into_iter().filter(|k| self.cook_outline(k).is_some()).collect()
    }

    /// Puts a cook in under `key` (loading a project file).
    pub fn insert_cooked(&self, key: String, cooked: Cooked) {
        self.cooks.cooks.insert(key, Ok(Arc::new(cooked)));
    }

    /// The cooks the uses read, by key, sorted (spilled ones read back: [`GraphStore::used_cook_keys`] and
    /// [`GraphStore::cooked`] read them one at a time).
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
        self.cooks.spilled.retain(|k| used.contains(k));
    }

    fn read_cache(&self) -> &CookCache {
        self.cooks.resize_source.as_deref().unwrap_or(&self.cooks)
    }

    fn pool(&self) -> Option<Arc<ThreadPool>> {
        self.read_cache().pool.get_or_init(|| texturecraft_graph::context::own_pool("graph")).clone()
    }

    /// A digest of the evaluator versions of the node kinds text `key` uses ([`texturecraft_graph::versions`]).
    fn kind_digest(&self, key: &str) -> u128 {
        if let Some(d) = lock(&self.read_cache().kinds).get(key) {
            return *d;
        }
        let mut h = StableHash::new();
        if let Some(text) = self.texts.get(key) {
            for kind in node_kinds(text) {
                h.str(kind).str(texturecraft_graph::versions::of_kind(kind));
            }
        }
        let d = h.finish();
        lock(&self.read_cache().kinds).insert(key.to_string(), d);
        d
    }

    /// A fingerprint of float data, by its address and length while it lives (baked maps are replaced, not changed in
    /// place, when they are baked again).
    fn fingerprint_floats(&self, data: &[f32]) -> u128 {
        let at = (data.as_ptr() as usize, data.len());
        if let Some(f) = lock(&self.read_cache().fingerprints).get(&at) {
            return *f;
        }
        let mut h = StableHash::new();
        for v in data {
            h.f32(*v);
        }
        let f = h.finish();
        lock(&self.read_cache().fingerprints).insert(at, f);
        f
    }

    /// [`GraphStore::fingerprint_floats`] of a baked map's values, however they are held.
    fn fingerprint_texels(&self, data: &Arc<crate::Texels>) -> u128 {
        let at = (Arc::as_ptr(data) as usize, data.len());
        if let Some(f) = lock(&self.read_cache().fingerprints).get(&at) {
            return *f;
        }
        let mut h = StableHash::new();
        for v in data.iter() {
            h.f32(v);
        }
        let f = h.finish();
        lock(&self.read_cache().fingerprints).insert(at, f);
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
            if let texturecraft_core::GraphValue::Text(t) = value {
                h.str(t);
            }
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
                    let fp = env.baked.maps.get(m).map_or(0, |d| self.fingerprint_texels(d));
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
    /// not [`texturecraft_graph::versions::TOOLCHAIN`]. The reference evaluator takes its maths from one library.
    /// A host accelerator adds its own version to newly computed results. `None` for graphs nested too deeply.
    pub fn cook_key(&self, g: &GraphRef, env: &RenderEnv, given: &[(u32, Tex)]) -> Option<String> {
        let base = self.reference_key(g, env, given)?;
        Some(self.backend_key(base, self.accelerator().as_deref()))
    }

    fn reference_key(&self, g: &GraphRef, env: &RenderEnv, given: &[(u32, Tex)]) -> Option<String> {
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

    fn backend_key(&self, base: String, accelerator: Option<&dyn GraphAccelerator>) -> String {
        let Some(accelerator) = accelerator else { return base };
        // Retain results already saved by the reference evaluator for this precise request.
        let held = |cache: &CookCache| cache.cooks.get(&base).is_some_and(|v| v.is_ok()) || cache.spilled.outline(&base).is_some();
        if held(&self.cooks) || self.cooks.resize_source.as_ref().is_some_and(|cache| held(cache)) {
            return base;
        }
        let mut h = StableHash::new();
        h.str("accelerated cook").str(accelerator.cache_tag()).str(&base);
        h.key()
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
        if let Some(g) = lock(&self.read_cache().parsed).get(key) {
            return Ok(g.clone());
        }
        let text = self.texts.get(key).ok_or_else(|| format!("the text of graph {key} is not in the project"))?;
        let g = Arc::new(Graph::parse(text).map_err(|e| e.to_string())?);
        let mut parsed = lock(&self.read_cache().parsed);
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
        let accelerator = self.accelerator();
        let base = self.reference_key(g, env, given).ok_or_else(|| format!("graphs nested deeper than {MAX_GRAPH_NESTING}"))?;
        let key = self.backend_key(base, accelerator.as_deref());
        let pool = self.pool();
        let got = self.cooks.cooks.get_or_fill_own_pool(&key, || {
            // waiting for the render's pool here must not pick up the caller's pool's work (see `once`)
            once::wait_outside(pool.as_deref(), || self.fill_cook(g, env, given, depth, &key, accelerator))
        });
        // a cancelled render (not started, or stopped part way) is not remembered as a failure
        if got.is_err() && self.cooks.cancel.is_cancelled() {
            self.cooks.cooks.remove(&key);
        }
        got.map(|c| (key, c))
    }

    /// The cook under `key`: spilled or kept earlier, or rendered now.
    fn fill_cook(
        &self,
        g: &GraphRef,
        env: &RenderEnv,
        given: &[(u32, Tex)],
        depth: usize,
        key: &str,
        accelerator: Option<Arc<dyn GraphAccelerator>>,
    ) -> Result<Arc<Cooked>, String> {
        if self.cooks.cancel.is_cancelled() {
            return Err("cancelled".to_string());
        }
        let trace = lock(&self.cooks.trace).clone();
        let name = trace_name(&g.name);
        if let Some(source) = &self.cooks.resize_source {
            let result = source.cooks.get(&key.to_owned()).or_else(|| source.spilled.get(key).map(|result| Ok(Arc::new(result))));
            if let Some(result) = result {
                self.cooks.filled.fetch_add(1, Ordering::Relaxed);
                self.cooks.resize_cache_hits.fetch_add(1, Ordering::Relaxed);
                return result;
            }
        }
        if let Some(back) = self.cooks.spilled.get(key) {
            if let Some(emit) = &trace {
                emit(format!("Graph {name:?}: loaded from project cache ({key})"));
            }
            self.cooks.filled.fetch_add(1, Ordering::Relaxed);
            return Ok(Arc::new(back));
        }
        let kept = lock(&self.cooks.kept).clone().or_else(|| self.cooks.resize_source.as_ref().and_then(|source| lock(&source.kept).clone()));
        if let Some(found) = kept.as_ref().and_then(|k| k.get(key)).and_then(|bytes| kept::decode(&bytes)) {
            if let Some(emit) = &trace {
                emit(format!("Graph {name:?}: result cache hit ({key})"));
            }
            self.cooks.kept_found.fetch_add(1, Ordering::Relaxed);
            self.cooks.filled.fetch_add(1, Ordering::Relaxed);
            return Ok(Arc::new(found));
        }
        self.cooks.renders.fetch_add(1, Ordering::Relaxed);
        if let Some(emit) = &trace {
            emit(format!("Graph {name:?}: preparing {} px cook ({key})", env.side));
        }
        // a share of the render budget for its estimated peak; nested graphs render within their parent's
        let share = if depth == 0 { Some(budget::acquire(budget::estimate(&*self.parsed(&g.key)?, env.side))) } else { None };
        let cooked = self.render(g, env, given, depth, trace.clone(), accelerator);
        drop(share);
        if let Some(emit) = &trace {
            let status = if cooked.is_ok() { "finished" } else { "stopped" };
            emit(format!("Graph {name:?}: {status} ({key})"));
        }
        let cooked = cooked?;
        // a render stopped part way never gets here: it is an error
        if let Some(k) = &kept {
            k.put(key, &kept::encode(&cooked));
        }
        self.cooks.filled.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(cooked))
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
    fn render(
        &self,
        g: &GraphRef,
        env: &RenderEnv,
        given: &[(u32, Tex)],
        depth: usize,
        trace: Option<CookTrace>,
        accelerator: Option<Arc<dyn GraphAccelerator>>,
    ) -> Result<Cooked, String> {
        let side = env.side;
        let pool = self.pool();
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
                GraphInput::Baked(m) => put(*k, baked_tex(env.baked, *m, side, pool.as_deref())),
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
        // the render's nodes as they finish, for the store's progress (nothing computed depends on it)
        let done = Arc::new(AtomicUsize::new(0));
        let nodes = graph.work_nodes();
        let _running = Running::start(&self.cooks.running, done.clone(), nodes, trace_name(&g.name));
        let profiler = trace.map(|emit| {
            let graph = trace_name(&g.name).to_owned();
            emit(format!("Graph {graph:?}: evaluating {nodes} nodes"));
            Arc::new(CookProfiler { emit, graph }) as Arc<dyn texturecraft_graph::profile::Profiler>
        });
        let ec = EvalContext {
            cancel: self.cooks.cancel.clone(),
            pool,
            files: Files::none(),
            nodes_done: Some(done),
            profiler,
            accelerator,
            ..EvalContext::default()
        };
        let outs = evaluate(&graph, &ec, side, images, given)?;
        let linear: BTreeSet<String> =
            graph.meta.get("working").map(|w| w.trim_matches('"').split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default();
        Ok(Cooked::from_textures(outs, linear))
    }
}

struct CookProfiler {
    emit: CookTrace,
    graph: String,
}

impl texturecraft_graph::profile::Profiler for CookProfiler {
    fn enabled(&self) -> bool {
        false
    }

    fn span(&self, _: &str) -> Box<dyn std::any::Any + Send> {
        Box::new(())
    }

    fn accelerated_node(&self, id: u32, kind: &str) {
        (self.emit)(format!("GPU result for node #{id} {kind} in {:?}", self.graph));
    }

    fn resident_node(&self, id: u32, kind: &str) {
        (self.emit)(format!("GPU-resident result for node #{id} {kind} in {:?}", self.graph));
    }

    fn resident_readback(&self, id: u32, bytes: usize) {
        (self.emit)(format!("GPU graph readback for node #{id} in {:?}: {bytes} bytes", self.graph));
    }

    fn node(&self, id: u32, kind: &str, (width, height): (usize, usize)) -> Option<Box<dyn std::any::Any + Send>> {
        let description = format!("#{id} {kind} ({width}×{height}) in {:?}", self.graph);
        (self.emit)(format!("Node {description}"));
        #[cfg(not(target_arch = "wasm32"))]
        {
            Some(Box::new(CookNodeTime { emit: self.emit.clone(), description, started: std::time::Instant::now() }))
        }
        #[cfg(target_arch = "wasm32")]
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct CookNodeTime {
    emit: CookTrace,
    description: String,
    started: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for CookNodeTime {
    fn drop(&mut self) {
        let ms = self.started.elapsed().as_millis();
        if ms >= 100 {
            (self.emit)(format!("Slow node {}: {ms} ms", self.description));
        }
    }
}

/// A render listed among the running ones (see [`GraphStore::cook_progress`]) until dropped.
struct Running<'a> {
    list: &'a Mutex<Vec<RunningCook>>,
    done: Arc<AtomicUsize>,
}

struct RunningCook {
    done: Arc<AtomicUsize>,
    nodes: usize,
    name: String,
}

impl<'a> Running<'a> {
    fn start(list: &'a Mutex<Vec<RunningCook>>, done: Arc<AtomicUsize>, nodes: usize, name: &str) -> Running<'a> {
        lock(list).push(RunningCook { done: done.clone(), nodes, name: name.to_owned() });
        Running { list, done }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        lock(self.list).retain(|cook| !Arc::ptr_eq(&cook.done, &self.done));
    }
}

/// Sets the graph's parameters to `values`. A parameter is every `input_value` node of its name, set to the value as it
/// is stored, whatever range a slider exposing it offers (a decompiled graph exposes each parameter with its slider
/// range, and files hold values outside it); a graph without such nodes takes it through its exposed parameter of that
/// label. A value for a parameter the graph does not have changes nothing.
pub fn apply_values(graph: &mut Graph, values: &BTreeMap<String, texturecraft_core::GraphValue>) {
    for (name, value) in values {
        // a text sets the text of every `input_value` node of its name (a text parameter has no exposed slider)
        if let texturecraft_core::GraphValue::Text(t) = value {
            for n in graph.nodes.values_mut().filter(|n| n.kind == "input_value" && n.strs.get("name").is_some_and(|s| s == name)) {
                n.strs.insert("v".to_string(), t.clone());
            }
            continue;
        }
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
        // (the images handed over: the runtime holds them without a copy)
        return graph.evaluate_owned(ec, Some(side), images).map_err(|e| e.to_string());
    }
    let mut merged: BTreeMap<String, Tex> = BTreeMap::new();
    for c in 0..3 {
        let mut one = images.clone();
        for k in &colour {
            if let Some(Some(t)) = one.get_mut(*k as usize) {
                t.px.iter_mut().for_each(|p| *p = [p[c], p[c], p[c], p[3]]);
            }
        }
        for (name, t) in graph.evaluate_owned(ec, Some(side), one).map_err(|e| e.to_string())? {
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
fn baked_tex(baked: &BakedMaps, map: BakedMap, side: usize, pool: Option<&ThreadPool>) -> Tex {
    let [r, g, b] = map.neutral();
    let mut tex = Tex::solid(side, side, [r, g, b, 1.0]);
    let Some(data) = baked.maps.get(&map).filter(|_| side > 0) else { return tex };

    // Match BakedMaps::sample's integer nearest-texel mapping. The source map and
    // horizontal sampling offsets are the same for every output row.
    let width = u64::from(baked.resolution.max(1));
    let height = u64::from(baked.height(map).max(1));
    let components = map.components();
    let columns: Vec<_> = (0..side).map(|x| (x as u64 * width / side as u64) as usize).collect();
    let normal = map == BakedMap::Normal;
    let row = |(y, pixels): (usize, &mut [[f32; 4]])| {
        let start = (y as u64 * height / side as u64 * width) as usize;
        for (p, x) in pixels.iter_mut().zip(&columns) {
            let Some(i) = (start + x).checked_mul(components) else { continue };
            if components == 3 {
                if let (Some(r), Some(g), Some(b)) = (data.get(i), data.get(i + 1), data.get(i + 2)) {
                    *p = [r, if normal { 1.0 - g } else { g }, b, 1.0];
                }
            } else if let Some(v) = data.get(i) {
                *p = [v, v, v, 1.0];
            }
        }
    };
    // A compositor worker can be waiting for this cook. Use the graph's pool
    // so preparing its inputs never waits on the compositor's pool.
    if let Some(pool) = pool.filter(|_| side >= 256) {
        pool.install(|| tex.px.par_chunks_mut(side).with_min_len(32).enumerate().for_each(row));
    } else {
        tex.px.chunks_mut(side).enumerate().for_each(row);
    }
    tex
}

#[cfg(test)]
pub(crate) mod tests;
