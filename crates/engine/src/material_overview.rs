//! A bounded, disposable material image for a viewport with a cold composite.
use std::sync::Arc;

use texturecraft_compose::{BakedMaps, Composite, DisplayKind, Inputs, all_tiles, composite_tiles, kinds_for, tile_bytes};
use texturecraft_core::Channel;
use texturecraft_geom::UvGBuffer;
use texturecraft_texel::TILE;

use crate::Session;

/// Packed display maps, sampled across the entire atlas before full-size tiles arrive.
#[derive(Clone)]
pub struct MaterialOverview {
    pub size: (u32, u32),
    pub maps: Vec<(DisplayKind, Vec<u8>)>,
}

impl MaterialOverview {
    /// Saved material channels have already been evaluated by the source application.
    pub(crate) fn from_stored(composite: &Composite) -> Self {
        let size = (composite.resolution, composite.height);
        let channels: Vec<_> = composite.channels.keys().copied().collect();
        let maps = pack_overview(composite, &BakedMaps::default(), &channels, &all_tiles(size.0, size.1));
        Self { size, maps }
    }
}

impl Session {
    /// Evaluate the ordinary material stack on a small UV grid, sharing its source pixels,
    /// images and bakes. This never enters a paint, undo or export cache.
    pub fn material_overview(&self, ts: usize, max_edge: u32) -> Option<MaterialOverview> {
        let project = self.solo_view();
        let project = project.as_ref().unwrap_or(&self.doc.project);
        let set = project.texture_sets.get(ts)?;
        // A small overview must not replace the graph cooks used by the full-size composite.
        if project.render_tree(&set.stack).iter().filter_map(|id| project.layers.get(id)).any(|layer| !layer.graphs().is_empty()) {
            return None;
        }
        let (w, h) = set.size();
        let limit = max_edge.clamp(TILE, 256);
        let mut step = 1;
        while w.max(h) / step > limit {
            step *= 2;
        }
        let (width, height) = ((w / step).max(1), (h / step).max(1));
        let cache = self.caches.get(ts)?;
        let source = &cache.gbuf;
        // Sample the existing UV raster. Rasterizing a many-triangle mesh again can take
        // longer than evaluating the overview itself.
        let positions = (0..height).flat_map(|y| {
            (0..width).map(move |x| {
                let sx = ((x as u64 * 2 + 1) * w as u64 / (2 * width as u64)) as usize;
                let sy = ((y as u64 * 2 + 1) * h as u64 / (2 * height as u64)) as usize;
                sy * w as usize + sx
            })
        });
        let (tri, bary) =
            positions.map(|i| (source.tri.get(i).copied().unwrap_or(texturecraft_geom::NO_TRI), source.bary.get(i).copied().unwrap_or_default())).unzip();
        let gbuf = Arc::new(UvGBuffer { width, height, tri, bary, overlaps: 0 });
        let empty = BakedMaps::default();
        let baked = self.doc.baked.get(&ts).unwrap_or(&empty);
        let channels = set.channel_list();
        let scales = set.block_scales().at_reduced_resolution(step);
        let inputs = Inputs {
            project,
            ts,
            mesh: &self.mesh,
            gbuf: &gbuf,
            pixels: &self.doc.pixels,
            baked,
            block_scales: scales.clone(),
            preview: None,
            images: &self.images,
            graphs: texturecraft_compose::GraphStore::empty(),
            bounds: self.mesh.bounds(),
        };
        let mut composite = Composite { block_scales: scales, ..Composite::with_size(&channels, width, height) };
        let tiles = all_tiles(width, height);
        composite_tiles(&inputs, &mut composite, &tiles);
        let padding =
            crate::session::DisplayPadding::new(&gbuf, gbuf.tri.iter().any(|&t| t != texturecraft_geom::NO_TRI), (set.block_resolution() / step).max(1));
        padding.apply_to_tiles(&mut composite, &tiles);
        let maps = pack_overview(&composite, baked, &channels, &tiles);
        Some(MaterialOverview { size: (width, height), maps })
    }
}

fn pack_overview(composite: &Composite, baked: &BakedMaps, channels: &[Channel], tiles: &[(u32, u32)]) -> Vec<(DisplayKind, Vec<u8>)> {
    let (width, height) = (composite.resolution, composite.height);
    kinds_for(channels)
        .map(|kind| {
            let stride = kind.bytes_per_texel();
            let mut image = vec![0; width as usize * height as usize * stride];
            for &(tx, ty) in tiles {
                let data = tile_bytes(composite, baked, kind, tx, ty);
                let (left, top) = (tx * TILE, ty * TILE);
                let row_bytes = (width - left).min(TILE) as usize * stride;
                for (row, src) in data.chunks_exact(TILE as usize * stride).take((height - top) as usize).enumerate() {
                    let offset = ((top as usize + row) * width as usize + left as usize) * stride;
                    image[offset..offset + row_bytes].copy_from_slice(&src[..row_bytes]);
                    // A preview tile can span several mixed-resolution UDIM blocks.
                    if kind == DisplayKind::Extra {
                        for (x, px) in image[offset..offset + row_bytes].chunks_exact_mut(stride).enumerate() {
                            px[3] = (-32.0 * composite.block_scales.at(left + x as u32, top + row as u32).log2()).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
            (kind, image)
        })
        .collect()
}
