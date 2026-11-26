use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use egui::{ColorImage, TextureHandle, TextureOptions};
use texturecraft_engine::Session;
use texturecraft_engine::core::{BakedMap, BlockScales};

/// Side of a thumbnail's texture, in pixels (twice the row's square, for high-density screens)
const THUMBNAIL_PIXELS: u32 = 44;

/// A thumbnail's texture, with the baked data, the working size and the UV tiles' scales it was drawn from
type Thumbnail = (Weak<Vec<f32>>, (u32, u32), BlockScales, TextureHandle);

/// The Mesh Map Bakers rows' thumbnails of the baked maps, redrawn when a map is baked again
#[derive(Default)]
pub struct MapThumbnails {
    mesh_id: Option<u64>,
    textures: BTreeMap<(usize, BakedMap), Thumbnail>,
}

impl MapThumbnails {
    pub(crate) fn retain_mesh(&mut self, mesh_id: u64) {
        if self.mesh_id != Some(mesh_id) {
            self.textures.clear();
            self.mesh_id = Some(mesh_id);
        }
    }

    /// The thumbnail of `map` as texture set `texture_set` has it baked (a running bake's finished maps included), `None` before it is baked
    pub fn texture(&mut self, context: &egui::Context, session: &Session, texture_set: usize, map: BakedMap) -> Option<TextureHandle> {
        self.retain_mesh(session.mesh_id);
        let key = (texture_set, map);
        let baked = session.bake_preview_baked(texture_set).filter(|preview| preview.maps.contains_key(&map)).or_else(|| session.doc.baked.get(&texture_set));
        let (Some(data), Some(cache)) = (baked.and_then(|baked| baked.maps.get(&map)), session.caches.get(texture_set)) else {
            self.textures.remove(&key);
            return None;
        };

        if let Some((drawn_from, working_size, block_scales, texture)) = self.textures.get(&key)
            && drawn_from.ptr_eq(&Arc::downgrade(data))
            && *working_size == cache.size
            && *block_scales == cache.composite.block_scales
        {
            return Some(texture.clone());
        }

        let rgba = texturecraft_render::mesh_maps::mesh_map_rgba(baked, map, THUMBNAIL_PIXELS, cache.size, &cache.composite.block_scales);
        let side = THUMBNAIL_PIXELS as usize;
        let image = ColorImage::from_rgba_unmultiplied([side, side], &rgba);
        let texture = context.load_texture(format!("bake_map_{texture_set}_{}", map.id()), image, TextureOptions::LINEAR);
        self.textures.insert(key, (Arc::downgrade(data), cache.size, cache.composite.block_scales.clone(), texture.clone()));
        Some(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_is_kept_until_its_map_is_baked_again() {
        let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
        let context = egui::Context::default();
        let mut thumbnails = MapThumbnails::default();
        let map = *session.doc.baked[&0].maps.keys().next().expect("the sample has a baked map");

        let first = thumbnails.texture(&context, &session, 0, map).expect("a baked map has a thumbnail");
        assert_eq!(first.size(), [THUMBNAIL_PIXELS as usize; 2]);
        assert_eq!(thumbnails.texture(&context, &session, 0, map).unwrap().id(), first.id(), "the same bake reuses its texture");

        let baked = session.doc.baked.get_mut(&0).unwrap();
        let rebaked = baked.maps[&map].as_ref().clone();
        baked.maps.insert(map, Arc::new(rebaked));

        assert_ne!(thumbnails.texture(&context, &session, 0, map).unwrap().id(), first.id(), "a new bake is drawn again");

        let mut tiled =
            texturecraft_engine::Session::new_project("Tiles", texturecraft_engine::geom::primitives::udim_plane(), "builtin:udim_plane", None, 256).unwrap();
        let tiled_set = &mut tiled.doc.project.texture_sets[0];
        let tile = *tiled_set.uv_tiles.first().expect("the UDIM plane has UV tiles");
        tiled_set.tile_resolutions.insert(tile, 128);
        let second = thumbnails.texture(&context, &session, 0, map).unwrap();
        session.caches[0].composite.block_scales = tiled_set.block_scales();
        assert_ne!(thumbnails.texture(&context, &session, 0, map).unwrap().id(), second.id(), "a UV tile resolution edit draws it again");

        session.doc.baked.get_mut(&0).unwrap().maps.remove(&map);

        assert!(thumbnails.texture(&context, &session, 0, map).is_none(), "a map not baked has none");
    }
}
