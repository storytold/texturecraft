//! Sparse, copy-on-write tiled images of `u16` components.

use std::sync::Arc;

/// Tile edge in texels.
pub const TILE: u32 = 64;
/// Texels per tile.
pub const TILE_AREA: usize = (TILE * TILE) as usize;

/// A tile's texel data: `TILE_AREA × comps` interleaved components.
pub type TileData = Arc<Vec<u16>>;

/// An image split into 64×64 tiles; unallocated tiles read as zero. Cloning shares the tile table
/// and every tile, so a snapshot costs one reference-count bump; the first write after a clone
/// copies the table (one pointer per tile slot) once.
#[derive(Clone, Debug, PartialEq)]
pub struct TileImage {
    width: u32,
    height: u32,
    comps: usize,
    tiles_x: u32,
    tiles_y: u32,
    tiles: Arc<Vec<Option<TileData>>>,
}

impl TileImage {
    /// `comps` is clamped to 1..=4; sizes to at least 1.
    pub fn new(width: u32, height: u32, comps: usize) -> TileImage {
        let (width, height) = (width.max(1), height.max(1));
        let tiles_x = width.div_ceil(TILE);
        let tiles_y = height.div_ceil(TILE);
        TileImage { width, height, comps: comps.clamp(1, 4), tiles_x, tiles_y, tiles: Arc::new(vec![None; (tiles_x * tiles_y) as usize]) }
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn comps(&self) -> usize {
        self.comps
    }
    pub fn tiles_x(&self) -> u32 {
        self.tiles_x
    }
    pub fn tiles_y(&self) -> u32 {
        self.tiles_y
    }

    fn tile_index(&self, tx: u32, ty: u32) -> Option<usize> {
        (tx < self.tiles_x && ty < self.tiles_y).then(|| (ty * self.tiles_x + tx) as usize)
    }

    pub fn tile(&self, tx: u32, ty: u32) -> Option<&[u16]> {
        self.tile_index(tx, ty).and_then(|i| self.tiles.get(i)).and_then(|t| t.as_deref().map(Vec::as_slice))
    }

    /// The shared tile handle (for undo snapshots and change fingerprints).
    pub fn tile_handle(&self, tx: u32, ty: u32) -> Option<&TileData> {
        self.tile_index(tx, ty).and_then(|i| self.tiles.get(i)).and_then(Option::as_ref)
    }

    /// Replace a tile wholesale (None frees it).
    pub fn set_tile(&mut self, tx: u32, ty: u32, data: Option<TileData>) {
        let len = TILE_AREA * self.comps;
        let data = data.filter(|d| d.len() == len);
        let Some(i) = self.tile_index(tx, ty) else { return };
        // Freeing a free tile changes nothing: don't copy a table that undo snapshots share.
        if data.is_none() && self.tiles.get(i).is_some_and(Option::is_none) {
            return;
        }
        if let Some(slot) = Arc::make_mut(&mut self.tiles).get_mut(i) {
            *slot = data;
        }
    }

    /// Mutable texels of a tile, allocating it filled with `fill` (one value per component)
    /// when absent and un-sharing it when shared.
    pub fn tile_mut(&mut self, tx: u32, ty: u32, fill: &[u16]) -> Option<&mut [u16]> {
        let comps = self.comps;
        let i = self.tile_index(tx, ty)?;
        let slot = Arc::make_mut(&mut self.tiles).get_mut(i)?;
        let tile = slot.get_or_insert_with(|| {
            let mut v = vec![0u16; TILE_AREA * comps];
            if fill.iter().any(|f| *f != 0) {
                for px in v.chunks_exact_mut(comps) {
                    for (d, s) in px.iter_mut().zip(fill) {
                        *d = *s;
                    }
                }
            }
            Arc::new(v)
        });
        Some(Arc::make_mut(tile).as_mut_slice())
    }

    pub fn is_allocated(&self, tx: u32, ty: u32) -> bool {
        self.tile_handle(tx, ty).is_some()
    }

    /// Allocated tile coordinates.
    pub fn allocated(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.tiles.iter().enumerate().filter(|(_, t)| t.is_some()).map(|(i, _)| (i as u32 % self.tiles_x, i as u32 / self.tiles_x))
    }

    pub fn allocated_count(&self) -> usize {
        self.tiles.iter().filter(|t| t.is_some()).count()
    }

    /// Components of the texel at (x, y); zeros when out of range or unallocated.
    pub fn get(&self, x: u32, y: u32) -> [u16; 4] {
        let mut out = [0u16; 4];
        if x >= self.width || y >= self.height {
            return out;
        }
        if let Some(t) = self.tile(x / TILE, y / TILE) {
            let o = (((y % TILE) * TILE + x % TILE) as usize) * self.comps;
            for (c, v) in out.iter_mut().take(self.comps).enumerate() {
                *v = t.get(o + c).copied().unwrap_or(0);
            }
        }
        out
    }

    /// Set one texel (allocating its tile with zeros).
    pub fn set(&mut self, x: u32, y: u32, v: &[u16]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let comps = self.comps;
        if let Some(t) = self.tile_mut(x / TILE, y / TILE, &[]) {
            let o = (((y % TILE) * TILE + x % TILE) as usize) * comps;
            for (c, s) in v.iter().take(comps).enumerate() {
                if let Some(d) = t.get_mut(o + c) {
                    *d = *s;
                }
            }
        }
    }

    /// Bytes held by allocated tiles (memory estimate for the status bar).
    pub fn bytes(&self) -> usize {
        self.allocated_count() * TILE_AREA * self.comps * 2
    }

    /// Whether `self` and `other` share their tile table: no tile of either changed since one
    /// was cloned from the other.
    pub fn same_tiles(&self, other: &TileImage) -> bool {
        Arc::ptr_eq(&self.tiles, &other.tiles)
    }

    /// Drop tiles whose every texel is zero.
    pub fn prune_empty(&mut self) {
        let is_empty = |t: &Option<TileData>| t.as_ref().is_some_and(|d| d.iter().all(|v| *v == 0));
        if !self.tiles.iter().any(is_empty) {
            return;
        }
        for t in Arc::make_mut(&mut self.tiles) {
            if t.as_ref().is_some_and(|d| d.iter().all(|v| *v == 0)) {
                *t = None;
            }
        }
    }
}

/// 0..1 → u16.
#[inline]
pub fn to_u16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

/// u16 → 0..1.
#[inline]
pub fn from_u16(v: u16) -> f32 {
    v as f32 * (1.0 / 65535.0)
}

/// -1..1 → u16.
#[inline]
pub fn to_u16_signed(v: f32) -> u16 {
    to_u16(v * 0.5 + 0.5)
}

/// u16 → -1..1.
#[inline]
pub fn from_u16_signed(v: u16) -> f32 {
    from_u16(v) * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unallocated_tiles_read_zero_and_writes_allocate() {
        let mut img = TileImage::new(200, 130, 2);
        assert_eq!(img.tiles_x(), 4);
        assert_eq!(img.tiles_y(), 3);
        assert_eq!(img.get(10, 10), [0; 4]);
        img.set(199, 129, &[7, 9]);
        assert_eq!(img.get(199, 129), [7, 9, 0, 0]);
        assert_eq!(img.allocated_count(), 1);
        assert_eq!(img.allocated().collect::<Vec<_>>(), vec![(3, 2)]);
        // Out of range is ignored, not a panic.
        img.set(500, 500, &[1, 1]);
        assert_eq!(img.get(500, 500), [0; 4]);
    }

    #[test]
    fn clones_share_tiles_until_written() {
        let mut a = TileImage::new(64, 64, 1);
        a.set(0, 0, &[5]);
        let b = a.clone();
        assert!(Arc::ptr_eq(a.tile_handle(0, 0).unwrap(), b.tile_handle(0, 0).unwrap()));
        a.set(1, 0, &[6]);
        assert!(!Arc::ptr_eq(a.tile_handle(0, 0).unwrap(), b.tile_handle(0, 0).unwrap()));
        assert_eq!(b.get(1, 0)[0], 0);
        assert_eq!(a.get(1, 0)[0], 6);
    }

    #[test]
    fn clone_shares_the_table_until_written() {
        let mut a = TileImage::new(512, 512, 2);
        a.set(0, 0, &[1, 2]);
        let mut b = a.clone();
        assert!(a.same_tiles(&b));
        b.set(70, 0, &[3, 4]);
        assert!(!a.same_tiles(&b));
        assert_eq!(a.get(70, 0)[..2], [0, 0]);
        assert_eq!(b.get(70, 0)[..2], [3, 4]);
        assert_eq!(a.get(0, 0)[..2], [1, 2]);
        assert_eq!(b.get(0, 0)[..2], [1, 2]);
    }

    #[test]
    fn out_of_range_writes_do_not_unshare() {
        let mut a = TileImage::new(512, 512, 2);
        a.set(0, 0, &[1, 2]);
        let mut b = a.clone();
        b.set_tile(999, 999, None);
        assert!(b.tile_mut(999, 999, &[0, 0]).is_none());
        assert!(a.same_tiles(&b));
    }

    /// Undo restores free the tiles a stroke allocated, including ones that are already free:
    /// that must not copy the table every snapshot shares.
    #[test]
    fn freeing_a_free_tile_does_not_unshare() {
        let mut a = TileImage::new(512, 512, 2);
        a.set(0, 0, &[1, 2]);
        let mut b = a.clone();
        b.set_tile(1, 0, None);
        assert!(a.same_tiles(&b));
        b.set_tile(0, 0, None);
        assert!(!a.same_tiles(&b), "freeing a used tile still un-shares");
        assert_eq!(b.get(0, 0)[..2], [0, 0]);
    }

    #[test]
    fn noop_prune_does_not_unshare() {
        let mut a = TileImage::new(512, 512, 2);
        a.set(0, 0, &[1, 2]);
        let mut b = a.clone();
        b.prune_empty();
        assert!(a.same_tiles(&b));
    }

    #[test]
    fn tile_mut_fills_new_tiles() {
        let mut img = TileImage::new(64, 64, 2);
        let t = img.tile_mut(0, 0, &[100, 200]).unwrap();
        assert_eq!(&t[..4], &[100, 200, 100, 200]);
        img.prune_empty();
        assert_eq!(img.allocated_count(), 1);
        img.set_tile(0, 0, Some(Arc::new(vec![0; TILE_AREA * 2])));
        img.prune_empty();
        assert_eq!(img.allocated_count(), 0);
    }

    #[test]
    fn unorm_round_trips() {
        for v in [0.0, 0.25, 0.5, 1.0] {
            assert!((from_u16(to_u16(v)) - v).abs() < 1e-4);
            assert!((from_u16_signed(to_u16_signed(v * 2.0 - 1.0)) - (v * 2.0 - 1.0)).abs() < 1e-4);
        }
        assert_eq!(to_u16(f32::NAN), 0);
    }
}

impl TileImage {
    /// A bilinear resample to `width × height` (same component count). Unallocated source
    /// texels read as `empty` (one value per component); destination tiles whose source
    /// footprint is entirely unallocated stay unallocated.
    pub fn resampled(&self, width: u32, height: u32, empty: &[u16]) -> TileImage {
        let mut out = TileImage::new(width, height, self.comps);
        if self.allocated_count() == 0 {
            return out;
        }
        for ty in 0..out.tiles_y {
            for tx in 0..out.tiles_x {
                if let Some(data) = self.resampled_tile(width, height, tx, ty, empty) {
                    out.set_tile(tx, ty, Some(Arc::new(data)));
                }
            }
        }
        out
    }

    /// Tile (tx, ty) of [`TileImage::resampled`] alone (None when its source footprint is
    /// entirely unallocated).
    pub fn resampled_tile(&self, width: u32, height: u32, tx: u32, ty: u32, empty: &[u16]) -> Option<Vec<u16>> {
        let (sw, sh) = (self.width as f32, self.height as f32);
        let (dw, dh) = (width.max(1), height.max(1));
        let comps = self.comps;
        // Skip destination tiles whose source footprint has no allocated tile.
        let (x0, y0) = ((tx * TILE) as f32 * sw / dw as f32, (ty * TILE) as f32 * sh / dh as f32);
        let (x1, y1) = (((tx + 1) * TILE) as f32 * sw / dw as f32, ((ty + 1) * TILE) as f32 * sh / dh as f32);
        let any = ((y0 as u32).saturating_sub(1) / TILE..=((y1 as u32 + 1) / TILE).min(self.tiles_y.saturating_sub(1))).any(|sy| {
            ((x0 as u32).saturating_sub(1) / TILE..=((x1 as u32 + 1) / TILE).min(self.tiles_x.saturating_sub(1))).any(|sx| self.is_allocated(sx, sy))
        });
        if !any || self.width == 0 || self.height == 0 {
            return None;
        }
        let mut data = vec![0u16; TILE_AREA * comps];
        for ly in 0..TILE {
            let y = ty * TILE + ly;
            if y >= dh {
                break;
            }
            let fy = (y as f32 + 0.5) * sh / dh as f32;
            for lx in 0..TILE {
                let x = tx * TILE + lx;
                if x >= dw {
                    break;
                }
                let fx = (x as f32 + 0.5) * sw / dw as f32;
                let o = ((ly * TILE + lx) as usize) * comps;
                if let Some(out) = data.get_mut(o..o + comps) {
                    self.bilinear(fx, fy, empty, out);
                }
            }
        }
        Some(data)
    }

    /// Bilinear sample at `(fx, fy)` in texels (texel centers at +0.5, clamped to the image)
    /// into `out` (one value per component); unallocated texels read as `empty`. Returns
    /// whether any of the four texels is allocated.
    fn bilinear(&self, fx: f32, fy: f32, empty: &[u16], out: &mut [u16]) -> bool {
        let (fx, fy) = ((fx - 0.5).max(0.0), (fy - 0.5).max(0.0));
        let (ix, wx, iy, wy) = (fx.floor() as u32, fx.fract(), fy.floor() as u32, fy.fract());
        let mut any = false;
        let mut get = |x: u32, y: u32| {
            let (x, y) = (x.min(self.width.saturating_sub(1)), y.min(self.height.saturating_sub(1)));
            if self.is_allocated(x / TILE, y / TILE) {
                any = true;
                self.get(x, y)
            } else {
                let mut e = [0u16; 4];
                for (d, s) in e.iter_mut().zip(empty) {
                    *d = *s;
                }
                e
            }
        };
        let (a, b, c, d) = (get(ix, iy), get(ix + 1, iy), get(ix, iy + 1), get(ix + 1, iy + 1));
        for (k, slot) in out.iter_mut().enumerate().take(4) {
            let top = a[k] as f32 * (1.0 - wx) + b[k] as f32 * wx;
            let bot = c[k] as f32 * (1.0 - wx) + d[k] as f32 * wx;
            *slot = (top * (1.0 - wy) + bot * wy + 0.5) as u16;
        }
        any
    }

    /// Whether a bilinear read anywhere in the texel box `(x0, y0)..(x1, y1)` can touch an
    /// allocated tile (callers skip work for regions that would read only `empty`).
    pub fn any_allocated_in(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
        let span = |lo: f32, hi: f32, n: u32| ((lo - 1.0).max(0.0) as u32 / TILE, ((hi + 1.0).max(0.0) as u32 / TILE).min(n.saturating_sub(1)));
        let ((ax, bx), (ay, by)) = (span(x0.min(x1), x0.max(x1), self.tiles_x), span(y0.min(y1), y0.max(y1), self.tiles_y));
        (ay..=by).any(|sy| (ax..=bx).any(|sx| self.is_allocated(sx, sy)))
    }

    /// Tile (tx, ty) of a `width × height` image resampled through `map`: the center of
    /// destination texel (x, y) reads this image bilinearly at `map(x + 0.5, y + 0.5)` (texels;
    /// None leaves `empty`). None when no texel of the tile reads an allocated source texel.
    pub fn remapped_tile(&self, width: u32, height: u32, tx: u32, ty: u32, empty: &[u16], map: &dyn Fn(f32, f32) -> Option<(f32, f32)>) -> Option<Vec<u16>> {
        if self.allocated_count() == 0 || self.width == 0 || self.height == 0 {
            return None;
        }
        let comps = self.comps;
        let mut data = vec![0u16; TILE_AREA * comps];
        let mut any = false;
        for ly in 0..TILE.min(height.saturating_sub(ty * TILE)) {
            for lx in 0..TILE.min(width.saturating_sub(tx * TILE)) {
                let o = ((ly * TILE + lx) as usize) * comps;
                let Some(out) = data.get_mut(o..o + comps) else { continue };
                match map((tx * TILE + lx) as f32 + 0.5, (ty * TILE + ly) as f32 + 0.5) {
                    Some((sx, sy)) if sx.is_finite() && sy.is_finite() => any |= self.bilinear(sx, sy, empty, out),
                    _ => {
                        for (d, s) in out.iter_mut().zip(empty) {
                            *d = *s;
                        }
                    }
                }
            }
        }
        any.then_some(data)
    }
}

#[cfg(test)]
mod resample_tests {
    use super::*;

    #[test]
    fn resample_up_and_down_keeps_values_and_sparsity() {
        let mut img = TileImage::new(128, 128, 2);
        for y in 0..64 {
            for x in 0..64 {
                img.set(x, y, &[30000, 65535]);
            }
        }
        let up = img.resampled(256, 256, &[]);
        assert_eq!(up.get(40, 40)[..2], [30000, 65535]);
        assert!(!up.is_allocated(3, 3), "empty regions stay sparse");
        let down = img.resampled(64, 64, &[]);
        assert_eq!(down.get(10, 10)[..2], [30000, 65535]);
    }

    #[test]
    fn remap_moves_texels_and_keeps_sparsity() {
        let mut img = TileImage::new(128, 128, 2);
        img.set(10, 20, &[1000, 65535]);
        // Mirror horizontally into a 256-wide image (destination x reads source 127.x - x / 2).
        let map = |x: f32, y: f32| (x < 256.0).then(|| (128.0 - x / 2.0, y / 2.0));
        let tile = |tx, ty| img.remapped_tile(256, 256, tx, ty, &[0, 0], &map);
        assert!(tile(0, 0).is_none(), "nothing allocated is read there");
        let t = tile(3, 0).unwrap();
        // Destination (234, 41) center (234.5, 41.5) reads (10.75, 20.75): mostly texel (10, 20).
        let v = t[((41 * TILE + (234 - 192)) * 2) as usize];
        assert!(v > 500, "{v}");
    }
}
