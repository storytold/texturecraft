//! Which pending display tiles can help the current stroke or zoomed UV view.
use std::collections::BTreeSet;

use texturecraft_geom::glam::Vec2;
use texturecraft_texel::TILE;

use crate::Session;

type Tile = (u32, u32);

/// A hint for the active texture set. Only the CPU and upload work order changes.
#[derive(Clone, Copy, Default)]
pub struct TileDemand {
    pub(crate) brush: Option<Tile>,
    visible: Option<(Tile, Tile)>,
}

impl Session {
    /// Atlas UV bounds come from the displayed 2D view. The live brush already has its
    /// UV point or surface hit, including on meshes mapped into a tiled atlas.
    pub fn tile_demand(&self, visible_uv: Option<[Vec2; 2]>) -> TileDemand {
        let ts = self.ts();
        let Some(cache) = self.caches.get(ts) else { return TileDemand::default() };
        let (w, h) = cache.size;
        if w == 0 || h == 0 {
            return TileDemand::default();
        }
        let size = Vec2::new(w as f32, h as f32);
        let to_tile = |pixel: Vec2| ((pixel.x as u32).min(w - 1) / TILE, (pixel.y as u32).min(h - 1) / TILE);
        let brush = self.stroke.as_ref().filter(|s| s.ts == ts && s.target.is_some()).and_then(|s| {
            let uv = match &s.uv {
                Some(progress) => progress.last?,
                None => {
                    let hit = s.stroke.last_hit()?;
                    (hit.material == self.doc.project.texture_sets.get(ts)?.material).then_some(hit.uv)?
                }
            };
            (uv.cmpge(Vec2::ZERO).all() && uv.cmple(Vec2::ONE).all()).then(|| to_tile(uv * size))
        });
        let visible = visible_uv.and_then(|[lo, hi]| {
            if !lo.is_finite() || !hi.is_finite() {
                return None;
            }
            let (lo, hi) = (lo.max(Vec2::ZERO), hi.min(Vec2::ONE));
            if !lo.cmplt(hi).all() {
                return None;
            }
            let bounds = (to_tile(lo * size), to_tile((hi * size).ceil() - Vec2::ONE));
            // A view of the entire atlas benefits from the usual block ordering.
            (bounds != ((0, 0), ((w - 1) / TILE, (h - 1) / TILE))).then_some(bounds)
        });
        let brush = if self.stroke.is_some() { brush } else { cache.finished_dab };
        TileDemand { brush, visible }
    }
}

impl TileDemand {
    /// A live brush or a released dab still waiting to be displayed.
    pub fn has_brush(self) -> bool {
        self.brush.is_some()
    }

    /// Whether the active set has a brush point or a proper subset of visible tiles.
    pub fn focused(self) -> bool {
        self.brush.is_some() || self.visible.is_some()
    }

    /// Remove wanted, pending tiles, up to the caller's slice. When all fit, the caller
    /// can drain the queue directly. A stationary view does not duplicate queue entries.
    pub(crate) fn take(self, pending: &mut BTreeSet<Tile>, limit: usize) -> Vec<Tile> {
        let mut tiles = Vec::new();
        if limit == 0 || pending.len() <= limit {
            return tiles;
        }
        if let Some((x, y)) = self.brush {
            if pending.remove(&(x, y)) {
                tiles.push((x, y));
            }
            take_rect(pending, (x.saturating_sub(1), y.saturating_sub(1)), (x.saturating_add(1), y.saturating_add(1)), limit, &mut tiles);
        }
        if let Some((lo, hi)) = self.visible {
            take_rect(pending, lo, hi, limit, &mut tiles);
        }
        tiles
    }
}

fn take_rect(pending: &mut BTreeSet<Tile>, lo: Tile, hi: Tile, limit: usize, tiles: &mut Vec<Tile>) {
    let mut left = lo.0;
    let start = tiles.len();
    while left <= hi.0 && tiles.len() < limit {
        // Seek to the next occupied column, then read only the requested rows in it.
        let Some(&(x, _)) = pending.range((left, lo.1)..=(hi.0, hi.1)).next() else { break };
        tiles.extend(pending.range((x, lo.1)..=(x, hi.1)).take(limit - tiles.len()).copied());
        let Some(next) = x.checked_add(1) else { break };
        left = next;
    }
    for tile in &tiles[start..] {
        pending.remove(tile);
    }
}
