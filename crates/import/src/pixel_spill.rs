//! The layer pixels of the texture sets an import has finished, moved out of memory until the import returns: a set
//! being built then holds its own pixels only, not every earlier set's. Tiles the images share are written once and
//! shared again when read back; what comes back is the same, bit for bit.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufReader, BufWriter, Read, Write};
use std::sync::Arc;

use texturecraft_compose::PixelStore;
use texturecraft_core::{Channel, LayerId};
use texturecraft_texel::TileImage;

/// Which image of a [`PixelStore`].
#[derive(Clone, Copy)]
enum Key {
    Layer(LayerId, Channel),
    Mask(LayerId),
}

/// One image moved out: its key, size, components and its tiles (coordinates and the number of the tile's data in
/// the file).
struct Moved {
    key: Key,
    size: (u32, u32),
    comps: usize,
    tiles: Vec<(u32, u32, usize)>,
}

/// See the module docs. Without a folder to write to (or in the web build) nothing moves.
#[derive(Default)]
pub struct PixelSpill {
    #[cfg(not(target_arch = "wasm32"))]
    file: Option<(std::path::PathBuf, BufWriter<std::fs::File>)>,
    moved: Vec<Moved>,
    /// Values of each tile written, in file order.
    lengths: Vec<usize>,
    /// A write failed: nothing more is written (what was is read back).
    broken: bool,
}

impl PixelSpill {
    /// Moves the images of `pixels` out of memory; an image that cannot be written stays. Returns how many moved.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spill(&mut self, pixels: &mut PixelStore) -> usize {
        if self.broken {
            return 0;
        }
        if self.file.is_none() {
            let Some(dir) = texturecraft_compose::graphs::spill::spill_folder("pixels") else { return 0 };
            let path = dir.join("pixels.bin");
            let Ok(file) = std::fs::File::create(&path) else {
                let _ = std::fs::remove_dir_all(&dir);
                return 0;
            };
            self.file = Some((path, BufWriter::new(file)));
        }
        let keys: Vec<Key> = pixels.layers.keys().map(|(id, c)| Key::Layer(*id, *c)).chain(pixels.masks.keys().map(|id| Key::Mask(*id))).collect();
        // tiles written in this call by their data's address: the images that share them are all alive until it ends
        let mut written: HashMap<usize, usize> = HashMap::new();
        let mut out: Vec<TileImage> = Vec::new();
        let mut count = 0;
        for key in keys {
            let Some(img) = (match key {
                Key::Layer(id, c) => pixels.layers.get(&(id, c)),
                Key::Mask(id) => pixels.masks.get(&id),
            }) else {
                continue;
            };
            let Some(tiles) = self.write(img, &mut written) else {
                self.broken = true;
                break;
            };
            self.moved.push(Moved { key, size: (img.width(), img.height()), comps: img.comps(), tiles });
            let taken = match key {
                Key::Layer(id, c) => pixels.layers.remove(&(id, c)),
                Key::Mask(id) => pixels.masks.remove(&id),
            };
            out.extend(taken);
            count += 1;
        }
        drop(out);
        count
    }

    #[cfg(target_arch = "wasm32")]
    pub fn spill(&mut self, _pixels: &mut PixelStore) -> usize {
        0
    }

    /// Writes the tiles of `img` not written yet; their coordinates and numbers, or `None` when writing failed.
    #[cfg(not(target_arch = "wasm32"))]
    fn write(&mut self, img: &TileImage, written: &mut HashMap<usize, usize>) -> Option<Vec<(u32, u32, usize)>> {
        let (_, w) = self.file.as_mut()?;
        let mut tiles = Vec::new();
        for (tx, ty) in img.allocated() {
            let handle = img.tile_handle(tx, ty)?;
            let at = Arc::as_ptr(handle) as usize;
            let number = match written.get(&at) {
                Some(n) => *n,
                None => {
                    let bytes: Vec<u8> = handle.iter().flat_map(|v| v.to_le_bytes()).collect();
                    w.write_all(&bytes).ok()?;
                    self.lengths.push(handle.len());
                    written.insert(at, self.lengths.len() - 1);
                    self.lengths.len() - 1
                }
            };
            tiles.push((tx, ty, number));
        }
        w.flush().ok()?;
        Some(tiles)
    }

    /// Puts every image moved out back into `pixels`, and removes the file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn restore(mut self, pixels: &mut PixelStore) -> std::io::Result<()> {
        let Some((path, w)) = self.file.take() else { return Ok(()) };
        drop(w);
        let read = (|| -> std::io::Result<Vec<Arc<Vec<u16>>>> {
            let mut r = BufReader::new(std::fs::File::open(&path)?);
            let mut tiles = Vec::with_capacity(self.lengths.len());
            let mut bytes = Vec::new();
            for len in &self.lengths {
                bytes.resize(len * 2, 0);
                r.read_exact(&mut bytes)?;
                tiles.push(Arc::new(bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect()));
            }
            Ok(tiles)
        })();
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
        let tiles = read?;
        for m in std::mem::take(&mut self.moved) {
            let mut img = TileImage::new(m.size.0, m.size.1, m.comps);
            for (tx, ty, number) in m.tiles {
                img.set_tile(tx, ty, tiles.get(number).cloned());
            }
            match m.key {
                Key::Layer(id, c) => pixels.layers.insert((id, c), img),
                Key::Mask(id) => pixels.masks.insert(id, img),
            };
        }
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn restore(self, _pixels: &mut PixelStore) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for PixelSpill {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((path, w)) = self.file.take() {
            drop(w);
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moved_images_come_back_the_same_with_their_shared_tiles_shared() {
        let mut pixels = PixelStore::default();
        let mut a = TileImage::new(300, 200, 4);
        a.tile_mut(0, 0, &[1, 2, 3, 4]).unwrap()[7] = 9;
        a.tile_mut(1, 0, &[5, 6, 7, 8]);
        let mut b = TileImage::new(300, 200, 4);
        b.set_tile(0, 1, a.tile_handle(0, 0).cloned());
        let mut m = TileImage::new(300, 200, 2);
        m.tile_mut(1, 1, &[100, 200]);
        pixels.layers.insert((LayerId(1), Channel::BaseColor), a.clone());
        pixels.layers.insert((LayerId(2), Channel::BaseColor), b.clone());
        pixels.masks.insert(LayerId(3), m.clone());

        let mut spill = PixelSpill::default();
        assert_eq!(spill.spill(&mut pixels), 3);
        assert!(pixels.layers.is_empty() && pixels.masks.is_empty());
        assert_eq!(spill.lengths.len(), 3, "the shared tile is written once");
        pixels.layers.insert((LayerId(4), Channel::Roughness), TileImage::new(8, 8, 2));
        spill.restore(&mut pixels).unwrap();

        assert_eq!(pixels.layers[&(LayerId(1), Channel::BaseColor)], a);
        assert_eq!(pixels.layers[&(LayerId(2), Channel::BaseColor)], b);
        assert_eq!(pixels.masks[&LayerId(3)], m);
        assert!(pixels.layers.contains_key(&(LayerId(4), Channel::Roughness)), "images added since stay");
        let shared = |id| pixels.layers[&(LayerId(id), Channel::BaseColor)].tile_handle(0, if id == 1 { 0 } else { 1 }).cloned().unwrap();
        assert!(Arc::ptr_eq(&shared(1), &shared(2)), "tiles shared before are shared again");
    }
}
