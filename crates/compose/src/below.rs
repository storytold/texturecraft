//! A per-tile cache of everything in a stack but one "hot" layer (the layer being painted or
//! edited), so recompositing a tile only evaluates that layer. Per texel it holds the stack
//! accumulated before the hot layer at each nesting level (inside its folders) and a tape of
//! what the layers above it contribute (masks, content values and coverage, none of which read
//! the stack), so the blends above are replayed instead of evaluated. The cache is keyed by
//! everything the cached part reads; any change to it empties the cache, so results are always
//! identical to a full evaluation.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use texturecraft_core::project::MAX_DEPTH;
use texturecraft_core::{BakedMap, Channel, Layer, LayerId, LayerKind, Project, TextureSet};
use texturecraft_texel::TileImage;

use crate::eval::{Fields, Inputs, PaintTarget};
use crate::store::{BakedMaps, ImageStore, PixelStore};

/// Default byte budget of a [`BelowCache`], per nesting level it holds (see
/// [`BelowCache::limit`]).
pub const BELOW_BUDGET: usize = 256 << 20;

/// See the module docs. One per texture set.
#[derive(Debug)]
pub struct BelowCache {
    hot: Option<LayerId>,
    key: Option<StackKey>,
    /// Tiles with the stamp of their last use.
    tiles: HashMap<(u32, u32), (u64, Vec<Band>)>,
    /// Uses, oldest first (eviction beyond the budget); entries a later use superseded are
    /// skipped.
    order: VecDeque<((u32, u32), u64)>,
    clock: u64,
    bytes: usize,
    /// Byte budget per nesting level; the least recently used tiles are dropped beyond
    /// [`BelowCache::limit`].
    pub budget: usize,
    /// Nesting levels of the path to the hot layer (at least 1).
    levels: usize,
    /// Tiles of the whole pass a caller splits into several composites (0: each composite is a
    /// pass). A pass larger than the cache records only what fits.
    pub pass: usize,
    /// Tiles the caller composites next in this pass, after those of the coming composite (empty: none, or a pass that is one
    /// composite). A stack with neighbourhood filters then filters the coming tiles and these at once and keeps the result for
    /// the composites that follow, instead of filtering each small chunk's surroundings again.
    pub ahead: Vec<(u32, u32)>,
    pub(crate) held: Option<HeldFields>,
}

/// Neighbourhood-filter fields kept between the composites of one pass.
pub(crate) struct HeldFields {
    /// Everything the fields read.
    pub key: Arc<StackKey>,
    /// The working size they were built at.
    pub size: (u32, u32),
    /// The tiles every layer's fields hold.
    pub stack_tiles: BTreeSet<(u32, u32)>,
    /// The tiles the hot layer's fields hold (the same tiles when the stack was not split).
    pub hot_tiles: BTreeSet<(u32, u32)>,
    pub fields: Arc<Fields>,
}

impl std::fmt::Debug for HeldFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldFields").field("stack_tiles", &self.stack_tiles.len()).field("hot_tiles", &self.hot_tiles.len()).finish_non_exhaustive()
    }
}

impl Default for BelowCache {
    fn default() -> Self {
        BelowCache {
            hot: None,
            key: None,
            tiles: HashMap::new(),
            order: VecDeque::new(),
            clock: 0,
            bytes: 0,
            budget: BELOW_BUDGET,
            levels: 1,
            pass: 0,
            ahead: Vec::new(),
            held: None,
        }
    }
}

/// The cached part of one band of rows of a tile: which nesting levels it stores the stack of,
/// then covered texel after covered texel the stack before the path layer at those levels and
/// what the layers above the hot layer contribute (see the module docs).
pub(crate) type Band = Vec<f32>;

fn band_bytes(b: &Band) -> usize {
    b.len() * std::mem::size_of::<f32>()
}

/// One nesting level of the path from the top of a stack to the hot layer (top level first):
/// the layers drawn before the path layer, the path layer (a folder holding the hot layer, or
/// the layer drawing the hot layer on the last level) and the layers drawn after it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Level {
    pub below: Vec<LayerId>,
    pub at: LayerId,
    pub above: Vec<LayerId>,
}

/// Everything compositing `roots` of a texture set reads: equal keys composite identically.
/// Pixels are compared by tile identity (copy-on-write), so building and comparing keys is cheap.
#[derive(Debug, PartialEq)]
pub struct StackKey {
    /// The texture set without its name, description, stack and bake settings.
    set: TextureSet,
    roots: Vec<LayerId>,
    /// The layers drawn, without names and other display-only fields.
    layers: Vec<Layer>,
    pixels: Vec<(LayerId, Option<Channel>, Shared)>,
    baked: (u32, Vec<(BakedMap, usize)>),
    images: Vec<(String, usize)>,
}

impl StackKey {
    /// The key of `roots` (layers of texture set `ts`, bottom → top) and their effects.
    pub fn new(project: &Project, pixels: &PixelStore, baked: &BakedMaps, images: &ImageStore, ts: usize, roots: &[LayerId]) -> Option<StackKey> {
        StackKey::of_tree(project, pixels, baked, images, ts, roots.to_vec(), &project.render_tree(roots))
    }

    /// The key of the layers in `tree` (and their effects), drawn as `roots` arrange them.
    fn of_tree(
        project: &Project,
        pixels: &PixelStore,
        baked: &BakedMaps,
        images: &ImageStore,
        ts: usize,
        roots: Vec<LayerId>,
        tree: &BTreeSet<LayerId>,
    ) -> Option<StackKey> {
        let mut set = project.texture_set(ts).ok()?.clone();
        set.name.clear();
        set.description.clear();
        set.stack.clear();
        set.bake = Default::default();
        let effects: BTreeSet<LayerId> = tree.iter().filter_map(|id| project.layers.get(id)).flat_map(effect_ids).collect();
        let pixels = tree
            .iter()
            .chain(&effects)
            .flat_map(|id| {
                let layer = Channel::ALL.into_iter().filter_map(move |c| pixels.layer(*id, c).map(|img| (*id, Some(c), Shared(img.clone()))));
                layer.chain(pixels.masks.get(id).map(|img| (*id, None, Shared(img.clone()))))
            })
            .collect();
        let mut images: Vec<(String, usize)> = images.iter().map(|(n, i)| (n.clone(), Arc::as_ptr(i) as usize)).collect();
        images.sort();
        Some(StackKey {
            set,
            roots,
            layers: tree.iter().filter_map(|id| project.layers.get(id)).map(render_view).collect(),
            pixels,
            baked: (baked.resolution, baked.maps.iter().map(|(m, d)| (*m, Arc::as_ptr(d) as usize)).collect()),
            images,
        })
    }
}

/// An image compared by tile identity: holding the clone keeps its tiles alive, and a tile
/// rewritten since is a different allocation (copy-on-write), so pointer equality means the
/// same pixels without reading them.
#[derive(Debug)]
struct Shared(TileImage);

impl PartialEq for Shared {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (&self.0, &other.0);
        a.width() == b.width()
            && a.height() == b.height()
            && a.comps() == b.comps()
            && (0..a.tiles_y()).all(|ty| {
                (0..a.tiles_x()).all(|tx| match (a.tile_handle(tx, ty), b.tile_handle(tx, ty)) {
                    (None, None) => true,
                    (Some(x), Some(y)) => Arc::ptr_eq(x, y),
                    _ => false,
                })
            })
    }
}

/// Ids of a layer's mask and content effects.
fn effect_ids(l: &Layer) -> impl Iterator<Item = LayerId> + '_ {
    l.mask.iter().flat_map(|m| m.effects.iter()).chain(l.content_effects.iter()).map(|e| e.id)
}

/// A layer as far as compositing is concerned (names, color tags and folding don't matter).
fn render_view(l: &Layer) -> Layer {
    let mut l = l.clone();
    l.name.clear();
    l.color_tag = None;
    if let LayerKind::Group { collapsed, .. } = &mut l.kind {
        *collapsed = false;
    }
    l
}

impl BelowCache {
    /// Cache the stack around `hot` from now on (a different layer empties the cache).
    pub fn set_hot(&mut self, hot: Option<LayerId>) {
        if self.hot != hot {
            self.hot = hot;
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        let tiles = std::mem::take(&mut self.tiles);
        // Freeing a full cache (up to the byte budget in small bands) can take most of a
        // second: a thread does it so the frame that switched the hot layer doesn't stall.
        #[cfg(not(target_arch = "wasm32"))]
        if !tiles.is_empty() {
            let _ = std::thread::Builder::new().name("texturecraft-free".into()).spawn(move || drop(tiles));
        }
        #[cfg(target_arch = "wasm32")]
        drop(tiles);
        self.order.clear();
        self.bytes = 0;
        self.key = None;
        // Held fields tell the hot layer's tiles from the others'
        self.held = None;
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Whether neighbourhood-filter fields are kept for the composites that follow (see [`BelowCache::ahead`]).
    pub fn holds_fields(&self) -> bool {
        self.held.is_some()
    }

    /// The byte limit: the budget for the top level and as much again for a hot layer nested in
    /// folders (each tile then holds the stack at two or more levels).
    pub fn limit(&self) -> usize {
        self.budget.saturating_mul(self.levels.clamp(1, 2))
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Bytes of an average cached tile (None while empty).
    pub fn tile_bytes(&self) -> Option<usize> {
        self.bytes.checked_div(self.tiles.len())
    }

    pub(crate) fn tile(&self, t: (u32, u32)) -> Option<&[Band]> {
        self.tiles.get(&t).map(|(_, b)| b.as_slice())
    }

    /// Mark tile `t` used now (it is evicted last).
    pub(crate) fn touch(&mut self, t: (u32, u32)) {
        let Some((stamp, _)) = self.tiles.get_mut(&t) else { return };
        self.clock += 1;
        *stamp = self.clock;
        self.order.push_back((t, self.clock));
        // Drop superseded uses once they outnumber the tiles.
        if self.order.len() > 2 * self.tiles.len() + 64 {
            let tiles = &self.tiles;
            self.order.retain(|(t, s)| tiles.get(t).is_some_and(|(stamp, _)| stamp == s));
        }
    }

    pub(crate) fn insert(&mut self, t: (u32, u32), mut bands: Vec<Band>) {
        for b in &mut bands {
            b.shrink_to_fit();
        }
        let size: usize = bands.iter().map(band_bytes).sum();
        if size > self.limit() {
            return;
        }
        while self.bytes + size > self.limit() {
            let Some((old, stamp)) = self.order.pop_front() else { break };
            if self.tiles.get(&old).is_some_and(|(s, _)| *s == stamp)
                && let Some((_, d)) = self.tiles.remove(&old)
            {
                self.bytes = self.bytes.saturating_sub(d.iter().map(band_bytes).sum());
            }
        }
        self.clock += 1;
        if let Some((_, d)) = self.tiles.insert(t, (self.clock, bands)) {
            self.bytes = self.bytes.saturating_sub(d.iter().map(band_bytes).sum());
        }
        self.order.push_back((t, self.clock));
        self.bytes += size;
    }

    /// The path to the hot layer for `inp`: everything off the path is served from the cache.
    /// None when the hot layer is not drawn in the set (or drawn twice), when there is nothing
    /// to cache, or when the cached part could change (anchors read the stack, or it holds the
    /// live stroke). Empties the cache when what the cached part reads changed.
    pub(crate) fn prepare(&mut self, inp: &Inputs) -> Option<Vec<Level>> {
        let hot = self.hot?;
        let p = inp.project;
        let set = p.texture_set(inp.ts).ok()?;
        // Anchor readers depend on the stack below them.
        if p.render_tree(&set.stack).iter().any(|id| p.layers.get(id).is_some_and(Layer::reads_anchor)) {
            return None;
        }
        let levels = path(p, &set.stack, hot)?;
        let rest: Vec<LayerId> = levels.iter().flat_map(|l| l.below.iter().chain(&l.above)).copied().collect();
        if rest.is_empty() && levels.len() == 1 {
            return None;
        }
        let mut tree = p.render_tree(&rest);
        // Folders holding the hot layer (and the folders their instances draw) count with their
        // own state only, not their children.
        for l in levels.iter().take(levels.len() - 1) {
            let mut id = Some(l.at);
            for _ in 0..=MAX_DEPTH {
                let Some(i) = id.filter(|i| tree.insert(*i)) else { break };
                id = p.layers.get(&i).and_then(Layer::instance_source);
            }
        }
        if tree.contains(&hot) {
            return None;
        }
        if let Some(pv) = inp.preview {
            let (PaintTarget::Layer(t) | PaintTarget::Mask(t)) = pv.target;
            if tree.contains(&t) || tree.iter().filter_map(|id| p.layers.get(id)).flat_map(effect_ids).any(|e| e == t) {
                return None;
            }
        }
        let roots = set.stack.iter().chain(levels.iter().map(|l| &l.at)).copied().collect();
        let key = StackKey::of_tree(p, inp.pixels, inp.baked, inp.images, inp.ts, roots, &tree)?;
        self.levels = levels.len();
        if self.key.as_ref() != Some(&key) {
            self.clear();
            self.key = Some(key);
        }
        Some(levels)
    }
}

/// The levels from the top of `stack` down to the layer drawing `hot` (the first one
/// compositing meets), through folders and instanced folders. None when `hot` is not drawn or is
/// nested deeper than compositing goes.
fn path(p: &Project, stack: &[LayerId], hot: LayerId) -> Option<Vec<Level>> {
    let mut levels = Vec::new();
    let mut list = stack;
    for _ in 0..=MAX_DEPTH {
        let i = list.iter().position(|id| p.render_tree(&[*id]).contains(&hot))?;
        let at = *list.get(i)?;
        levels.push(Level { below: list.get(..i)?.to_vec(), at, above: list.get(i + 1..)?.to_vec() });
        let content = p.content_layer(at)?;
        if at == hot || content.id == hot || !content.is_group() {
            return Some(levels);
        }
        list = content.children();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_keeps_to_the_budget_dropping_the_least_recently_used() {
        let mut c = BelowCache { budget: 3 * 16, ..BelowCache::default() };
        let band = |n: usize| vec![vec![0.0; n]];
        for i in 0..5 {
            c.insert((i, 0), band(4));
        }
        assert_eq!(c.len(), 3);
        assert_eq!(c.bytes(), 48);
        assert_eq!(c.tile_bytes(), Some(16));
        assert!(c.tile((0, 0)).is_none() && c.tile((4, 0)).is_some());
        // A used tile outlives newer ones; superseded uses stay bounded.
        for _ in 0..100 {
            c.touch((2, 0));
        }
        assert!(c.order.len() <= 2 * c.len() + 65);
        c.insert((5, 0), band(4));
        c.insert((6, 0), band(4));
        assert!(c.tile((2, 0)).is_some() && c.tile((3, 0)).is_none() && c.tile((4, 0)).is_none());
        assert_eq!(c.bytes(), 48);
        c.insert((9, 9), band(100));
        assert!(c.tile((9, 9)).is_none(), "bigger than the whole budget");
        c.set_hot(Some(LayerId(3)));
        assert!(c.is_empty() && c.bytes() == 0);
    }
}
