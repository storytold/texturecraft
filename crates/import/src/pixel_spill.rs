//! The layer pixels and baked maps of the texture sets an import has finished, moved out of memory: a set being built
//! then holds its own only, not every earlier set's, and the import hands them back as images and maps that read
//! from the spill file when needed (a save of the import writes them one by one). Tiles the images share are written
//! once; what is read back is the same, bit for bit.

#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

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

    /// Puts every image moved out back into `pixels` as an image that reads its tiles from the file when it is first
    /// needed ([`TileImage::lazy`]): the import's result, and a save of it, need not hold every set's pixels at once.
    /// The file goes when the last such image does.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn hand_over(mut self, pixels: &mut PixelStore) {
        let Some((path, w)) = self.file.take() else { return };
        drop(w);
        let file = Arc::new(SpillFile(path));
        // where each tile's data starts in the file
        let mut offsets = Vec::with_capacity(self.lengths.len());
        let mut at = 0u64;
        for len in &self.lengths {
            offsets.push((at, *len));
            at += *len as u64 * 2;
        }
        let offsets = Arc::new(offsets);
        for m in std::mem::take(&mut self.moved) {
            let (file, offsets, size, comps) = (file.clone(), offsets.clone(), m.size, m.comps);
            let count = m.tiles.len();
            let tiles = m.tiles;
            let source: texturecraft_texel::TileSource = Arc::new(move || {
                use std::io::Seek;
                let probe = TileImage::new(size.0, size.1, comps);
                let mut slots = vec![None; (probe.tiles_x() * probe.tiles_y()) as usize];
                let mut r = BufReader::new(std::fs::File::open(&file.0).ok()?);
                let mut bytes = Vec::new();
                for (tx, ty, number) in &tiles {
                    let (start, len) = *offsets.get(*number)?;
                    r.seek(std::io::SeekFrom::Start(start)).ok()?;
                    bytes.resize(len * 2, 0);
                    r.read_exact(&mut bytes).ok()?;
                    let slot = slots.get_mut((ty * probe.tiles_x() + tx) as usize)?;
                    *slot = Some(Arc::new(bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()));
                }
                Some(slots)
            });
            let img = TileImage::lazy(size.0, size.1, comps, count, source);
            match m.key {
                Key::Layer(id, c) => pixels.layers.insert((id, c), img),
                Key::Mask(id) => pixels.masks.insert(id, img),
            };
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn hand_over(self, _pixels: &mut PixelStore) {}

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
                tiles.push(Arc::new(bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()));
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

/// The baked maps of the texture sets an import has finished, moved to a file and read back from it when needed
/// ([`texturecraft_compose::Texels::lazy`]); the file goes when the last such map does.
#[derive(Default)]
pub struct BakedSpill {
    #[cfg(not(target_arch = "wasm32"))]
    file: Option<(Arc<SpillFile>, BufWriter<std::fs::File>, u64)>,
    broken: bool,
}

impl BakedSpill {
    /// Moves the maps of `baked` that are held (not read from a file already) out of memory. Returns how many moved.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spill(&mut self, baked: &mut std::collections::BTreeMap<usize, texturecraft_compose::BakedMaps>) -> usize {
        use texturecraft_compose::Texels;
        if self.broken {
            return 0;
        }
        if self.file.is_none() {
            let Some(dir) = texturecraft_compose::graphs::spill::spill_folder("baked") else { return 0 };
            let path = dir.join("baked.bin");
            let Ok(file) = std::fs::File::create(&path) else {
                let _ = std::fs::remove_dir_all(&dir);
                return 0;
            };
            self.file = Some((Arc::new(SpillFile(path)), BufWriter::new(file), 0));
        }
        let Some((file, w, at)) = self.file.as_mut() else { return 0 };
        let mut moved = 0;
        for maps in baked.values_mut() {
            for map in maps.maps.values_mut() {
                if matches!(**map, Texels::Lazy(_)) {
                    continue;
                }
                let start = *at;
                let written = map.try_for_chunks(1 << 16, |chunk| {
                    let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                    w.write_all(&bytes)
                });
                if written.and_then(|()| w.flush()).is_err() {
                    self.broken = true;
                    return moved;
                }
                let len = map.len();
                *at += len as u64 * 4;
                let file = file.clone();
                *map = Arc::new(Texels::lazy(
                    len,
                    Arc::new(move || {
                        use std::io::Seek;
                        let mut r = BufReader::new(std::fs::File::open(&file.0).ok()?);
                        r.seek(std::io::SeekFrom::Start(start)).ok()?;
                        let mut bytes = vec![0u8; len * 4];
                        r.read_exact(&mut bytes).ok()?;
                        Some(bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect())
                    }),
                ));
                moved += 1;
            }
        }
        moved
    }

    #[cfg(target_arch = "wasm32")]
    pub fn spill(&mut self, _baked: &mut std::collections::BTreeMap<usize, texturecraft_compose::BakedMaps>) -> usize {
        0
    }
}

/// The spill file of images handed over: removed (with its folder) when the last of them goes.
#[cfg(not(target_arch = "wasm32"))]
struct SpillFile(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for SpillFile {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
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
    use texturecraft_texel::TILE_AREA;

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

    #[test]
    fn baked_maps_moved_out_read_back_the_same_until_the_last_goes() {
        use texturecraft_compose::{BakedMaps, Texels};
        use texturecraft_core::BakedMap;
        let ao: Vec<f32> = (0..64).map(|i| f32::from(i as u16 * 1000) / 65535.0).collect();
        let normal: Vec<f32> = (0..64 * 3).map(|i| i as f32 / 191.0).collect();
        let maps = BakedMaps {
            resolution: 8,
            maps: [(BakedMap::AmbientOcclusion, Arc::new(Texels::new(ao.clone()))), (BakedMap::Normal, Arc::new(Texels::new(normal.clone())))].into(),
        };
        let mut baked: std::collections::BTreeMap<usize, BakedMaps> = [(0, maps.clone()), (1, maps.clone())].into();
        let mut spill = BakedSpill::default();
        assert_eq!(spill.spill(&mut baked), 4);
        assert_eq!(spill.spill(&mut baked), 0, "maps read from the file stay so");
        let dir = spill.file.as_ref().and_then(|(f, _, _)| f.0.parent().map(std::path::Path::to_path_buf)).unwrap();
        drop(spill);
        assert!(baked.values().flat_map(|b| b.maps.values()).all(|m| m.bytes() == 0));
        assert_eq!(baked[&1], maps);
        assert_eq!(baked[&0].maps[&BakedMap::Normal].to_f32().as_ref(), normal.as_slice());
        drop(baked);
        assert!(!dir.exists());
    }

    #[test]
    fn images_handed_over_read_their_tiles_from_the_file_until_the_last_goes() {
        let mut pixels = PixelStore::default();
        let mut a = TileImage::new(300, 200, 4);
        a.tile_mut(0, 0, &[1, 2, 3, 4]).unwrap()[7] = 9;
        a.tile_mut(4, 3, &[5, 6, 7, 8]);
        let mut m = TileImage::new(300, 200, 2);
        m.set_tile(2, 2, a.tile_handle(0, 0).map(|t| Arc::new(t.iter().take(TILE_AREA * 2).copied().collect())));
        pixels.layers.insert((LayerId(1), Channel::BaseColor), a.clone());
        pixels.masks.insert(LayerId(3), m.clone());
        let mut spill = PixelSpill::default();
        assert_eq!(spill.spill(&mut pixels), 2);
        let dir = spill.file.as_ref().and_then(|(p, _)| p.parent().map(std::path::Path::to_path_buf)).unwrap();
        spill.hand_over(&mut pixels);
        let back = pixels.layers[&(LayerId(1), Channel::BaseColor)].clone();
        assert!(!back.is_loaded() && back.allocated_count() == 2);
        assert_eq!(back, a);
        assert_eq!(pixels.masks[&LayerId(3)], m);
        assert!(dir.exists());
        drop(pixels);
        assert!(dir.exists(), "a clone still reads from it");
        drop(back);
        assert!(!dir.exists(), "gone with the last image");
    }
}
