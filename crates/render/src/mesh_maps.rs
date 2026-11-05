//! Debug textures for the mesh-map view.

use texturecraft_compose::BakedMaps;
use texturecraft_core::BakedMap;

/// RGBA8 of baked map `m` resampled to `size × size` stretched over the map (mid gray where not
/// baked). A bake larger than `size` is area-averaged down (as the compositor reads it), so thin
/// details keep their weight instead of being hit or missed. Scalar maps are shown as
/// grayscale; ID maps as their colors. Values are shown as stored, i.e. as display values.
pub fn mesh_map_rgba(baked: Option<&BakedMaps>, m: BakedMap, size: u32) -> Vec<u8> {
    let size = size.max(1);
    let mut out = vec![128u8; (size as usize).pow(2) * 4];
    let Some(b) = baked.filter(|b| b.maps.contains_key(&m)) else { return out };
    let view = b.view((size, size), &[m]);
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i % size as usize) as u32, (i / size as usize) as u32);
        let v = view.sample(m, x, y).unwrap_or([0.5; 3]);
        for (d, v) in px.iter_mut().zip(v) {
            *d = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        px[3] = 255;
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A `res²` AO bake: white with dark 1-texel-wide vertical lines every `period` texels.
    fn creases(res: u32, period: u32) -> BakedMaps {
        let ao = (0..res * res).map(|i| if (i % res).is_multiple_of(period) { 0.0 } else { 1.0 }).collect();
        BakedMaps { resolution: res, maps: [(BakedMap::AmbientOcclusion, Arc::new(ao))].into() }
    }

    #[test]
    fn a_larger_bake_is_averaged_not_point_sampled() {
        // 512² bake shown at 64²: every view texel covers 8 columns, one of them a crease.
        let rgba = mesh_map_rgba(Some(&creases(512, 8)), BakedMap::AmbientOcclusion, 64);
        let shown = (0.875f32 * 255.0 + 0.5) as u8;
        assert!(rgba.chunks_exact(4).all(|px| px == [shown, shown, shown, 255]), "{:?}", &rgba[..16]);
    }

    #[test]
    fn same_size_shows_every_texel() {
        let rgba = mesh_map_rgba(Some(&creases(64, 8)), BakedMap::AmbientOcclusion, 64);
        for (i, px) in rgba.chunks_exact(4).enumerate() {
            let v = if (i % 64).is_multiple_of(8) { 0 } else { 255 };
            assert_eq!(px, [v, v, v, 255], "texel {i}");
        }
        assert!(mesh_map_rgba(None, BakedMap::AmbientOcclusion, 4).iter().all(|v| *v == 128));
    }
}
