//! Packed 16-bit project tiles and the lookup table for a window of their atlas.

use texturecraft_texel::{TILE, TILE_AREA, TileImage};

#[derive(Clone, Copy)]
pub(crate) struct TileWindow {
    pub origin: [u32; 2],
    pub size: [u32; 2],
}

const COMPOSITE_IMAGE_WORDS: usize = 6;

/// Per-image windows for compositing, where a linked source can have a different resolution.
pub(crate) enum CompositePaintWindow {
    Uniform(TileWindow),
    PerImage(Vec<TileWindow>),
}

impl CompositePaintWindow {
    pub fn covering(min: [u32; 2], max: [u32; 2], size: [u32; 2], images: &[TileImage]) -> Self {
        let common = TileWindow::covering(min, max, size);
        if images.iter().all(|image| [image.width(), image.height()] == size) {
            return Self::Uniform(common);
        }
        let windows = images
            .iter()
            .map(|image| {
                let source = [image.width(), image.height()];
                if source == size {
                    return common;
                }
                // Include both bilinear neighbours of each output texel, even at tile edges.
                let start = std::array::from_fn(|i| (u64::from(min[i]) * u64::from(source[i]) / u64::from(size[i])) as u32);
                let end = std::array::from_fn(|i| (u64::from(max[i]) * u64::from(source[i])).div_ceil(u64::from(size[i])) as u32);
                TileWindow::covering(start.map(|p| p.saturating_sub(1)), end.map(|p| p.saturating_add(2)), source)
            })
            .collect();
        Self::PerImage(windows)
    }

    pub fn params(&self) -> [u32; 4] {
        match self {
            Self::Uniform(window) => [window.origin[0], window.origin[1], window.size[0], window.size[1]],
            Self::PerImage(_) => [u32::MAX, 0, 0, 0],
        }
    }

    pub fn words(&self, images: &[TileImage], limit: usize) -> Option<usize> {
        let windows = match self {
            Self::Uniform(window) => return window.words(images, limit),
            Self::PerImage(windows) => windows,
        };
        let mut words = images.len().max(1);
        for (&window, image) in windows.iter().zip(images) {
            words += COMPOSITE_IMAGE_WORDS;
            words = window.image_words(image, words, limit)?;
        }
        (words <= limit).then_some(words)
    }

    /// One offset per image, then the image size, tile window, tile offsets and packed pixels.
    pub fn pack(&self, images: &[TileImage], words: usize, output: &mut Vec<u32>) {
        let windows = match self {
            Self::Uniform(window) => return window.pack(images, words, output),
            Self::PerImage(windows) => windows,
        };
        output.clear();
        output.reserve(words);
        output.resize(images.len().max(1), 0);
        for (i, (&window, image)) in windows.iter().zip(images).enumerate() {
            output[i] = output.len() as u32;
            output.extend([image.width(), image.height(), window.origin[0], window.origin[1], window.size[0], window.size[1]]);
            window.pack_image(image, output);
        }
    }
}

impl TileWindow {
    pub fn covering(min: [u32; 2], max: [u32; 2], image_size: [u32; 2]) -> Self {
        let origin = min.map(|p| p / TILE);
        let end = [max[0].min(image_size[0]).div_ceil(TILE), max[1].min(image_size[1]).div_ceil(TILE)];
        Self { origin, size: [end[0].saturating_sub(origin[0]), end[1].saturating_sub(origin[1])] }
    }

    pub fn words(self, images: &[TileImage], limit: usize) -> Option<usize> {
        let mut words = images.len().max(1);
        for image in images {
            words = self.image_words(image, words, limit)?;
        }
        (words <= limit).then_some(words)
    }

    fn image_words(self, image: &TileImage, mut words: usize, limit: usize) -> Option<usize> {
        words += (self.size[0] * self.size[1]) as usize;
        let per_tile = TILE_AREA * image.comps() / 2;
        for y in self.origin[1]..self.origin[1] + self.size[1] {
            let allocated = (self.origin[0]..self.origin[0] + self.size[0]).filter(|&x| image.is_allocated(x, y)).count();
            words += allocated * per_tile;
            if words > limit {
                return None;
            }
        }
        (words <= limit).then_some(words)
    }

    /// An offset per image, followed by each image's tile table and packed tile data.
    pub fn pack(self, images: &[TileImage], words: usize, output: &mut Vec<u32>) {
        output.clear();
        output.reserve(words);
        output.resize(images.len().max(1), 0);
        for (i, image) in images.iter().enumerate() {
            output[i] = output.len() as u32;
            self.pack_image(image, output);
        }
    }

    fn pack_image(self, image: &TileImage, output: &mut Vec<u32>) {
        let table = output.len();
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
