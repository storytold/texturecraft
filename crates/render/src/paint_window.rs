//! Packed 16-bit project tiles and the lookup table for a window of their atlas.

use texturecraft_texel::{TILE, TILE_AREA, TileImage};

#[derive(Clone, Copy)]
pub(crate) struct TileWindow {
    pub origin: [u32; 2],
    pub size: [u32; 2],
}

impl TileWindow {
    pub fn covering(min: [u32; 2], max: [u32; 2], image_size: [u32; 2]) -> Self {
        let origin = min.map(|p| p / TILE);
        let end = [max[0].min(image_size[0]).div_ceil(TILE), max[1].min(image_size[1]).div_ceil(TILE)];
        Self { origin, size: [end[0].saturating_sub(origin[0]), end[1].saturating_sub(origin[1])] }
    }

    pub fn words(self, images: &[TileImage], limit: usize) -> Option<usize> {
        let table = (self.size[0] * self.size[1]) as usize;
        let mut words = images.len().max(1);
        for image in images {
            words += table;
            let per_tile = TILE_AREA * image.comps() / 2;
            for y in self.origin[1]..self.origin[1] + self.size[1] {
                let allocated = (self.origin[0]..self.origin[0] + self.size[0]).filter(|&x| image.is_allocated(x, y)).count();
                words += allocated * per_tile;
                if words > limit {
                    return None;
                }
            }
        }
        Some(words)
    }

    /// An offset per image, followed by each image's tile table and packed tile data.
    pub fn pack(self, images: &[TileImage], words: usize, output: &mut Vec<u32>) {
        output.clear();
        output.reserve(words);
        output.resize(images.len().max(1), 0);
        for (i, image) in images.iter().enumerate() {
            let table = output.len();
            output[i] = table as u32;
            output.resize(table + (self.size[0] * self.size[1]) as usize, 0);
            for y in 0..self.size[1] {
                for x in 0..self.size[0] {
                    let Some(tile) = image.tile(x + self.origin[0], y + self.origin[1]) else { continue };
                    output[table + (y * self.size[0] + x) as usize] = output.len() as u32;
                    #[cfg(target_endian = "little")]
                    if let Ok(words) = bytemuck::try_cast_slice::<u16, u32>(tile) {
                        output.extend_from_slice(words);
                        continue;
                    }
                    output.extend(tile.as_chunks::<2>().0.iter().map(|&[lo, hi]| u32::from(lo) | (u32::from(hi) << 16)));
                }
            }
        }
    }
}
