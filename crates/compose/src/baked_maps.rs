use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use texturecraft_core::{BakedMap, BlockScales};
use texturecraft_texel::{TILE, area_downsample};

/// Dense mesh maps with `components()` floats per texel in row-major order
/// Each UV tile fills its block independently of the working resolution
/// The width is `resolution`; height follows the output size and UV tile grid
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedMaps {
    /// Width in texels
    pub resolution: u32,
    /// Replace a map's data, never mutate it in place: averaged levels are cached by the
    /// address of the data (held weakly), see [`BakedMaps::view`].
    pub maps: BTreeMap<BakedMap, Arc<Vec<f32>>>,
}

impl BakedMaps {
    /// Sample a working atlas texel without resampling the stored bake (nearest; see
    /// [`BakedMaps::view`] for reads below the baked resolution)
    pub fn sample_working(&self, map: BakedMap, x: u32, y: u32, size: (u32, u32), scales: &BlockScales) -> Option<[f32; 3]> {
        let uv = [(x as f32 + 0.5) / size.0.max(1) as f32, (y as f32 + 0.5) / size.1.max(1) as f32];
        self.sample_uv(map, scales.bake_uv(uv, size)?)
    }

    /// Nearest sample in the uniform bake atlas
    pub fn sample_uv(&self, map: BakedMap, uv: [f32; 2]) -> Option<[f32; 3]> {
        if uv.iter().any(|value| !value.is_finite()) {
            return None;
        }

        let size = (self.resolution, self.height(map));
        self.sample(map, (uv[0] * size.0 as f32) as u32, (uv[1] * size.1 as f32) as u32, size)
    }

    /// Height in texels of map `m`, from its length
    pub fn height(&self, m: BakedMap) -> u32 {
        self.maps.get(&m).map_or(0, |d| (d.len() / (self.resolution.max(1) as usize * m.components())) as u32)
    }

    /// Sample the corresponding texel of an image with the supplied output size
    pub fn sample(&self, map: BakedMap, x: u32, y: u32, output_size: (u32, u32)) -> Option<[f32; 3]> {
        let data = self.maps.get(&map)?;
        let width = u64::from(self.resolution.max(1));
        let height = u64::from(self.height(map).max(1));
        let sample_x = (u64::from(x) * width / u64::from(output_size.0.max(1))).min(width - 1);
        let sample_y = (u64::from(y) * height / u64::from(output_size.1.max(1))).min(height - 1);
        let component_count = map.components();
        let offset = ((sample_y * width + sample_x) as usize).checked_mul(component_count)?;
        let texel = data.get(offset..offset.checked_add(component_count)?)?;
        Some(if component_count == 3 { [texel[0], texel[1], texel[2]] } else { [texel[0]; 3] })
    }

    pub fn scalar(&self, m: BakedMap, x: u32, y: u32, size: (u32, u32)) -> Option<f32> {
        self.sample(m, x, y, size).map(|v| v[0])
    }

    /// The maps as a reader of `read` (width, height) texels stretched over a working atlas of
    /// `working` texels laid out by `scales` sees them (`read == working` for every reader but
    /// the mesh-map view). Each UV tile's block of each of `filtered` that the reader samples
    /// more coarsely than it was baked, along an axis, is area-averaged down along it to the
    /// reader's texels there, so a thin crease baked at 8K keeps its share of every texel of a
    /// 2K read instead of being hit or missed. Every other read is
    /// [`BakedMaps::sample_working`]'s. ID maps are never averaged: their colours are labels,
    /// and a mix of two would match neither. Averaged directions (normals, bent normals) are
    /// renormalised to unit length.
    ///
    /// The averaged maps are cached by their data, so a view per composite pass, tile or frame
    /// costs a lookup once the first one has been made. Make the view before a parallel loop
    /// rather than inside it: concurrent first views of a map would each average it.
    pub fn view<'a>(&'a self, read: (u32, u32), working: (u32, u32), scales: &'a BlockScales, filtered: &[BakedMap]) -> BakedView<'a> {
        let read = (read.0.max(1), read.1.max(1));
        let tile_scales = tile_scales(scales, working);
        let mut levels: Vec<Level> = Vec::new();
        for &map in filtered {
            if map == BakedMap::Id || levels.iter().any(|l| l.map == map) {
                continue;
            }
            let Some(data) = self.maps.get(&map) else { continue };
            let from = (self.resolution, self.height(map));
            for &scale in &tile_scales {
                let to = (level_axis(from.0, read.0, scale), level_axis(from.1, read.1, scale));
                // Not coarser on either axis: nearest reads are exact already.
                if to == from {
                    continue;
                }
                if let Some(data) = cached_level(data, Shape { from, comps: map.components(), to, directions: is_direction(map) }) {
                    levels.push(Level { map, scale, data, size: to });
                }
            }
        }
        BakedView { maps: self, scales, read, working, levels }
    }
}

/// Texels along one axis of a level of a bake `bake` texels long, for a reader of `read` texels
/// over the working atlas in the blocks whose tiles fill `scale` of them: as many texels per
/// block as the reader samples there (block boundaries stay on texel boundaries, as each
/// block's tile is a whole number of working texels), or `bake` when the reader is not coarser.
fn level_axis(bake: u32, read: u32, scale: f32) -> u32 {
    // Saturating: a NaN scale reads 0 texels, clamped to 1.
    ((read as f32 * scale).round() as u32).clamp(1, bake.max(1))
}

/// Every tile scale of `scales` over a `working` atlas, each once. Blocks are whole tiles of
/// `TILE` texels, so one probe per tile finds them all.
fn tile_scales(scales: &BlockScales, working: (u32, u32)) -> Vec<f32> {
    if *scales == BlockScales::default() {
        return vec![1.0];
    }
    let mut out: Vec<f32> = Vec::new();
    for ty in 0..working.1.div_ceil(TILE) {
        for tx in 0..working.0.div_ceil(TILE) {
            let scale = scales.at(tx * TILE, ty * TILE);
            if !out.iter().any(|s| s.to_bits() == scale.to_bits()) {
                out.push(scale);
            }
        }
    }
    out
}

/// Maps holding unit directions, packed as `v · 0.5 + 0.5`.
fn is_direction(map: BakedMap) -> bool {
    matches!(map, BakedMap::Normal | BakedMap::WorldNormal | BakedMap::BentNormal)
}

/// Packed directions (`v · 0.5 + 0.5`, 3 floats each) scaled back to unit length; ones that
/// average out to nothing are left as they are.
fn renormalise(packed: &mut [f32]) {
    for texel in packed.as_chunks_mut::<3>().0 {
        let v = texel.map(|c| c * 2.0 - 1.0);
        let len = v.iter().map(|c| c * c).sum::<f32>().sqrt();
        if len > 1e-6 {
            *texel = v.map(|c| (c / len * 0.5 + 0.5).clamp(0.0, 1.0));
        }
    }
}

/// [`BakedMaps`] read at one size (from [`BakedMaps::view`]).
#[derive(Debug)]
pub struct BakedView<'a> {
    maps: &'a BakedMaps,
    scales: &'a BlockScales,
    /// Reader texels across and down.
    read: (u32, u32),
    /// The working atlas `scales` lays out.
    working: (u32, u32),
    /// Maps averaged down for the blocks of one tile scale.
    levels: Vec<Level>,
}

/// Baked map `map` area-averaged down to `size`, read in the blocks whose tiles have scale
/// `scale`.
#[derive(Debug)]
struct Level {
    map: BakedMap,
    scale: f32,
    data: Arc<Vec<f32>>,
    size: (u32, u32),
}

impl BakedView<'_> {
    /// Map `m` at texel (`x`, `y`) of the reader: the average of the baked texels it covers
    /// when the map was averaged for its block, the nearest baked texel otherwise.
    pub fn sample(&self, m: BakedMap, x: u32, y: u32) -> Option<[f32; 3]> {
        let uv = [(x as f32 + 0.5) / self.read.0 as f32, (y as f32 + 0.5) / self.read.1 as f32];
        let bake_uv = self.scales.bake_uv(uv, self.working)?;
        match self.level(m, uv) {
            Some(level) => nearest_uv(&level.data, level.size, m.components(), bake_uv),
            None => self.maps.sample_uv(m, bake_uv),
        }
    }

    pub fn scalar(&self, m: BakedMap, x: u32, y: u32) -> Option<f32> {
        self.sample(m, x, y).map(|v| v[0])
    }

    /// The level of `m` for the block under working atlas UV `uv`.
    fn level(&self, m: BakedMap, uv: [f32; 2]) -> Option<&Level> {
        if self.levels.is_empty() {
            return None;
        }
        let scale = self.scales.at((uv[0] * self.working.0 as f32) as u32, (uv[1] * self.working.1 as f32) as u32);
        self.levels.iter().find(|l| l.map == m && l.scale.to_bits() == scale.to_bits())
    }
}

/// The texel of a `size` image of `comps` floats per texel under `uv` (in 0..1).
fn nearest_uv(data: &[f32], size: (u32, u32), comps: usize, uv: [f32; 2]) -> Option<[f32; 3]> {
    let x = ((uv[0] * size.0 as f32) as u32).min(size.0.saturating_sub(1));
    let y = ((uv[1] * size.1 as f32) as u32).min(size.1.saturating_sub(1));
    let offset = usize::try_from(u64::from(y) * u64::from(size.0) + u64::from(x)).ok()?.checked_mul(comps)?;
    match *data.get(offset..offset.checked_add(comps)?)? {
        [r, g, b] => Some([r, g, b]),
        [v, ..] => Some([v; 3]),
        [] => None,
    }
}

/// How a level is made: `from` texels of `comps` floats averaged down to `to`, renormalised
/// when the map holds `directions`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    from: (u32, u32),
    comps: usize,
    to: (u32, u32),
    directions: bool,
}

/// An averaged map kept for reuse. The source is held weakly, so a level dies with its bake,
/// and its address can't be reused by another while the entry exists.
struct CachedLevel {
    source: Weak<Vec<f32>>,
    shape: Shape,
    level: Arc<Vec<f32>>,
}

impl CachedLevel {
    fn bytes(&self) -> usize {
        self.level.len() * size_of::<f32>()
    }
}

/// Bytes of levels kept at most: the live levels of an 8K bake read at 4K (AO, curvature,
/// thickness and normal) take about 400 MiB; undo snapshots keep older bakes alive.
const MAX_LEVEL_BYTES: usize = 1 << 30;

/// Averaged maps by source data, least recently used first.
#[derive(Default)]
struct LevelCache {
    entries: Vec<CachedLevel>,
}

impl LevelCache {
    /// The level of `source` shaped `shape`, marked most recently used.
    fn find(&mut self, source: &Arc<Vec<f32>>, shape: Shape) -> Option<Arc<Vec<f32>>> {
        self.entries.retain(|c| c.source.strong_count() > 0);
        let i = self.entries.iter().position(|c| std::ptr::eq(c.source.as_ptr(), Arc::as_ptr(source)) && c.shape == shape)?;
        let hit = self.entries.remove(i);
        let level = Arc::clone(&hit.level);
        self.entries.push(hit);
        Some(level)
    }

    /// Keep `level`, then drop the least recently used levels until the rest fit in `budget`
    /// bytes (the newest stays even when it alone does not).
    fn insert(&mut self, source: &Arc<Vec<f32>>, shape: Shape, level: &Arc<Vec<f32>>, budget: usize) {
        self.entries.push(CachedLevel { source: Arc::downgrade(source), shape, level: Arc::clone(level) });
        let mut total: usize = self.entries.iter().map(CachedLevel::bytes).sum();
        while total > budget && self.entries.len() > 1 {
            total -= self.entries.remove(0).bytes();
        }
    }
}

/// Averaged maps shared by every [`BakedMaps`] holding their source data (undo snapshots and
/// the document alike).
static LEVELS: Mutex<LevelCache> = Mutex::new(LevelCache { entries: Vec::new() });

/// `source` averaged down as `shape` says (see [`area_downsample`]), from the cache when it was
/// computed before. None when the source does not hold `shape.from` texels.
fn cached_level(source: &Arc<Vec<f32>>, shape: Shape) -> Option<Arc<Vec<f32>>> {
    let lock = || LEVELS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(level) = lock().find(source, shape) {
        return Some(level);
    }
    // Averaged without the lock: the work runs on the thread pool, where a task waiting on
    // the lock could deadlock the thread holding it.
    let mut level = area_downsample(source, shape.from, shape.comps, shape.to)?;
    if shape.directions {
        renormalise(&mut level);
    }
    let level = Arc::new(level);
    let mut cache = lock();
    if let Some(raced) = cache.find(source, shape) {
        return Some(raced);
    }
    cache.insert(source, shape, &level, MAX_LEVEL_BYTES);
    Some(level)
}

#[cfg(test)]
mod tests {
    use texturecraft_core::TextureSet;

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

    /// `b` read by a reader of `size`, the working atlas of a single uniform tile.
    fn uniform<'a>(b: &'a BakedMaps, size: (u32, u32), filtered: &[BakedMap]) -> BakedView<'a> {
        b.view(size, size, Box::leak(Box::default()), filtered)
    }

    #[test]
    fn views_average_larger_bakes() {
        // 256×512 read at 32×64: 8× on both axes, so every texel covers one line row in eight.
        let b = bake(BakedMap::Curvature, (256, 512), lines);
        let v = uniform(&b, (32, 64), &[BakedMap::Curvature]);
        for (x, y) in [(0, 0), (31, 63), (7, 20)] {
            assert_eq!(v.scalar(BakedMap::Curvature, x, y), Some(0.125), "({x}, {y})");
        }
        // Nearest picks one row in eight: never on the line.
        assert_eq!(b.sample_working(BakedMap::Curvature, 0, 0, (32, 64), &BlockScales::default()), Some([0.0; 3]));
        // Three components: each averaged.
        let p = bake(BakedMap::Position, (64, 64), lines);
        assert_eq!(uniform(&p, (8, 8), &[BakedMap::Position]).sample(BakedMap::Position, 2, 5), Some([0.125; 3]));
        // A map left out of the view is read as before.
        assert_eq!(uniform(&b, (32, 64), &[]).scalar(BakedMap::Curvature, 0, 0), Some(0.0));
    }

    #[test]
    fn views_keep_reads_at_the_same_size_or_larger() {
        let b = bake(BakedMap::AmbientOcclusion, (16, 16), |x, y| (y * 16 + x) as f32);
        let none = BlockScales::default();
        for size in [(16, 16), (32, 64)] {
            let v = uniform(&b, size, &[BakedMap::AmbientOcclusion]);
            for (x, y) in (0..size.1).flat_map(|y| (0..size.0).map(move |x| (x, y))) {
                assert_eq!(v.sample(BakedMap::AmbientOcclusion, x, y), b.sample_working(BakedMap::AmbientOcclusion, x, y, size, &none));
            }
        }
        // Shrinking one axis averages it and keeps the other's nearest reads: 16 → 4 across.
        let v = uniform(&b, (4, 32), &[BakedMap::AmbientOcclusion]);
        assert_eq!(v.scalar(BakedMap::AmbientOcclusion, 1, 2), Some(16.0 + 5.5));
    }

    #[test]
    fn id_maps_are_never_averaged() {
        let b = bake(BakedMap::Id, (64, 64), |x, _| if x % 2 == 0 { 1.0 } else { 0.0 });
        let v = uniform(&b, (8, 8), &[BakedMap::Id]);
        assert_eq!(v.sample(BakedMap::Id, 3, 3), b.sample_working(BakedMap::Id, 3, 3, (8, 8), &BlockScales::default()));
        assert!(v.levels.is_empty());
    }

    #[test]
    fn levels_are_cached_by_their_data() {
        let b = bake(BakedMap::Thickness, (128, 128), lines);
        let level = |b: &BakedMaps| uniform(b, (16, 16), &[BakedMap::Thickness]).levels.first().map(|l| Arc::as_ptr(&l.data));
        let first = level(&b);
        assert!(first.is_some());
        // A clone (an undo snapshot) shares the level; new data gets its own.
        assert_eq!(level(&b.clone()), first);
        assert_ne!(level(&bake(BakedMap::Thickness, (128, 128), lines)), first);
        // A map whose data does not match its size is read nearest, not averaged.
        let mut short = b.clone();
        short.maps.insert(BakedMap::Thickness, Arc::new(vec![1.0; 100]));
        assert!(uniform(&short, (16, 16), &[BakedMap::Thickness]).levels.is_empty());
    }

    #[test]
    fn averaged_directions_stay_unit() {
        // Two normals tilted ±37° apart average to 0.8 long: the level holds them at unit length.
        let pack = |v: [f32; 3]| v.map(|c| c * 0.5 + 0.5);
        let data = [pack([0.6, 0.0, 0.8]), pack([-0.6, 0.0, 0.8])].concat();
        for m in [BakedMap::Normal, BakedMap::WorldNormal, BakedMap::BentNormal] {
            let b = BakedMaps { resolution: 2, maps: [(m, Arc::new(data.clone()))].into() };
            let v = uniform(&b, (1, 1), &[m]);
            let n = v.sample(m, 0, 0).unwrap();
            assert!(n.iter().zip([0.5, 0.5, 1.0]).all(|(a, b)| (a - b).abs() < 1e-6), "{m:?}: {n:?}");
        }
        // Any spread of directions: every averaged texel decodes to unit length.
        let wavy = |x: u32, y: u32| {
            let (a, b) = (x as f32 * 0.7, y as f32 * 1.3);
            let v = [a.sin() * b.cos(), a.cos() * b.cos(), b.sin().abs()];
            pack(v)
        };
        let data: Vec<f32> = (0..64 * 64).flat_map(|i| wavy(i % 64, i / 64)).collect();
        let b = BakedMaps { resolution: 64, maps: [(BakedMap::Normal, Arc::new(data))].into() };
        let v = uniform(&b, (7, 5), &[BakedMap::Normal]);
        for (x, y) in (0..5).flat_map(|y| (0..7).map(move |x| (x, y))) {
            let n = v.sample(BakedMap::Normal, x, y).unwrap().map(|c| c * 2.0 - 1.0);
            assert!((n.iter().map(|c| c * c).sum::<f32>().sqrt() - 1.0).abs() < 1e-5, "({x}, {y}): {n:?}");
        }
        // Scalar maps are averaged as they are.
        let ao = BakedMaps { resolution: 2, maps: [(BakedMap::AmbientOcclusion, Arc::new(vec![0.2, 0.6]))].into() };
        assert_eq!(uniform(&ao, (1, 1), &[BakedMap::AmbientOcclusion]).scalar(BakedMap::AmbientOcclusion, 0, 0), Some(0.4));
    }

    #[test]
    fn the_level_cache_keeps_the_most_recently_used_within_its_byte_budget() {
        let sources: Vec<Arc<Vec<f32>>> = (0..4).map(|_| Arc::new(vec![0.0; 64])).collect();
        let shape = Shape { from: (8, 8), comps: 1, to: (4, 4), directions: false };
        let level = Arc::new(vec![0.0f32; 16]);
        // Room for three 64-byte levels.
        let mut cache = LevelCache::default();
        for s in &sources[..3] {
            cache.insert(s, shape, &level, 3 * 64);
        }
        // Using the oldest makes the second the least recently used: a fourth level evicts it.
        assert!(cache.find(&sources[0], shape).is_some());
        cache.insert(&sources[3], shape, &level, 3 * 64);
        let held: Vec<bool> = sources.iter().map(|s| cache.find(s, shape).is_some()).collect();
        assert_eq!(held, [true, false, true, true]);
        // Another shape of the same data is another level.
        assert!(cache.find(&sources[0], Shape { to: (2, 2), ..shape }).is_none());
        // A level larger than the budget is still kept, alone.
        cache.insert(&sources[1], shape, &Arc::new(vec![0.0f32; 1024]), 3 * 64);
        assert_eq!(cache.entries.len(), 1);
        // Levels of dropped bakes go.
        let gone = Arc::new(vec![0.0f32; 64]);
        cache.insert(&gone, shape, &level, usize::MAX);
        drop(gone);
        assert!(cache.find(&sources[1], shape).is_some());
        assert_eq!(cache.entries.len(), 1);
    }

    /// Two UV tiles side by side: 1001 at 64² and 1002 at 16² (a quarter of its 64² block).
    fn two_tiles() -> TextureSet {
        let mut set = TextureSet::new("Tiles", 0, 64);
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 16);
        set
    }

    #[test]
    fn each_tile_block_is_averaged_to_its_own_resolution() {
        // A 256×128 bake (128² per tile) with a 1-texel line every 8 rows in both tiles.
        let set = two_tiles();
        let (size, scales) = (set.size(), set.block_scales());
        assert_eq!(size, (128, 64));
        let b = bake(BakedMap::AmbientOcclusion, (256, 128), lines);
        let v = b.view(size, size, &scales, &[BakedMap::AmbientOcclusion]);
        // 1001 reads 2× (each texel covers 2 rows of 8: a quarter of the time on the line, but
        // averaged rather than missing it); 1002 reads 8× (one line row in every texel).
        let at = |x, y| v.scalar(BakedMap::AmbientOcclusion, x, y);
        assert_eq!((at(0, 1), at(0, 0)), (Some(0.5), Some(0.0)), "1001: rows 2 and 3, rows 0 and 1");
        for (x, y) in [(64, 0), (79, 15), (70, 9)] {
            assert_eq!(at(x, y), Some(0.125), "1002 at ({x}, {y})");
        }
        // Nearest reads of 1002 never land on the line.
        assert_eq!(b.sample_working(BakedMap::AmbientOcclusion, 64, 0, size, &scales), Some([0.0; 3]));
        // The unused part of 1002's block reads nothing, as before.
        assert_eq!(at(100, 40), None);
        assert_eq!(b.sample_working(BakedMap::AmbientOcclusion, 100, 40, size, &scales), None);
    }

    #[test]
    fn tile_blocks_at_or_above_the_bake_read_nearest() {
        // A 128×64 bake (64² per tile): 1001 reads it texel for texel, 1002 at a quarter.
        let set = two_tiles();
        let (size, scales) = (set.size(), set.block_scales());
        let b = bake(BakedMap::Curvature, (128, 64), |x, y| (y * 128 + x) as f32);
        let v = b.view(size, size, &scales, &[BakedMap::Curvature]);
        for (x, y) in (0..64).flat_map(|y| (0..64).map(move |x| (x, y))) {
            assert_eq!(v.sample(BakedMap::Curvature, x, y), b.sample_working(BakedMap::Curvature, x, y, size, &scales), "({x}, {y})");
        }
        // 1002's texel (0, 0) averages its 4×4 baked texels from (64, 0).
        let mean = (0..4).flat_map(|y| (64..68).map(move |x| (y * 128 + x) as f32)).sum::<f32>() / 16.0;
        assert_eq!(v.scalar(BakedMap::Curvature, 64, 0), Some(mean));
    }
}
