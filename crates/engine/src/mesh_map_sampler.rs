//! Sample image resources into a mesh-map atlas or compare them with an existing atlas.

use std::collections::BTreeMap;
use std::sync::Arc;

use rayon::prelude::*;
use texturecraft_compose::ImageRes;
use texturecraft_core::{BakedMap, NormalFormat, TileGrid, udim_at};

use crate::commands::bad;
use crate::{Result, Session};

pub(crate) struct MeshMapSampler<'a> {
    width: u32,
    height: u32,
    components: usize,
    flip_green: bool,
    grid: Option<TileGrid>,
    first: &'a ImageRes,
    images: &'a BTreeMap<u32, Arc<ImageRes>>,
}

impl<'a> MeshMapSampler<'a> {
    pub(crate) fn new(session: &Session, cmd: &str, texture_set: usize, map: BakedMap, images: &'a BTreeMap<u32, Arc<ImageRes>>) -> Result<Self> {
        let set = session.doc.project.texture_set(texture_set)?;
        let grid = set.tile_grid();
        if let Some(tile) = set.uv_tiles.iter().find(|tile| grid.is_some() && !images.contains_key(tile)) {
            return Err(bad(cmd, format!("no image for UV tile {tile}")));
        }

        let (width, height) = crate::meshmap_cmds::image_map_size(session, cmd, texture_set)?;
        let first = images.values().next().ok_or_else(|| bad(cmd, "no image to use"))?;

        // Both tangent-space normal maps follow the project's green-channel convention
        let flip_green = matches!(map, BakedMap::Normal | BakedMap::BentNormal) && session.doc.project.normal_format == NormalFormat::DirectX;
        Ok(Self { width, height, components: map.components(), flip_green, grid, first, images })
    }

    fn row(&self, y: usize) -> impl Iterator<Item = [f32; 4]> + '_ {
        let v = (y as f32 + 0.5) / self.height as f32;
        (0..self.width).map(move |x| {
            let u = (x as f32 + 0.5) / self.width as f32;
            let mut pixel = match self.grid {
                None => self.first.sample(u, v),
                Some(grid) => {
                    let uv = grid.from_atlas([u, v]);
                    udim_at(uv).and_then(|tile| self.images.get(&tile)).map_or([0.0; 4], |image| image.sample(uv[0] - uv[0].floor(), uv[1] - uv[1].floor()))
                }
            };
            if self.flip_green {
                pixel[1] = 1.0 - pixel[1];
            }

            pixel
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn matches(&self, current: &[f32]) -> bool {
        let row_len = self.width as usize * self.components;
        current.len() == row_len * self.height as usize
            && current
                .par_chunks_exact(row_len)
                .enumerate()
                .all(|(y, row)| row.chunks_exact(self.components).zip(self.row(y)).all(|(stored, sampled)| stored == &sampled[..self.components]))
    }

    pub(crate) fn render(&self) -> (u32, Vec<f32>) {
        let row_len = self.width as usize * self.components;
        let mut pixels = vec![0.0; row_len * self.height as usize];
        pixels.par_chunks_exact_mut(row_len).enumerate().for_each(|(y, row)| {
            for (output, sampled) in row.chunks_exact_mut(self.components).zip(self.row(y)) {
                output.copy_from_slice(&sampled[..self.components]);
            }
        });

        (self.width, pixels)
    }
}
