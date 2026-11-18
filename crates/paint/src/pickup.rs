//! Clone and smudge: strokes that carry channel values picked up from the texture set instead of
//! painting the tool's material. The carried values are kept straight (not premultiplied) per
//! channel next to the stroke alpha, so the live preview and the commit read them directly.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use texturecraft_compose::eval::FlatTile;
use texturecraft_compose::{Composite, MASK_CHANNEL, PaintTarget};
use texturecraft_core::Channel;
use texturecraft_geom::{NO_TRI, UvGBuffer};
use texturecraft_texel::{TILE, TILE_AREA, TileImage, from_u16, to_u16};

use crate::StampSource;
use crate::commit::{dec, enc};

mod lane;
mod pickup_snapshot;

pub use pickup_snapshot::PickupSnapshot;

use lane::Lane;

/// How a pickup stroke finds the values it deposits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickupMode {
    /// Copy the values `offset` texels away (source = destination + offset).
    Clone { offset: [i64; 2] },
    /// Drag values along the stroke: each stamp deposits what lay under the previous stamp,
    /// blended by `strength` (0..1).
    Smudge { strength: f32 },
}

/// The values a clone or smudge stroke carries.
#[derive(Clone, Debug)]
pub struct Pickup {
    pub mode: PickupMode,
    /// The composite when the stroke began (what the stroke reads).
    source: Composite,
    /// How much of `source` there is to read per texel (a single layer's coverage); None reads
    /// everywhere.
    coverage: Option<TileImage>,
    mask: bool,
    snapshot: Option<Arc<PickupSnapshot>>,
    loaded: BTreeSet<(u32, u32)>,
    /// Texels outside the texture set's UV islands are never read.
    gbuf: Arc<UvGBuffer>,
    /// Straight values per painted channel; the stroke alpha lives in the stroke canvas.
    pub values: BTreeMap<Channel, TileImage>,
    /// Previous smudge stamp center (texels).
    prev: Option<[f32; 2]>,
    /// Smudge with symmetry: each copy drags its own trail (see [`Pickup::isolate_copies`]).
    isolated: bool,
    lanes: Vec<Lane>,
    /// Per-stamp scratch: texels that receive values, and the values read for them.
    reads: Vec<(u32, u32, f32)>,
    read_values: Vec<[f32; 3]>,
}

fn get(img: &TileImage, c: Channel, x: u32, y: u32) -> [f32; 3] {
    let raw = img.get(x, y).map(|v| dec(c.kind(), v));
    if c.components() == 3 { [raw[0], raw[1], raw[2]] } else { [raw[0]; 3] }
}

fn put(img: &mut TileImage, c: Channel, x: u32, y: u32, v: [f32; 3]) {
    img.set(x, y, &v.map(|x| enc(c.kind(), x))[..c.components().min(3)]);
}

impl Pickup {
    /// A pickup reading `source` (whose resolution is the stroke's) for `channels`.
    pub fn new(mode: PickupMode, source: Composite, gbuf: Arc<UvGBuffer>, channels: impl IntoIterator<Item = Channel>) -> Pickup {
        let values = channels.into_iter().map(|c| (c, TileImage::new(source.resolution, source.height, c.components()))).collect();
        Pickup {
            mode,
            source,
            coverage: None,
            mask: false,
            snapshot: None,
            loaded: BTreeSet::new(),
            gbuf,
            values,
            prev: None,
            isolated: false,
            lanes: Vec::new(),
            reads: Vec::new(),
            read_values: Vec::new(),
        }
    }

    /// A pickup reading one layer alone, from its flattened tiles (premultiplied values plus
    /// coverage per channel, see `texturecraft_compose::flatten_layer`) at `size`: where the
    /// layer is empty the stroke deposits nothing.
    pub fn from_layer(mode: PickupMode, flat: &[FlatTile], size: (u32, u32), gbuf: Arc<UvGBuffer>, channels: impl IntoIterator<Item = Channel>) -> Pickup {
        let channels: Vec<Channel> = channels.into_iter().collect();
        let source = Composite::with_size(&channels, size.0, size.1);
        let mut pickup = Self::new(mode, source, gbuf, channels);
        pickup.coverage = Some(TileImage::new(size.0, size.1, 1));
        pickup.add_layer_tiles(flat);
        pickup
    }

    fn add_layer_tiles(&mut self, flat: &[FlatTile]) {
        let Some(coverage) = self.coverage.as_mut() else { return };

        for (c, (tx, ty), data) in flat {
            let Some(img) = self.source.channels.get_mut(c) else { continue };
            let (n, kind) = (c.components(), c.kind());
            let mut straight = vec![0u16; TILE_AREA * n];
            let Some(cov) = coverage.tile_mut(*tx, *ty, &[0]) else { continue };
            for (o, px) in data.chunks_exact(n + 1).enumerate() {
                let a = px.get(n).copied().unwrap_or(0);
                if a == 0 {
                    continue;
                }
                let inv = 1.0 / from_u16(a);
                for (k, v) in px.iter().take(n).enumerate() {
                    if let Some(slot) = straight.get_mut(o * n + k) {
                        *slot = enc(kind, dec(kind, *v) * inv);
                    }
                }
                if let Some(slot) = cov.get_mut(o) {
                    *slot = (*slot).max(a);
                }
            }
            img.set_tile(*tx, *ty, Some(Arc::new(straight)));
        }
    }

    /// A pickup reading mask `values` (one component per texel, at the stroke's size): the stroke
    /// carries mask values under [`MASK_CHANNEL`].
    pub fn from_mask(mode: PickupMode, values: TileImage, gbuf: Arc<UvGBuffer>) -> Pickup {
        let mut source = Composite::with_size(&[MASK_CHANNEL], values.width(), values.height());
        source.channels.insert(MASK_CHANNEL, values);
        let mut pickup = Pickup::new(mode, source, gbuf, [MASK_CHANNEL]);
        pickup.mask = true;
        pickup
    }

    /// Evaluate only the source tiles read by this stroke, sharing each tile across its stamps.
    pub fn from_snapshot(mode: PickupMode, snapshot: PickupSnapshot, gbuf: Arc<UvGBuffer>, channels: impl IntoIterator<Item = Channel>) -> Self {
        let mask = matches!(snapshot.target, PaintTarget::Mask(_));
        let channels = if mask { vec![MASK_CHANNEL] } else { channels.into_iter().collect() };
        let source = Composite::with_size(&channels, gbuf.width, gbuf.height);
        let coverage = (!mask).then(|| TileImage::new(gbuf.width, gbuf.height, 1));
        let mut pickup = Self::new(mode, source, gbuf, channels);
        pickup.coverage = coverage;
        pickup.mask = mask;
        pickup.snapshot = Some(Arc::new(snapshot));
        pickup
    }

    /// Only a mask pickup can supply coverage to a mask preview or commit.
    pub fn mask_values(&self) -> Option<&TileImage> {
        self.mask.then(|| self.values.get(&MASK_CHANNEL)).flatten()
    }

    fn load_tiles(&mut self, texels: &[(u32, u32, f32)], shift: [i64; 2]) {
        let Some(snapshot) = &self.snapshot else { return };
        let mut missing = Vec::new();
        let mut previous = None;
        for &(x, y, alpha) in texels {
            let Ok(x) = u32::try_from(i64::from(x).saturating_add(shift[0])) else { continue };
            let Ok(y) = u32::try_from(i64::from(y).saturating_add(shift[1])) else { continue };
            let tile = (x / TILE, y / TILE);
            if previous == Some(tile) || alpha <= 0.0 || !covered(&self.gbuf, x, y) {
                continue;
            }

            previous = Some(tile);
            if self.loaded.insert(tile) {
                missing.push(tile);
            }
        }
        if missing.is_empty() {
            return;
        }

        let Some(inputs) = snapshot.inputs(&self.gbuf) else { return };
        match snapshot.target {
            PaintTarget::Layer(id) => {
                if let Some(flat) = texturecraft_compose::flatten_layer(&inputs, id, &missing) {
                    self.add_layer_tiles(&flat);
                }
            }
            PaintTarget::Mask(owner) => {
                if let Some(output) = self.source.channels.get_mut(&MASK_CHANNEL) {
                    texturecraft_compose::eval::mask_values_into(&inputs, owner, &missing, output);
                }
            }
        }
    }

    /// Smudge with symmetry: every copy of the stamp (told apart by its [`StampSource`]) drags
    /// its own trail. A copy reads the texture as it was when the stroke began plus what it
    /// deposited itself, never what another copy deposited, so copies where they overlap (at the
    /// mirror plane) do not smear each other. Their deposits still blend into the stroke, later
    /// copies over earlier ones.
    pub fn isolate_copies(&mut self) {
        self.isolated = true;
    }

    /// Blend one stamp's texels `(x, y, alpha)` (stamp center `center` in texels, `source` the
    /// symmetry copy it belongs to, None for the stroke itself) into the stroke `alpha` and the
    /// carried values. False when nothing was deposited: the first smudge stamp only picks up.
    pub fn stamp(&mut self, alpha: &mut TileImage, texels: &[(u32, u32, f32)], center: [f32; 2], source: Option<StampSource>) -> bool {
        if texels.is_empty() {
            return false;
        }

        let mut lane = if self.isolated && matches!(self.mode, PickupMode::Smudge { .. }) { Some(self.take_lane(source, alpha)) } else { None };
        let (shift, strength, smudge) = match self.mode {
            PickupMode::Clone { offset } => (offset, 1.0, false),
            PickupMode::Smudge { strength } => {
                let prev = match &mut lane {
                    Some(l) => l.prev.replace(center),
                    None => self.prev.replace(center),
                };
                let Some(prev) = prev else { return self.put_back(lane, false) };
                let d = [center[0] - prev[0], center[1] - prev[1]];

                // A jump across a UV seam or lifted cursor has nothing to drag along
                if d[0].hypot(d[1]) > extent(texels) {
                    return self.put_back(lane, false);
                }

                // Nearest-texel pickup avoids smoothing sub-texel stamp spacing
                ([-(d[0].round() as i64), -(d[1].round() as i64)], strength.clamp(0.0, 1.0), true)
            }
        };
        self.load_tiles(texels, shift);
        let (w, h) = (i64::from(self.source.resolution), i64::from(self.source.height));
        self.reads.clear();
        self.read_values.clear();
        for &(x, y, a) in texels {
            // Saturate hostile offsets so the bounds check rejects them without overflow
            let (sx, sy) = (i64::from(x).saturating_add(shift[0]), i64::from(y).saturating_add(shift[1]));
            if !(a > 0.0 && (0..w).contains(&sx) && (0..h).contains(&sy) && covered(&self.gbuf, sx as u32, sy as u32)) {
                continue;
            }

            let (sx, sy) = (sx as u32, sy as u32);
            let a = a * strength * self.coverage.as_ref().map_or(1.0, |c| from_u16(c.get(sx, sy)[0]));
            if a <= 0.0 {
                continue;
            }

            // Each smudge copy reads its own deposits over the original source
            let (seen_alpha, seen_values) = match &lane {
                Some(l) => (&l.alpha, &l.values),
                None => (&*alpha, &self.values),
            };
            let m = if smudge { from_u16(seen_alpha.get(sx, sy)[0]) } else { 0.0 };
            for c in self.values.keys() {
                let base = self.source.value(*c, sx, sy);
                let v = match seen_values.get(c) {
                    Some(img) if m > 0.0 => {
                        let carried = get(img, *c, sx, sy);
                        std::array::from_fn(|k| carried[k] * m + base[k] * (1.0 - m))
                    }
                    _ => base,
                };
                self.read_values.push(v);
            }
            self.reads.push((x, y, a));
        }
        if let Some(l) = &mut lane {
            deposit(&mut l.alpha, &mut l.values, &self.reads, &self.read_values);
        }
        deposit(alpha, &mut self.values, &self.reads, &self.read_values);
        let painted = !self.reads.is_empty();
        self.put_back(lane, painted)
    }

    /// The lane of the copy `source`, started when the copy first stamps.
    fn take_lane(&mut self, source: Option<StampSource>, alpha: &TileImage) -> Lane {
        match self.lanes.iter().position(|l| l.source == source) {
            Some(i) => self.lanes.swap_remove(i),
            None => {
                let values = self.values.iter().map(|(c, img)| (*c, TileImage::new(img.width(), img.height(), c.components()))).collect();
                Lane { source, prev: None, alpha: TileImage::new(alpha.width(), alpha.height(), 1), values }
            }
        }
    }

    fn put_back(&mut self, lane: Option<Lane>, painted: bool) -> bool {
        self.lanes.extend(lane);
        painted
    }
}

fn covered(gbuf: &UvGBuffer, x: u32, y: u32) -> bool {
    if x >= gbuf.width || y >= gbuf.height {
        return false;
    }

    let i = u64::from(y) * u64::from(gbuf.width) + u64::from(x);
    usize::try_from(i).ok().and_then(|i| gbuf.tri.get(i)).is_some_and(|t| *t != NO_TRI)
}

/// Blend the texels `reads` `(x, y, alpha)` with their `read_values` (one entry per channel of
/// `values`, in order) into `alpha` and `values` ("over").
fn deposit(alpha: &mut TileImage, values: &mut BTreeMap<Channel, TileImage>, reads: &[(u32, u32, f32)], read_values: &[[f32; 3]]) {
    let n = values.len();
    for (i, &(x, y, a)) in reads.iter().enumerate() {
        let m = from_u16(alpha.get(x, y)[0]);
        let m2 = a + m * (1.0 - a);
        for (k, (c, img)) in values.iter_mut().enumerate() {
            let Some(src) = read_values.get(i * n + k) else { continue };
            let old = get(img, *c, x, y);
            put(img, *c, x, y, std::array::from_fn(|j| (src[j] * a + old[j] * m * (1.0 - a)) / m2.max(1e-6)));
        }
        alpha.set(x, y, &[to_u16(m2)]);
    }
}

/// Largest side of a stamp's texel bounding box.
fn extent(texels: &[(u32, u32, f32)]) -> f32 {
    let (mut lo, mut hi) = ([u32::MAX; 2], [0u32; 2]);
    for &(x, y, _) in texels {
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y)];
    }
    hi[0].saturating_sub(lo[0]).max(hi[1].saturating_sub(lo[1])) as f32 + 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::primitives;
    use texturecraft_texel::{TILE, to_u16};

    /// A 64² plane (fully covered) whose roughness is `f(x)`.
    fn setup(f: impl Fn(u32) -> f32) -> (Composite, Arc<UvGBuffer>) {
        let m = primitives::plane(2.0, 4);
        let g = Arc::new(UvGBuffer::new(&m, 0, TILE, TILE));
        let mut c = Composite::new(&[Channel::Roughness], TILE);
        if let Some(img) = c.channels.get_mut(&Channel::Roughness) {
            for y in 0..TILE {
                for x in 0..TILE {
                    img.set(x, y, &[to_u16(f(x))]);
                }
            }
        }
        (c, g)
    }

    fn disc(cx: u32, cy: u32, r: u32) -> Vec<(u32, u32, f32)> {
        let mut v = Vec::new();
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if (x.abs_diff(cx).pow(2) + y.abs_diff(cy).pow(2)) <= r * r {
                    v.push((x, y, 1.0));
                }
            }
        }
        v
    }

    #[test]
    fn layer_and_mask_pickups_only_evaluate_the_shifted_tiles_the_stroke_reads() {
        use texturecraft_core::{Fill, Layer, LayerKind, Mask, MaskBackground, Project, Source, TextureSet};

        let size = 4 * TILE;
        let mesh = Arc::new(primitives::plane(2.0, 4));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, size, size));
        let mut project = Project::new("test", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Set", 0, size));
        let id = project.alloc_id();
        let fill = Fill { sources: [(Channel::Roughness, Source::Uniform([0.25; 3]))].into(), projection: Default::default(), link: None };
        let mut layer = Layer::new(id, "fill", LayerKind::Fill(fill), &[Channel::Roughness]);
        layer.mask = Some(Mask::new(MaskBackground::White));
        project.insert_layer(0, layer, None, false).unwrap();

        for (target, expected) in [(PaintTarget::Layer(id), 0.25), (PaintTarget::Mask(id), 1.0)] {
            let snapshot = PickupSnapshot {
                project: project.clone(),
                ts: 0,
                target,
                mesh: mesh.clone(),
                bounds: mesh.bounds(),
                pixels: Default::default(),
                baked: Default::default(),
                images: Default::default(),
            };
            let mut pickup = Pickup::from_snapshot(PickupMode::Clone { offset: [i64::from(TILE), 0] }, snapshot, gbuf.clone(), [Channel::Roughness]);
            let mut alpha = TileImage::new(size, size, 1);
            assert!(pickup.loaded.is_empty());

            for x in [8, 12, TILE + 8] {
                assert!(pickup.stamp(&mut alpha, &[(x, 8, 1.0)], [x as f32, 8.0], None));
                let image = &pickup.values[&Channel::Roughness];
                assert!((from_u16(image.get(x, 8)[0]) - expected).abs() < 1e-4);
                assert_eq!(pickup.mask_values().is_some(), matches!(target, PaintTarget::Mask(_)));
            }

            assert_eq!(pickup.loaded, [(1, 0), (2, 0)].into());
            assert_eq!(pickup.source.channels[&Channel::Roughness].allocated_count(), 2);
        }
    }

    #[test]
    fn clone_copies_values_from_the_offset() {
        let (c, g) = setup(|x| if x < 32 { 0.9 } else { 0.1 });
        let mut p = Pickup::new(PickupMode::Clone { offset: [-30, 0] }, c, g, [Channel::Roughness]);
        let mut alpha = TileImage::new(TILE, TILE, 1);
        assert!(p.stamp(&mut alpha, &disc(45, 32, 4), [45.0, 32.0], None));
        let v = get(&p.values[&Channel::Roughness], Channel::Roughness, 45, 32);
        assert!((v[0] - 0.9).abs() < 1e-3, "{v:?}");
        assert!(from_u16(alpha.get(45, 32)[0]) > 0.99);

        // Sources outside the image are skipped
        assert!(!p.stamp(&mut alpha, &disc(5, 5, 2), [5.0, 5.0], None));
    }

    #[test]
    fn clone_skips_offsets_outside_the_coordinate_range() {
        for offset in [[i64::MAX, 0], [0, i64::MAX], [i64::MIN, 0], [0, i64::MIN]] {
            let (composite, gbuf) = setup(|_| 0.5);
            let mut pickup = Pickup::new(PickupMode::Clone { offset }, composite, gbuf, [Channel::Roughness]);
            let mut alpha = TileImage::new(TILE, TILE, 1);

            let painted = pickup.stamp(&mut alpha, &[(1, 1, 1.0)], [1.0, 1.0], None);

            assert!(!painted);
            assert_eq!(alpha.allocated_count(), 0);
            assert_eq!(pickup.values[&Channel::Roughness].allocated_count(), 0);
        }
    }

    #[test]
    fn layer_clones_copy_only_where_the_layer_has_paint() {
        let (_, g) = setup(|_| 0.0);

        // The layer has roughness 0.8 at half coverage on its left half
        let mut data = vec![0u16; TILE_AREA * 2];
        for y in 0..TILE {
            for x in 0..TILE / 2 {
                let o = (y * TILE + x) as usize * 2;
                data[o] = to_u16(0.4);
                data[o + 1] = to_u16(0.5);
            }
        }
        let flat = vec![(Channel::Roughness, (0, 0), data)];
        let mut p = Pickup::from_layer(PickupMode::Clone { offset: [-30, 0] }, &flat, (TILE, TILE), g.clone(), [Channel::Roughness]);
        let mut alpha = TileImage::new(TILE, TILE, 1);
        assert!(p.stamp(&mut alpha, &disc(45, 32, 3), [45.0, 32.0], None));
        let v = get(&p.values[&Channel::Roughness], Channel::Roughness, 45, 32)[0];
        assert!((v - 0.8).abs() < 1e-3, "straight value: {v}");
        assert!((from_u16(alpha.get(45, 32)[0]) - 0.5).abs() < 1e-3, "the layer's coverage scales the stroke");

        // Reading the layer's empty right half deposits nothing
        let mut q = Pickup::from_layer(PickupMode::Clone { offset: [30, 0] }, &flat, (TILE, TILE), g, [Channel::Roughness]);
        assert!(!q.stamp(&mut TileImage::new(TILE, TILE, 1), &disc(20, 32, 3), [20.0, 32.0], None), "the source is the empty half");
    }

    #[test]
    fn smudge_drags_values_along_the_stroke() {
        let (c, g) = setup(|x| if x < 20 { 1.0 } else { 0.0 });
        let mut p = Pickup::new(PickupMode::Smudge { strength: 1.0 }, c, g, [Channel::Roughness]);
        let mut alpha = TileImage::new(TILE, TILE, 1);
        assert!(!p.stamp(&mut alpha, &disc(18, 32, 5), [18.0, 32.0], None), "the first stamp only picks up");
        for cx in 20..30 {
            p.stamp(&mut alpha, &disc(cx, 32, 5), [cx as f32, 32.0], None);
        }
        let v = get(&p.values[&Channel::Roughness], Channel::Roughness, 30, 32)[0];
        assert!(v > 0.5 && from_u16(alpha.get(30, 32)[0]) > 0.99, "white dragged to the right: {v}");

        // A jump larger than the stamp deposits nothing
        assert!(!p.stamp(&mut alpha, &disc(50, 10, 3), [50.0, 10.0], None));
    }

    /// Two copies smudging through the same texels: copy B drags exactly what it would alone.
    #[test]
    fn mirrored_smudge_copies_do_not_feed_each_other() {
        use texturecraft_geom::glam::{Mat3, Vec3};
        let source = |x: f32| Some(StampSource { origin: Vec3::ZERO, to_source: Mat3::from_diagonal(Vec3::new(x, 1.0, 1.0)) });
        let (a, b) = (source(1.0), source(-1.0));
        let smudge = || {
            let (c, g) = setup(|x| if x < 20 { 1.0 } else { 0.0 });
            Pickup::new(PickupMode::Smudge { strength: 1.0 }, c, g, [Channel::Roughness])
        };

        // A drags white right along row 32 while B drags left through the same texels
        let (mut alone, mut isolated) = (smudge(), smudge());
        isolated.isolate_copies();
        let (mut alpha_alone, mut alpha_isolated) = (TileImage::new(TILE, TILE, 1), TileImage::new(TILE, TILE, 1));
        for i in 0..24 {
            let (xa, xb) = (18.0 + i as f32, 46.0 - i as f32);
            for (pickup, alpha, copies) in [(&mut isolated, &mut alpha_isolated, true), (&mut alone, &mut alpha_alone, false)] {
                if copies {
                    pickup.stamp(alpha, &disc(xa as u32, 32, 5), [xa, 32.0], a);
                }
                pickup.stamp(alpha, &disc(xb as u32, 32, 5), [xb, 32.0], b);
            }
        }
        let lane_b = isolated.lanes.iter().find(|l| l.source == b).expect("copy B has its lane");
        let (mut stroke_differs, mut lane_differs) = (false, false);
        for x in 14..52 {
            let alone_value = get(&alone.values[&Channel::Roughness], Channel::Roughness, x, 32)[0];
            lane_differs |= (get(&lane_b.values[&Channel::Roughness], Channel::Roughness, x, 32)[0] - alone_value).abs() > 1e-4;
            stroke_differs |= (get(&isolated.values[&Channel::Roughness], Channel::Roughness, x, 32)[0] - alone_value).abs() > 1e-4;
        }
        assert!(!lane_differs, "copy B drags what it would drag alone");
        assert!(stroke_differs, "the stroke shows both copies where they overlap");
    }
}
