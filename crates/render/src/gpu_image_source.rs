//! Image fill bindings and world-space decal frames for the GPU compositor.

use std::sync::Arc;

use texturecraft_compose::{ImageRes, ImageStore, pattern::decal_axes};
use texturecraft_core::{Decal, DecalFlip, Source};
use texturecraft_geom::{Mesh, glam::Vec3};

use crate::gpu_tile_pattern::TileImageAtlas;

pub(crate) const IMAGE_SOURCE_OPCODE: u32 = 14;
pub(crate) const DECAL_PROJECTION: u32 = 9;

#[derive(Debug, Default)]
pub(crate) struct MaterialImages {
    names: Vec<String>,
    images: Vec<Option<Arc<ImageRes>>>,
    decals: Vec<DecalSource>,
}

#[derive(Debug, PartialEq)]
struct DecalSource {
    frame: Decal,
    alpha_images: Vec<u32>,
}

impl MaterialImages {
    pub(crate) fn register_image(&mut self, name: &str) -> u32 {
        if let Some(index) = self.names.iter().position(|existing| existing == name) {
            return index as u32;
        }
        let index = self.names.len() as u32;
        self.names.push(name.to_owned());
        index
    }

    pub(crate) fn register_decal<'a>(&mut self, frame: Decal, sources: impl Iterator<Item = &'a Source>) -> u32 {
        let alpha_images = sources
            .filter_map(|source| match source {
                Source::Image { name, .. } => Some(self.register_image(name)),
                _ => None,
            })
            .collect();
        let decal = DecalSource { frame, alpha_images };
        if let Some(index) = self.decals.iter().position(|existing| *existing == decal) {
            return index as u32;
        }
        let index = self.decals.len() as u32;
        self.decals.push(decal);
        index
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.names.is_empty() && self.decals.is_empty()
    }

    pub(crate) fn set_images(&mut self, images: &ImageStore) {
        self.images = self.names.iter().map(|name| images.get(name).cloned()).collect();
    }

    pub(crate) fn sources(&self) -> impl Iterator<Item = &Arc<ImageRes>> {
        self.images.iter().flatten()
    }

    pub(crate) fn append_words(&self, words: &mut Vec<u32>, atlas: Option<&TileImageAtlas>, mesh: Option<&Mesh>) -> (Vec<u32>, Vec<u32>) {
        let mut index = 0;
        let image_offsets: Vec<_> = self
            .names
            .iter()
            .enumerate()
            .map(|(slot, _)| {
                if let Some((image, atlas)) = self.images.get(slot).and_then(Option::as_ref).zip(atlas) {
                    let offset = atlas.append_rgba(words, index, image.srgb);
                    index += 1;
                    offset
                } else {
                    u32::MAX
                }
            })
            .collect();
        let normalization = mesh.map(|mesh| {
            let (lo, hi) = mesh.bounds();
            ((lo + hi) * 0.5, ((hi - lo).length() * 0.5).max(1e-4))
        });
        let decal_offsets = self
            .decals
            .iter()
            .map(|decal| {
                let offset = words.len() as u32;
                let alpha = decal.alpha_images.iter().map(|&slot| image_offsets[slot as usize]).find(|&offset| offset != u32::MAX).unwrap_or(u32::MAX);
                words.extend(normalization.map_or([0; 16], |(center, radius)| decal_words(&decal.frame, alpha, center, radius)));
                offset
            })
            .collect();
        (image_offsets, decal_offsets)
    }
}

fn decal_words(decal: &Decal, alpha: u32, center: Vec3, radius: f32) -> [u32; 16] {
    let Some((origin, right, up, normal)) = decal_axes(decal) else {
        return [0; 16];
    };
    // The UV surface buffer already stores positions relative to this center and radius.
    let origin = (origin - center) / radius;
    let right = right * (radius / decal.size[0].max(1e-6));
    let up = up * (radius / decal.size[1].max(1e-6));
    let depth = decal.depth.max(1e-6) / radius;
    let flags = 1 | (u32::from(decal.flip[0]) << 1) | (u32::from(decal.flip[1]) << 2);
    let (flags, mirror) = decal.symmetry.map_or((flags, 0.0), |symmetry| {
        let flip = match symmetry.flip {
            DecalFlip::None => 0,
            DecalFlip::U => 1 << 5,
            DecalFlip::V => 1 << 6,
        };
        (flags | ((u32::from(symmetry.axis.min(2)) + 1) << 3) | flip, symmetry.offset / radius)
    });
    [
        origin.x.to_bits(),
        origin.y.to_bits(),
        origin.z.to_bits(),
        flags,
        right.x.to_bits(),
        right.y.to_bits(),
        right.z.to_bits(),
        mirror.to_bits(),
        up.x.to_bits(),
        up.y.to_bits(),
        up.z.to_bits(),
        alpha,
        normal.x.to_bits(),
        normal.y.to_bits(),
        normal.z.to_bits(),
        depth.to_bits(),
    ]
}
