//! Image fill bindings, world-space decal frames and projector depth grids for the GPU compositor.

use std::sync::Arc;

use texturecraft_compose::{
    ImageRes, ImageStore,
    pattern::{decal_axes, mesh_frame},
    placed::{
        rotation_xyz,
        warp::{self, WarpSurface},
    },
};
use texturecraft_core::{Decal, DecalFlip, Falloff, FillSymmetry, Filtering, ImagePlacement, PlacedProjection, Projection, Sampling, Source, Volume, WarpGrid};
use texturecraft_geom::{Mesh, glam::Vec3};

use crate::gpu_tile_pattern::{AtlasMips, ColorImage, RawPlane, TileImageAtlas};
use texturecraft_texel::TileImage;

#[path = "gpu_depth_grid.rs"]
mod depth_grid;
use depth_grid::{FillGrids, depth_grids, missing_entry};

pub(crate) const IMAGE_SOURCE_OPCODE: u32 = 14;
pub(crate) const DECAL_PROJECTION: u32 = 9;
// Placed projections of imported fills: after the UV tile selector (10), before P1's (16..32)
pub(crate) const PLACED_UV_PROJECTION: u32 = 11;
pub(crate) const PLACED_TRIPLANAR_PROJECTION: u32 = 12;
pub(crate) const PLACED_WARP_PROJECTION: u32 = 13;

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
    /// The image and either chain of its halved levels: an alpha read, whose levels agree in both.
    MipmappedAlpha,
}

#[derive(Debug, Default)]
pub(crate) struct MaterialImages {
    /// Each registered image with what it is read for, or an imported graph's cooked output
    sources: Vec<MaterialSource>,
    /// The decoded image of each named source (`set_images`)
    images: Vec<Option<Arc<ImageRes>>>,
    /// The atlas's colour images, each image and chain once (`set_images`), and each registered
    /// image's entry among them; the cooked graph outputs follow them in the atlas
    atlas: Vec<(Arc<ImageRes>, AtlasMips)>,
    entries: Vec<Option<usize>>,
    decals: Vec<DecalSource>,
    volumes: Vec<VolumeSource>,
    /// The composited mesh's centre and radius (`set_surface`): P1's band uniform turns positions
    /// in mesh radii back into world positions with them
    mesh_frame: Option<(Vec3, f32)>,
    /// Depth-culled fills: projection, symmetry, and the side-table word their depth directory's
    /// offset goes in
    depths: Vec<(Projection, Option<FillSymmetry>, u32)>,
    /// Each depth-culled fill's grids over the mesh (`set_surface`), with each copy's raw plane
    depth_grids: Vec<Option<(FillGrids, Vec<Option<usize>>)>>,
    raw_planes: Vec<RawPlane>,
}

#[derive(Debug)]
enum MaterialSource {
    Named(String, ImageUse),
    Graph { key: Arc<str>, name: String, image: TileImage, srgb: bool },
}

#[derive(Debug, PartialEq)]
struct VolumeSource {
    volume: Volume,
    image: ImagePlacement,
    settings: [f32; 3],
    warp: Option<Arc<WarpSurface>>,
}

#[derive(Debug, PartialEq)]
struct DecalSource {
    frame: Decal,
    alpha_images: Vec<u32>,
}

impl MaterialImages {
    pub(crate) fn register_image(&mut self, name: &str, usage: ImageUse) -> u32 {
        if let Some(index) = self
            .sources
            .iter()
            .position(|source| matches!(source, MaterialSource::Named(existing, existing_usage) if existing == name && *existing_usage == usage))
        {
            return index as u32;
        }

        let index = self.sources.len() as u32;
        self.sources.push(MaterialSource::Named(name.to_owned(), usage));
        index
    }

    pub(crate) fn register_graph(&mut self, key: &Arc<str>, name: &str, image: &TileImage, srgb: bool) -> u32 {
        if let Some(index) = self
            .sources
            .iter()
            .position(|source| matches!(source, MaterialSource::Graph { key: k, name: n, srgb: color, .. } if k == key && n == name && *color == srgb))
        {
            return index as u32;
        }

        let index = self.sources.len() as u32;
        self.sources.push(MaterialSource::Graph { key: key.clone(), name: name.to_owned(), image: image.clone(), srgb });
        index
    }

    /// A frame and the images whose alpha it reads, sampled with the fill's `sampling`: Bilinear
    /// HQ reads the alpha's halved levels.
    pub(crate) fn register_decal<'a>(&mut self, frame: Decal, sources: impl Iterator<Item = &'a Source>, sampling: Sampling) -> u32 {
        let usage = if sampling.filtering == Filtering::BilinearHq { ImageUse::MipmappedAlpha } else { ImageUse::Material };
        let alpha_images = sources
            .filter_map(|source| match source {
                Source::Image { name, .. } => Some(self.register_image(name, usage)),
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

    /// A depth-culled fill with `sampling`: its grids are built over the mesh in
    /// [`Self::set_surface`], and side-table word `directory_word` gets its depth directory's
    /// offset when the program uploads.
    pub(crate) fn register_depth(&mut self, projection: &Projection, sampling: Sampling, directory_word: u32) {
        self.depths.push((projection.clone(), sampling.symmetry.on.then_some(sampling.symmetry), directory_word));
    }

    pub(crate) fn register_triplanar(&mut self, placed: &PlacedProjection, hardness: f32) -> u32 {
        let edge = 0.5 + 0.5 * normalized_hardness(hardness);
        self.register_volume(placed, [edge, 0.0, 0.0], None)
    }

    pub(crate) fn register_warp(&mut self, placed: &PlacedProjection, grid: &WarpGrid, depth: Falloff) -> Option<u32> {
        let surface = if flat_regular_grid(grid) { None } else { Some(warp::surface_for(grid).ok()?) };
        let reach = if grid.depth.is_finite() { grid.depth.max(0.0) } else { 0.0 };
        Some(self.register_volume(placed, [reach, f32::from(u8::from(depth.on)), normalized_hardness(depth.hardness)], surface))
    }

    fn register_volume(&mut self, placed: &PlacedProjection, settings: [f32; 3], warp: Option<Arc<WarpSurface>>) -> u32 {
        let source = VolumeSource { volume: placed.volume, image: placed.image, settings, warp };
        if let Some(index) = self.volumes.iter().position(|existing| *existing == source) {
            return index as u32;
        }
        let index = self.volumes.len() as u32;
        self.volumes.push(source);
        index
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sources.is_empty() && self.decals.is_empty() && self.volumes.is_empty() && self.depths.is_empty()
    }

    pub(crate) fn has_curved_warp(&self) -> bool {
        self.volumes.iter().any(|volume| volume.warp.is_some())
    }

    /// Resolve the registered images and lay out the atlas's colour images: each image once per
    /// chain of halved levels its reads need (`ImageRes::mip_chain`, or `linear_mip_chain` for an
    /// sRGB colour read). An alpha read takes any chain the image already has, and a read of the
    /// image's own texels any entry at all.
    pub(crate) fn set_images(&mut self, images: &ImageStore) {
        self.images = self
            .sources
            .iter()
            .map(|source| match source {
                MaterialSource::Named(name, _) => images.get(name).cloned(),
                MaterialSource::Graph { .. } => None,
            })
            .collect();
        self.atlas.clear();
        self.entries = vec![None; self.sources.len()];

        // The chains first, then the reads an existing entry may serve
        for pass in 0..3 {
            for (slot, (source, image)) in self.sources.iter().zip(&self.images).enumerate() {
                let (MaterialSource::Named(_, usage), Some(image)) = (source, image) else { continue };
                let mips = match (pass, usage) {
                    (0, ImageUse::Mipmapped) => AtlasMips::Stored,
                    (0, ImageUse::MipmappedColor) => AtlasMips::LinearColor,
                    (1, ImageUse::MipmappedAlpha) => AtlasMips::Stored,
                    (2, ImageUse::Material) => AtlasMips::None,
                    _ => continue,
                };

                // An sRGB colour read needs the linear chain, which a stored image does not differ from
                let mips = if mips == AtlasMips::LinearColor && !image.srgb { AtlasMips::Stored } else { mips };
                let serves = |entry: &(Arc<ImageRes>, AtlasMips)| {
                    Arc::ptr_eq(&entry.0, image)
                        && match usage {
                            ImageUse::Material => true,
                            ImageUse::MipmappedAlpha => entry.1 != AtlasMips::None,
                            ImageUse::Mipmapped | ImageUse::MipmappedColor => entry.1 == mips,
                        }
                };
                let entry = self.atlas.iter().position(serves).unwrap_or_else(|| {
                    self.atlas.push((image.clone(), mips));
                    self.atlas.len() - 1
                });
                self.entries[slot] = Some(entry);
            }
        }
    }

    /// The colour images for the atlas with the halved levels each holds, then the cooked graph outputs
    pub(crate) fn atlas_images(&self) -> impl Iterator<Item = (ColorImage<'_>, AtlasMips)> {
        let authored = self.atlas.iter().map(|(image, mips)| (ColorImage::Authored(image), *mips));
        authored.chain(self.sources.iter().filter_map(|source| match source {
            MaterialSource::Graph { key, name, image, srgb } => Some((ColorImage::Cooked { key, name, image, srgb: *srgb }, AtlasMips::None)),
            MaterialSource::Named(..) => None,
        }))
    }

    /// Exact-f32 planes for the atlas: the projector depth grids built in [`Self::set_surface`].
    pub(crate) fn raw_planes(&self) -> &[RawPlane] {
        &self.raw_planes
    }

    /// The mesh the program composites: its frame, and the depth grids of the depth-culled
    /// fills, one raw plane per distinct grid. Called by `CompositeProgram::set_surface_inputs`
    /// before the atlas reads [`Self::raw_planes`].
    pub(crate) fn set_surface(&mut self, mesh: &Arc<Mesh>) {
        self.mesh_frame = Some(mesh_frame(mesh.bounds()));
        let requests: Vec<_> = self.depths.iter().map(|(projection, symmetry, _)| (projection.clone(), *symmetry)).collect();
        let mut planes: Vec<RawPlane> = Vec::new();
        let grids = depth_grids(mesh, &requests)
            .into_iter()
            .map(|grids| {
                let grids = grids?;
                let indices = grids
                    .iter()
                    .map(|grid| {
                        let grid = grid.as_ref()?;
                        let index = planes.iter().position(|plane| Arc::ptr_eq(&plane.texels, &grid.plane.texels)).unwrap_or_else(|| {
                            planes.push(grid.plane.clone());
                            planes.len() - 1
                        });
                        Some(index)
                    })
                    .collect();
                Some((grids, indices))
            })
            .collect();
        self.depth_grids = grids;
        self.raw_planes = planes;
    }

    /// The composited mesh's centre and radius, once [`Self::set_surface`] has run.
    pub(crate) fn mesh_frame(&self) -> Option<(Vec3, f32)> {
        self.mesh_frame
    }

    pub(crate) fn append_words(&self, words: &mut Vec<u32>, atlas: Option<&TileImageAtlas>, mesh: Option<&Mesh>) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
        let entry_offsets: Vec<u32> = atlas
            .map_or_else(Vec::new, |atlas| self.atlas_images().enumerate().map(|(index, (image, _))| atlas.append_rgba(words, index, image.srgb())).collect());
        let mut graphs = self.atlas.len();
        let image_offsets: Vec<_> = self
            .sources
            .iter()
            .enumerate()
            .map(|(slot, source)| {
                let entry = match source {
                    MaterialSource::Named(..) => self.entries.get(slot).copied().flatten(),
                    MaterialSource::Graph { .. } => {
                        graphs += 1;
                        Some(graphs - 1)
                    }
                };
                entry.and_then(|entry| entry_offsets.get(entry).copied()).unwrap_or(u32::MAX)
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
        let volume_offsets = self
            .volumes
            .iter()
            .map(|source| {
                let offset = words.len() as u32;
                let start = words.len();
                words.extend(mesh.map_or([0; 24], |mesh| volume_words(source, mesh)));
                if let Some(warp) = &source.warp {
                    let data = words.len() as u32;
                    words[start + 22] = data;
                    append_warp(words, warp);
                }
                offset
            })
            .collect();
        self.append_depth_directories(words, atlas);
        (image_offsets, decal_offsets, volume_offsets)
    }

    /// Each depth-culled fill's directory (its copy count, then one entry per copy) after the
    /// grids' plane descriptors, its offset written into the fill's side-table word.
    fn append_depth_directories(&self, words: &mut Vec<u32>, atlas: Option<&TileImageAtlas>) {
        let plane_offsets: Vec<u32> = atlas.map_or_else(Vec::new, |atlas| (0..self.raw_planes.len()).map(|index| atlas.append_raw(words, index)).collect());
        for ((_, _, directory_word), grids) in self.depths.iter().zip(&self.depth_grids) {
            let Some((grids, indices)) = grids else { continue };

            let directory = words.len() as u32;
            words.push(grids.len() as u32);
            for (grid, index) in grids.iter().zip(indices) {
                let plane = index.and_then(|index| plane_offsets.get(index).copied());
                words.extend(grid.as_ref().zip(plane).map_or_else(missing_entry, |(grid, plane)| grid.entry(plane)));
            }
            if let Some(word) = words.get_mut(*directory_word as usize) {
                *word = directory;
            }
        }
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

/// Geometry shared by the inverse queries for every texel of a warp.
fn append_warp(words: &mut Vec<u32>, surface: &WarpSurface) {
    let grid = surface.inverse_grid();
    words.extend([grid.rows as u32, grid.columns as u32, grid.size[0] as u32, grid.size[1] as u32]);
    words.extend([grid.origin[0], grid.origin[1], grid.cell_size].map(|v| (v as f32).to_bits()));
    words.extend(grid.controls.iter().flatten().map(|&v| (v as f32).to_bits()));
    words.extend(grid.image_uvs.iter().flatten().map(|&v| (v as f32).to_bits()));
    for seed in grid.seeds() {
        words.extend(seed.unwrap_or([-1.0; 2]).map(f32::to_bits));
    }
}

fn normalized_hardness(hardness: f32) -> f32 {
    if hardness.is_finite() { hardness.clamp(0.0, 1.0) } else { 0.5 }
}

/// This grid's Bézier surface is the plane z=0 with affine image coordinates. Imported
/// grids can start at either end of either axis and can store their regular handles explicitly.
fn flat_regular_grid(grid: &WarpGrid) -> bool {
    let maximum = texturecraft_compose::placed::warp::MAX_SIDE;
    if !(2..=maximum).contains(&grid.rows) || !(2..=maximum).contains(&grid.columns) {
        return false;
    }
    let rows = grid.rows as usize;
    let columns = grid.columns as usize;
    let Some(first) = grid.points.first() else { return true };
    if grid.points.len() != rows * columns || first.pos[0].abs() != 1.0 || first.pos[1].abs() != 1.0 {
        return false;
    }
    let [left, top, _] = first.pos;
    let dx = -2.0 * left / (columns - 1) as f32;
    let dy = -2.0 * top / (rows - 1) as f32;
    let close = |actual: f32, expected: f32| (actual - expected).abs() <= 4.0 * f32::EPSILON;
    grid.points.iter().enumerate().all(|(index, point)| {
        let x = left + dx * (index % columns) as f32;
        let y = top + dy * (index / columns) as f32;
        let handles = [[-dx / 3.0, 0.0, 0.0], [dx / 3.0, 0.0, 0.0], [0.0, -dy / 3.0, 0.0], [0.0, dy / 3.0, 0.0]];
        [x, y, 0.0].into_iter().zip(point.pos).all(|(expected, actual)| close(actual, expected))
            && [0.5 + 0.5 * x, 0.5 - 0.5 * y].into_iter().zip(point.uv).all(|(expected, actual)| close(actual, expected))
            && handles
                .into_iter()
                .zip(point.tangents)
                .all(|(expected, actual)| actual == [0.0; 3] || expected.into_iter().zip(actual).all(|(e, a)| close(a, e)))
    })
}

fn volume_words(source: &VolumeSource, mesh: &Mesh) -> [u32; 24] {
    let (lo, hi) = mesh.bounds();
    let extent = hi - lo;
    let radius = (extent.length() * 0.5).max(1e-4);
    let unit = extent.max_element().max(1e-6);
    let world_to_local = rotation_xyz(source.volume.rotation);
    let inverse_size = source.volume.size.map(|size| {
        let size = if size.is_finite() { size } else { 1.0 };
        1.0 / size.abs().max(1e-6).copysign(size)
    });
    let offset = Vec3::from(source.volume.offset.map(|v| if v.is_finite() { v } else { 0.0 }));
    let frame = [world_to_local.x_axis, world_to_local.y_axis, world_to_local.z_axis];
    let rows: [[f32; 4]; 3] = std::array::from_fn(|i| {
        let axis = frame[i];
        let basis = axis * (inverse_size[i] * (radius / unit));
        [basis.x, basis.y, basis.z, -axis.dot(offset) * inverse_size[i]]
    });
    let angle = if source.image.rotation.is_finite() { source.image.rotation } else { 0.0 };
    let (sin, cos) = angle.to_radians().sin_cos();
    let [u, v] = source.image.tiling.map(|s| if s.is_finite() { s } else { 1.0 });
    let [x, y] = source.image.offset.map(|s| if s.is_finite() { s } else { 0.0 });
    let [setting, falloff, hardness] = source.settings;
    let [a, b, c] = rows;
    [
        a[0],
        a[1],
        a[2],
        a[3],
        b[0],
        b[1],
        b[2],
        b[3],
        c[0],
        c[1],
        c[2],
        c[3],
        cos * u,
        sin * u,
        -sin * v,
        cos * v,
        x,
        y,
        setting,
        f32::from(u8::from(source.volume.clip)),
        falloff,
        hardness,
        0.0,
        0.0,
    ]
    .map(f32::to_bits)
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
