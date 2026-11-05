//! Pixel data owned by a document: painted layer channels, painted mask effects and baked mesh
//! maps. Everything is copy-on-write, so cloning the store for undo is cheap.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use texturecraft_core::{BakedMap, Channel, ChannelKind, LayerId};
use texturecraft_texel::{TileImage, area_downsample, from_u16, from_u16_signed};

/// Painted data of one paint layer channel: premultiplied value(s) plus coverage, u16 per
/// component (height stored signed).
#[derive(Clone, Debug, Default)]
pub struct PixelStore {
    /// (paint layer or content paint effect, channel) → texels.
    pub layers: HashMap<(LayerId, Channel), TileImage>,
    /// Paint effect of a mask → (premultiplied value, coverage).
    pub masks: HashMap<LayerId, TileImage>,
}

impl PixelStore {
    pub fn layer(&self, id: LayerId, c: Channel) -> Option<&TileImage> {
        self.layers.get(&(id, c))
    }

    /// The image for `(id, c)` at `size` (width, height), created empty if missing or of
    /// another size.
    pub fn layer_mut(&mut self, id: LayerId, c: Channel, size: (u32, u32)) -> &mut TileImage {
        sized(self.layers.entry((id, c)).or_insert_with(|| TileImage::new(size.0, size.1, c.components() + 1)), size)
    }

    pub fn mask_mut(&mut self, id: LayerId, size: (u32, u32)) -> &mut TileImage {
        sized(self.masks.entry(id).or_insert_with(|| TileImage::new(size.0, size.1, 2)), size)
    }

    /// Drop pixels of layers that no longer exist.
    pub fn retain(&mut self, alive: impl Fn(LayerId) -> bool) {
        self.layers.retain(|(id, _), _| alive(*id));
        self.masks.retain(|id, _| alive(*id));
    }

    pub fn bytes(&self) -> usize {
        self.layers.values().chain(self.masks.values()).map(TileImage::bytes).sum()
    }
}

/// `img`, replaced by an empty image of its components when it is not `size`.
fn sized(img: &mut TileImage, size: (u32, u32)) -> &mut TileImage {
    if (img.width(), img.height()) != size {
        *img = TileImage::new(size.0, size.1, img.comps());
    }
    img
}

/// Decode a stored component of channel kind `kind` to its float value.
#[inline]
pub fn decode(kind: ChannelKind, v: u16) -> f32 {
    if kind == ChannelKind::Signed { from_u16_signed(v) } else { from_u16(v) }
}

/// Baked mesh maps of one texture set (dense, row-major, `components()` floats per texel). The
/// width is `resolution`; height follows the output size and UV tile grid
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedMaps {
    /// Width in texels.
    pub resolution: u32,
    pub maps: BTreeMap<BakedMap, Arc<Vec<f32>>>,
}

impl BakedMaps {
    /// Height in texels of map `m` (from its length).
    pub fn height(&self, m: BakedMap) -> u32 {
        self.maps.get(&m).map_or(0, |d| (d.len() / (self.resolution.max(1) as usize * m.components())) as u32)
    }

    /// The texel of map `map` that texel (`x`, `y`) of an `output_size` image stretched over the
    /// map falls on (nearest, no filtering; see [`BakedMaps::view`] for reads below the baked
    /// resolution).
    pub fn sample(&self, map: BakedMap, x: u32, y: u32, output_size: (u32, u32)) -> Option<[f32; 3]> {
        let data = self.maps.get(&map)?;
        nearest(data, (self.resolution.max(1), self.height(map).max(1)), map.components(), (x, y), output_size)
    }

    pub fn scalar(&self, m: BakedMap, x: u32, y: u32, size: (u32, u32)) -> Option<f32> {
        self.sample(m, x, y, size).map(|v| v[0])
    }

    /// The maps as a reader of `size` (width, height) texels sees them: each of `filtered` that
    /// is larger than `size` along an axis is area-averaged down along it (so a thin crease
    /// baked at 8K keeps its share of every texel of a 2K read instead of being hit or
    /// missed), every other read is [`BakedMaps::sample`]'s. ID maps are never averaged: their
    /// colours are labels, and a mix of two would match neither.
    ///
    /// The averaged maps are cached by their data, so a view per composite pass, tile or frame
    /// costs a lookup once the first one has been made. Make the view before a parallel loop
    /// rather than inside it: concurrent first views of a map would each average it.
    pub fn view(&self, size: (u32, u32), filtered: &[BakedMap]) -> BakedView<'_> {
        let size = (size.0.max(1), size.1.max(1));
        let mut levels: Vec<(BakedMap, Level)> = Vec::new();
        for &m in filtered {
            if m == BakedMap::Id || levels.iter().any(|(l, _)| *l == m) {
                continue;
            }
            let Some(data) = self.maps.get(&m) else { continue };
            let from = (self.resolution, self.height(m));
            let to = (from.0.min(size.0), from.1.min(size.1));
            // Not smaller on either axis: nearest reads are exact already.
            if to == from {
                continue;
            }
            if let Some(data) = cached_level(data, from, m.components(), to) {
                levels.push((m, Level { data, size: to }));
            }
        }
        BakedView { maps: self, size, levels }
    }
}

/// [`BakedMaps`] read at one size (from [`BakedMaps::view`]).
#[derive(Debug)]
pub struct BakedView<'a> {
    maps: &'a BakedMaps,
    size: (u32, u32),
    /// Maps averaged down to (at most) `size`.
    levels: Vec<(BakedMap, Level)>,
}

/// A baked map area-averaged down to `size`.
#[derive(Debug)]
struct Level {
    data: Arc<Vec<f32>>,
    size: (u32, u32),
}

impl BakedView<'_> {
    /// Map `m` at texel (`x`, `y`) of the view's size: the average of the baked texels it
    /// covers when the map was averaged for the view, the nearest baked texel otherwise.
    pub fn sample(&self, m: BakedMap, x: u32, y: u32) -> Option<[f32; 3]> {
        match self.levels.iter().find(|(l, _)| *l == m) {
            Some((_, level)) => nearest(&level.data, level.size, m.components(), (x, y), self.size),
            None => self.maps.sample(m, x, y, self.size),
        }
    }

    pub fn scalar(&self, m: BakedMap, x: u32, y: u32) -> Option<f32> {
        self.sample(m, x, y).map(|v| v[0])
    }
}

/// The texel of a `size` image of `comps` floats per texel under texel `at` of an `output`
/// image stretched over it.
fn nearest(data: &[f32], size: (u32, u32), comps: usize, at: (u32, u32), output: (u32, u32)) -> Option<[f32; 3]> {
    let (width, height) = (u64::from(size.0.max(1)), u64::from(size.1.max(1)));
    let x = (u64::from(at.0) * width / u64::from(output.0.max(1))).min(width - 1);
    let y = (u64::from(at.1) * height / u64::from(output.1.max(1))).min(height - 1);
    let offset = usize::try_from(y * width + x).ok()?.checked_mul(comps)?;
    match *data.get(offset..offset.checked_add(comps)?)? {
        [r, g, b] => Some([r, g, b]),
        [v, ..] => Some([v; 3]),
        [] => None,
    }
}

/// An averaged map kept for reuse: `source` (`from` texels, `comps` floats each) averaged down
/// to `to`. The source is held weakly, so a level dies with its bake, and its address can't be
/// reused by another while the entry exists.
struct CachedLevel {
    source: Weak<Vec<f32>>,
    from: (u32, u32),
    comps: usize,
    to: (u32, u32),
    level: Arc<Vec<f32>>,
}

/// Levels kept at most: a few readers' sizes (composite, mesh-map view) per map of the live
/// bakes; the least recently used goes first.
const MAX_LEVELS: usize = 32;

/// Averaged maps by source data, shared by every [`BakedMaps`] holding that data (undo
/// snapshots and the document alike).
static LEVELS: Mutex<Vec<CachedLevel>> = Mutex::new(Vec::new());

/// `source` averaged down to `to` (see [`area_downsample`]), from the cache when it was
/// computed before. None when the source does not hold `from` texels.
fn cached_level(source: &Arc<Vec<f32>>, from: (u32, u32), comps: usize, to: (u32, u32)) -> Option<Arc<Vec<f32>>> {
    let find = |levels: &mut Vec<CachedLevel>| {
        levels.retain(|c| c.source.strong_count() > 0);
        let i = levels.iter().position(|c| std::ptr::eq(c.source.as_ptr(), Arc::as_ptr(source)) && (c.from, c.comps, c.to) == (from, comps, to))?;
        // Most recently used last.
        let hit = levels.remove(i);
        let level = Arc::clone(&hit.level);
        levels.push(hit);
        Some(level)
    };
    let lock = || LEVELS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(level) = find(&mut lock()) {
        return Some(level);
    }
    // Averaged without the lock: the work runs on the thread pool, where a task waiting on
    // the lock could deadlock the thread holding it.
    let level = Arc::new(area_downsample(source, from, comps, to)?);
    let mut levels = lock();
    if let Some(raced) = find(&mut levels) {
        return Some(raced);
    }
    levels.push(CachedLevel { source: Arc::downgrade(source), from, comps, to, level: Arc::clone(&level) });
    if levels.len() > MAX_LEVELS {
        levels.remove(0);
    }
    Some(level)
}

/// A decoded image resource sampled by fills (values in the file's encoding; see `srgb`).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRes {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[f32; 4]>,
    /// 8/16-bit images are sRGB-encoded; color channels linearize them.
    pub srgb: bool,
}

/// Image resources by name.
pub type ImageStore = HashMap<String, Arc<ImageRes>>;

impl ImageRes {
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let (w, h) = (self.width.max(1) as i64, self.height.max(1) as i64);
        let i = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize;
        self.rgba.get(i).copied().unwrap_or([0.0; 4])
    }

    /// Bilinear sample with wrap-around (texture coordinates, v down).
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let (a, b, c, d) = (self.texel(x0, y0), self.texel(x0 + 1, y0), self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            out[k] = top + (bot - top) * ty;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `res.0 × res.1` texels of map `m`, `f(x, y)` in every component.
    fn bake(m: BakedMap, res: (u32, u32), f: impl Fn(u32, u32) -> f32) -> BakedMaps {
        let data = (0..res.0 * res.1).flat_map(|i| std::iter::repeat_n(f(i % res.0, i / res.0), m.components())).collect();
        BakedMaps { resolution: res.0, maps: [(m, Arc::new(data))].into() }
    }

    /// A 1-texel line every 8 rows.
    fn lines(_: u32, y: u32) -> f32 {
        if y % 8 == 3 { 1.0 } else { 0.0 }
    }

    #[test]
    fn views_average_larger_bakes() {
        // 256×512 read at 32×64: 8× on both axes, so every texel covers one line row in eight.
        let b = bake(BakedMap::Curvature, (256, 512), lines);
        let v = b.view((32, 64), &[BakedMap::Curvature]);
        for (x, y) in [(0, 0), (31, 63), (7, 20)] {
            assert_eq!(v.scalar(BakedMap::Curvature, x, y), Some(0.125), "({x}, {y})");
        }
        // Nearest picks row 8y: never on the line.
        assert_eq!(b.scalar(BakedMap::Curvature, 0, 0, (32, 64)), Some(0.0));
        // Three components: each averaged.
        let n = bake(BakedMap::Normal, (64, 64), lines);
        assert_eq!(n.view((8, 8), &[BakedMap::Normal]).sample(BakedMap::Normal, 2, 5), Some([0.125; 3]));
        // A map left out of the view is read as before.
        assert_eq!(b.view((32, 64), &[]).scalar(BakedMap::Curvature, 0, 0), Some(0.0));
    }

    #[test]
    fn views_keep_reads_at_the_same_size_or_larger() {
        let b = bake(BakedMap::AmbientOcclusion, (16, 16), |x, y| (y * 16 + x) as f32);
        for size in [(16, 16), (32, 64)] {
            let v = b.view(size, &[BakedMap::AmbientOcclusion]);
            for (x, y) in (0..size.1).flat_map(|y| (0..size.0).map(move |x| (x, y))) {
                assert_eq!(v.scalar(BakedMap::AmbientOcclusion, x, y), b.scalar(BakedMap::AmbientOcclusion, x, y, size));
            }
        }
        // Shrinking one axis averages it and keeps the other's nearest reads: 16 → 4 across.
        let v = b.view((4, 32), &[BakedMap::AmbientOcclusion]);
        assert_eq!(v.scalar(BakedMap::AmbientOcclusion, 1, 2), Some(16.0 + 5.5));
    }

    #[test]
    fn id_maps_are_never_averaged() {
        let b = bake(BakedMap::Id, (64, 64), |x, _| if x % 2 == 0 { 1.0 } else { 0.0 });
        let v = b.view((8, 8), &[BakedMap::Id]);
        assert_eq!(v.sample(BakedMap::Id, 3, 3), b.sample(BakedMap::Id, 3, 3, (8, 8)));
        assert_eq!(v.sample(BakedMap::Id, 3, 3), Some([1.0; 3]));
    }

    #[test]
    fn levels_are_cached_by_their_data() {
        let b = bake(BakedMap::Thickness, (128, 128), lines);
        let level = |b: &BakedMaps| b.view((16, 16), &[BakedMap::Thickness]).levels.first().map(|(_, l)| Arc::as_ptr(&l.data));
        let first = level(&b);
        assert!(first.is_some());
        // A clone (an undo snapshot) shares the level; new data gets its own.
        assert_eq!(level(&b.clone()), first);
        assert_ne!(level(&bake(BakedMap::Thickness, (128, 128), lines)), first);
        // A map whose data does not match its size is read nearest, not averaged.
        let mut short = b.clone();
        short.maps.insert(BakedMap::Thickness, Arc::new(vec![1.0; 100]));
        assert!(short.view((16, 16), &[BakedMap::Thickness]).levels.is_empty());
    }
}
