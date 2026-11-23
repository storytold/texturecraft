//! Image fill bindings and world-space decal frames for the GPU compositor.

use std::sync::Arc;

use texturecraft_compose::{
    ImageRes, ImageStore,
    pattern::{decal_axes, mesh_frame},
};
use texturecraft_core::{Decal, DecalFlip, Sampling, Source};
use texturecraft_geom::{Mesh, glam::Vec3};

use crate::gpu_tile_pattern::{AtlasMips, RawPlane, TileImageAtlas};

pub(crate) const IMAGE_SOURCE_OPCODE: u32 = 14;
pub(crate) const DECAL_PROJECTION: u32 = 9;

/// What a registered image is read for. The GPU port packages register images only through
/// [`MaterialImages::register_image`]; P1 owns this type and adds kinds (mip-mapped, nearest, …)
/// as variants here, keeping that signature. P3 (Color Selection ID maps, effect image inputs)
/// and P7 (the mask image a stroke paints) read plain texels with `Material`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageUse {
    /// The image's texels alone (Bilinear Sharp and Nearest reads).
    Material,
    /// The image and its halved levels as stored: Bilinear HQ of a scalar, normal or alpha read.
    Mipmapped,
    /// The image and its halved levels for a colour read: an sRGB image's levels are linear.
    MipmappedColor,
}

#[derive(Debug, Default)]
pub(crate) struct MaterialImages {
    /// Each registered image with what it is read for: one image read two ways is held twice
    names: Vec<(String, ImageUse)>,
    images: Vec<Option<Arc<ImageRes>>>,
    decals: Vec<DecalSource>,
    /// The composited mesh's centre and radius (`set_surface`): P1's band uniform turns positions
    /// in mesh radii back into world positions with them
    mesh_frame: Option<(Vec3, f32)>,
}

#[derive(Debug, PartialEq)]
struct DecalSource {
    frame: Decal,
    alpha_images: Vec<u32>,
}

impl MaterialImages {
    pub(crate) fn register_image(&mut self, name: &str, usage: ImageUse) -> u32 {
        if let Some(index) = self.names.iter().position(|(existing, existing_usage)| existing == name && *existing_usage == usage) {
            return index as u32;
        }
        let index = self.names.len() as u32;
        self.names.push((name.to_owned(), usage));
        index
    }

    /// A decal's frame and the images whose alpha it reads, sampled with the fill's `sampling`
    /// (P1: the decal path's Nearest and Bilinear HQ; today every decal image samples as
    /// Bilinear Sharp, `register_decal` in gpu_composite.rs rejects the others).
    pub(crate) fn register_decal<'a>(&mut self, frame: Decal, sources: impl Iterator<Item = &'a Source>, _sampling: Sampling) -> u32 {
        let alpha_images = sources
            .filter_map(|source| match source {
                Source::Image { name, .. } => Some(self.register_image(name, ImageUse::Material)),
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
        self.images = self.names.iter().map(|(name, _)| images.get(name).cloned()).collect();
    }

    /// The colour images for the atlas, each with the halved levels its reads need: the chain
    /// Bilinear HQ reads (`ImageRes::mip_chain`), linear for an sRGB image's colour read
    /// (`ImageRes::linear_mip_chain`).
    pub(crate) fn atlas_images(&self) -> impl Iterator<Item = (&Arc<ImageRes>, AtlasMips)> {
        self.names.iter().zip(&self.images).filter_map(|((_, usage), image)| {
            let image = image.as_ref()?;
            let mips = match usage {
                ImageUse::Material => AtlasMips::None,
                ImageUse::MipmappedColor if image.srgb => AtlasMips::LinearColor,
                ImageUse::Mipmapped | ImageUse::MipmappedColor => AtlasMips::Stored,
            };
            Some((image, mips))
        })
    }

    /// Exact-f32 planes for the atlas (P1: projector depth grids built in [`Self::set_surface`]).
    pub(crate) fn raw_planes(&self) -> &[RawPlane] {
        &[]
    }

    /// The mesh the program composites (P1: build the depth grids of depth-culled projections
    /// here, with `texturecraft_compose::projector_depth`). Called by
    /// `CompositeProgram::set_surface_inputs` before the atlas reads [`Self::raw_planes`].
    pub(crate) fn set_surface(&mut self, mesh: &Mesh) {
        self.mesh_frame = Some(mesh_frame(mesh.bounds()));
    }

    /// The composited mesh's centre and radius, once [`Self::set_surface`] has run.
    pub(crate) fn mesh_frame(&self) -> Option<(Vec3, f32)> {
        self.mesh_frame
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
                words.extend(normalization.map_or_else(|| unplaced_decal_words(alpha), |(center, radius)| decal_words(&decal.frame, alpha, center, radius)));
                offset
            })
            .collect();
        (image_offsets, decal_offsets)
    }
}

/// The word of a decal's alpha image (`composite_projection.wgsl` reads it for P1's frames)
const DECAL_ALPHA_WORD: usize = 11;

/// The words of a decal that projects nothing (the built-in decal path skips it), keeping its
/// alpha image for P1's frames
fn unplaced_decal_words(alpha: u32) -> [u32; 16] {
    let mut words = [0; 16];
    words[DECAL_ALPHA_WORD] = alpha;
    words
}

fn decal_words(decal: &Decal, alpha: u32, center: Vec3, radius: f32) -> [u32; 16] {
    let Some((origin, right, up, normal)) = decal_axes(decal) else {
        return unplaced_decal_words(alpha);
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
