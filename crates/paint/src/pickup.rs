//! Clone and smudge: strokes that carry channel values picked up from the texture set instead of
//! painting the tool's material. The carried values are kept straight (not premultiplied) per
//! channel next to the stroke alpha, so the live preview and the commit read them directly.

use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_compose::eval::FlatTile;
use texturecraft_compose::{Composite, MASK_CHANNEL};
use texturecraft_core::Channel;
use texturecraft_geom::{NO_TRI, UvGBuffer};
use texturecraft_texel::{TILE_AREA, TileImage, from_u16, to_u16};

use crate::commit::{dec, enc};

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
    /// Texels outside the texture set's UV islands are never read.
    gbuf: Arc<UvGBuffer>,
    /// Straight values per painted channel; the stroke alpha lives in the stroke canvas.
    pub values: BTreeMap<Channel, TileImage>,
    /// Previous smudge stamp center (texels).
    prev: Option<[f32; 2]>,
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
        Pickup { mode, source, coverage: None, gbuf, values, prev: None, reads: Vec::new(), read_values: Vec::new() }
    }

    /// A pickup reading one layer alone, from its flattened tiles (premultiplied values plus
    /// coverage per channel, see `texturecraft_compose::flatten_layer`) at `size`: where the
    /// layer is empty the stroke deposits nothing.
    pub fn from_layer(mode: PickupMode, flat: &[FlatTile], size: (u32, u32), gbuf: Arc<UvGBuffer>, channels: impl IntoIterator<Item = Channel>) -> Pickup {
        let channels: Vec<Channel> = channels.into_iter().collect();
        let mut source = Composite::with_size(&channels, size.0, size.1);
        let mut coverage = TileImage::new(size.0, size.1, 1);
        for (c, (tx, ty), data) in flat {
            let Some(img) = source.channels.get_mut(c) else { continue };
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
        let mut p = Pickup::new(mode, source, gbuf, channels);
        p.coverage = Some(coverage);
        p
    }

    /// A pickup reading mask `values` (one component per texel, at the stroke's size): the stroke
    /// carries mask values under [`MASK_CHANNEL`].
    pub fn from_mask(mode: PickupMode, values: TileImage, gbuf: Arc<UvGBuffer>) -> Pickup {
        let mut source = Composite::with_size(&[MASK_CHANNEL], values.width(), values.height());
        source.channels.insert(MASK_CHANNEL, values);
        Pickup::new(mode, source, gbuf, [MASK_CHANNEL])
    }

    fn covered(&self, x: u32, y: u32) -> bool {
        let i = y as usize * self.gbuf.width as usize + x as usize;
        x < self.gbuf.width && self.gbuf.tri.get(i).is_some_and(|t| *t != NO_TRI)
    }

    /// Blend one stamp's texels `(x, y, alpha)` (stamp center `center` in texels) into the
    /// stroke `alpha` and the carried values. False when nothing was deposited: the first smudge
    /// stamp only picks up.
    pub fn stamp(&mut self, alpha: &mut TileImage, texels: &[(u32, u32, f32)], center: [f32; 2]) -> bool {
        if texels.is_empty() {
            return false;
        }
        let (shift, strength, smudge) = match self.mode {
            PickupMode::Clone { offset } => (offset, 1.0, false),
            PickupMode::Smudge { strength } => {
                let Some(prev) = self.prev.replace(center) else { return false };
                let d = [center[0] - prev[0], center[1] - prev[1]];
                // A jump (UV seam, lifted cursor) has nothing to drag along.
                if d[0].hypot(d[1]) > extent(texels) {
                    return false;
                }
                // shortcut: nearest-texel pickup; bilinear would smooth sub-texel stamp spacing.
                ([-(d[0].round() as i64), -(d[1].round() as i64)], strength.clamp(0.0, 1.0), true)
            }
        };
        let (w, h) = (i64::from(self.source.resolution), i64::from(self.source.height));
        self.reads.clear();
        self.read_values.clear();
        for &(x, y, a) in texels {
            let (sx, sy) = (i64::from(x) + shift[0], i64::from(y) + shift[1]);
            if !(a > 0.0 && (0..w).contains(&sx) && (0..h).contains(&sy) && self.covered(sx as u32, sy as u32)) {
                continue;
            }
            let (sx, sy) = (sx as u32, sy as u32);
            let a = a * strength * self.coverage.as_ref().map_or(1.0, |c| from_u16(c.get(sx, sy)[0]));
            if a <= 0.0 {
                continue;
            }
            // Smudge reads what the stroke shows so far: carried values over the source.
            let m = if smudge { from_u16(alpha.get(sx, sy)[0]) } else { 0.0 };
            for (c, img) in &self.values {
                let base = self.source.value(*c, sx, sy);
                let v = if m > 0.0 {
                    let carried = get(img, *c, sx, sy);
                    std::array::from_fn(|k| carried[k] * m + base[k] * (1.0 - m))
                } else {
                    base
                };
                self.read_values.push(v);
            }
            self.reads.push((x, y, a));
        }
        let n = self.values.len();
        for (i, &(x, y, a)) in self.reads.iter().enumerate() {
            let m = from_u16(alpha.get(x, y)[0]);
            let m2 = a + m * (1.0 - a);
            for (k, (c, img)) in self.values.iter_mut().enumerate() {
                let Some(src) = self.read_values.get(i * n + k) else { continue };
                let old = get(img, *c, x, y);
                put(img, *c, x, y, std::array::from_fn(|j| (src[j] * a + old[j] * m * (1.0 - a)) / m2.max(1e-6)));
            }
            alpha.set(x, y, &[to_u16(m2)]);
        }
        !self.reads.is_empty()
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
    fn clone_copies_values_from_the_offset() {
        let (c, g) = setup(|x| if x < 32 { 0.9 } else { 0.1 });
        let mut p = Pickup::new(PickupMode::Clone { offset: [-30, 0] }, c, g, [Channel::Roughness]);
        let mut alpha = TileImage::new(TILE, TILE, 1);
        assert!(p.stamp(&mut alpha, &disc(45, 32, 4), [45.0, 32.0]));
        let v = get(&p.values[&Channel::Roughness], Channel::Roughness, 45, 32);
        assert!((v[0] - 0.9).abs() < 1e-3, "{v:?}");
        assert!(from_u16(alpha.get(45, 32)[0]) > 0.99);
        // Sources outside the image are skipped.
        assert!(!p.stamp(&mut alpha, &disc(5, 5, 2), [5.0, 5.0]));
    }

    #[test]
    fn layer_clones_copy_only_where_the_layer_has_paint() {
        let (_, g) = setup(|_| 0.0);
        // The layer: roughness 0.8 at half coverage on the left half (premultiplied).
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
        assert!(p.stamp(&mut alpha, &disc(45, 32, 3), [45.0, 32.0]));
        let v = get(&p.values[&Channel::Roughness], Channel::Roughness, 45, 32)[0];
        assert!((v - 0.8).abs() < 1e-3, "straight value: {v}");
        assert!((from_u16(alpha.get(45, 32)[0]) - 0.5).abs() < 1e-3, "the layer's coverage scales the stroke");
        // Reading the layer's empty right half deposits nothing.
        let mut q = Pickup::from_layer(PickupMode::Clone { offset: [30, 0] }, &flat, (TILE, TILE), g, [Channel::Roughness]);
        assert!(!q.stamp(&mut TileImage::new(TILE, TILE, 1), &disc(20, 32, 3), [20.0, 32.0]), "the source is the empty half");
    }

    #[test]
    fn smudge_drags_values_along_the_stroke() {
        let (c, g) = setup(|x| if x < 20 { 1.0 } else { 0.0 });
        let mut p = Pickup::new(PickupMode::Smudge { strength: 1.0 }, c, g, [Channel::Roughness]);
        let mut alpha = TileImage::new(TILE, TILE, 1);
        assert!(!p.stamp(&mut alpha, &disc(18, 32, 5), [18.0, 32.0]), "the first stamp only picks up");
        for cx in 20..30 {
            p.stamp(&mut alpha, &disc(cx, 32, 5), [cx as f32, 32.0]);
        }
        let v = get(&p.values[&Channel::Roughness], Channel::Roughness, 30, 32)[0];
        assert!(v > 0.5 && from_u16(alpha.get(30, 32)[0]) > 0.99, "white dragged to the right: {v}");
        // A jump larger than the stamp deposits nothing.
        assert!(!p.stamp(&mut alpha, &disc(50, 10, 3), [50.0, 10.0]));
    }
}
