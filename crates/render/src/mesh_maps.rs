//! Debug textures for the mesh-map view.

use texturecraft_compose::BakedMaps;
use texturecraft_core::{BakedMap, BlockScales};

/// RGBA8 of baked map `m` resampled to `size × size` stretched over the working atlas of
/// `working_size` texels that `scales` lays out (mid gray where not baked). Where the view
/// samples a tile's block more coarsely than it was baked, the bake is area-averaged down (as
/// the compositor reads it), so thin details keep their weight instead of being hit or missed.
/// Scalar maps are shown as grayscale; ID maps as their colors. Values are shown as stored,
/// i.e. as display values.
pub fn mesh_map_rgba(baked: Option<&BakedMaps>, m: BakedMap, size: u32, working_size: (u32, u32), scales: &BlockScales) -> Vec<u8> {
    let size = size.max(1);
    let mut out = vec![128u8; (size as usize).pow(2) * 4];
    let Some(b) = baked.filter(|b| b.maps.contains_key(&m)) else { return out };

    let view = b.view((size, size), working_size, scales, &[m]);
    for (i, pixel) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i % size as usize) as u32, (i / size as usize) as u32);
        let v = view.sample(m, x, y).unwrap_or([0.5; 3]);
        for (d, v) in pixel.iter_mut().zip(v) {
            *d = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        pixel[3] = 255;
    }
    out
}

/// RGBA8 of the neutral value of map `m` (what a map not baked yet shows during a bake: flat
/// normals, white occlusion, mid-gray height ...), `size × size`, encoded like [`mesh_map_rgba`].
pub fn neutral_rgba(m: BakedMap, size: u32) -> Vec<u8> {
    let value = m.neutral();
    let pixel = [0, 1, 2].map(|k| (value[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    let size = size.max(1);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for _ in 0..size * size {
        out.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use texturecraft_core::TextureSet;

    #[test]
    fn mesh_map_preview_places_bake_detail_in_the_working_tile() {
        let mut set = TextureSet::new("Tiles", 0, 256);
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 128);
        let pixels = (0..128).flat_map(|y| (0..256).map(move |x| ((x % 128) + y) as f32 / 255.0)).collect();
        let baked = BakedMaps { resolution: 256, maps: [(BakedMap::AmbientOcclusion, Arc::new(pixels))].into() };
        let preview = mesh_map_rgba(Some(&baked), BakedMap::AmbientOcclusion, 512, set.size(), &set.block_scales());
        assert_eq!(preview[(240 * 512 + 376) * 4], 240);
        assert_eq!(preview[(400 * 512 + 500) * 4], 128, "unused working block area stays neutral");
    }

    /// A `res²` AO bake: white with dark 1-texel-wide vertical lines every `period` texels.
    fn creases(res: u32, period: u32) -> BakedMaps {
        let ao = (0..res * res).map(|i| if (i % res).is_multiple_of(period) { 0.0 } else { 1.0 }).collect();
        BakedMaps { resolution: res, maps: [(BakedMap::AmbientOcclusion, Arc::new(ao))].into() }
    }

    #[test]
    fn a_larger_bake_is_averaged_not_point_sampled() {
        // 512² bake shown at 64²: every view texel covers 8 columns, one of them a crease.
        let rgba = mesh_map_rgba(Some(&creases(512, 8)), BakedMap::AmbientOcclusion, 64, (512, 512), &BlockScales::default());
        let shown = (0.875f32 * 255.0 + 0.5) as u8;
        assert!(rgba.as_chunks::<4>().0.iter().all(|px| *px == [shown, shown, shown, 255]), "{:?}", &rgba[..16]);
    }

    #[test]
    fn same_size_shows_every_texel() {
        let rgba = mesh_map_rgba(Some(&creases(64, 8)), BakedMap::AmbientOcclusion, 64, (64, 64), &BlockScales::default());
        for (i, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let v = if (i % 64).is_multiple_of(8) { 0 } else { 255 };
            assert_eq!(*px, [v, v, v, 255], "texel {i}");
        }
        assert!(mesh_map_rgba(None, BakedMap::AmbientOcclusion, 4, (4, 4), &BlockScales::default()).iter().all(|v| *v == 128));
    }

    #[test]
    fn each_uv_tile_is_averaged_at_the_resolution_the_view_shows_it() {
        // Tiles 1001 (256²) and 1002 (64², a quarter of its block) over a 512² bake with a
        // crease every 8 columns (256 × 512 per tile), shown at 128²: 64 view texels across 1001's 256 baked
        // texels (4 each), 16 across 1002's 256 (16 each).
        let mut set = TextureSet::new("Tiles", 0, 256);
        set.uv_tiles = vec![1001, 1002];
        set.tile_resolutions.insert(1002, 64);
        let rgba = mesh_map_rgba(Some(&creases(512, 8)), BakedMap::AmbientOcclusion, 128, set.size(), &set.block_scales());
        let red = |x: usize, y: usize| rgba[(y * 128 + x) * 4];
        let quarter = (0.75f32 * 255.0 + 0.5) as u8;
        let eighth = (0.875f32 * 255.0 + 0.5) as u8;
        // 1001: every other view texel holds a crease (a quarter of its 4 columns).
        assert_eq!((red(0, 10), red(1, 10), red(2, 10)), (quarter, 255, quarter));
        // 1002: every view texel covers two creases in its 16 columns.
        assert!((64..80).all(|x| red(x, 5) == eighth), "{:?}", (64..80).map(|x| red(x, 5)).collect::<Vec<_>>());
        assert_eq!(red(100, 40), 128, "unused working block area stays neutral");
    }
}
