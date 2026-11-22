//! Image fill bindings and world-space decal frames for the GPU compositor.

use std::sync::Arc;

use texturecraft_compose::{
    ImageRes, ImageStore,
    pattern::decal_axes,
    placed::{
        rotation_xyz,
        warp::{self, WarpSurface},
    },
};
use texturecraft_core::{Decal, DecalFlip, Falloff, ImagePlacement, PlacedProjection, Source, Volume, WarpGrid};
use texturecraft_geom::{Mesh, glam::Vec3};

use crate::gpu_tile_pattern::{ColorImage, TileImageAtlas};
use texturecraft_texel::TileImage;

pub(crate) const IMAGE_SOURCE_OPCODE: u32 = 14;
pub(crate) const DECAL_PROJECTION: u32 = 9;
pub(crate) const PLACED_UV_PROJECTION: u32 = 10;
pub(crate) const PLACED_TRIPLANAR_PROJECTION: u32 = 11;
pub(crate) const PLACED_WARP_PROJECTION: u32 = 12;

#[derive(Debug, Default)]
pub(crate) struct MaterialImages {
    sources: Vec<MaterialSource>,
    decals: Vec<DecalSource>,
    volumes: Vec<VolumeSource>,
}

#[derive(Debug)]
enum MaterialSource {
    Named(String, Option<Arc<ImageRes>>),
    Graph { key: Arc<str>, name: String, image: TileImage, srgb: bool },
}

impl MaterialSource {
    fn image(&self) -> Option<ColorImage<'_>> {
        match self {
            Self::Named(_, image) => image.as_ref().map(ColorImage::Authored),
            Self::Graph { key, name, image, srgb } => Some(ColorImage::Cooked { key, name, image, srgb: *srgb }),
        }
    }
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
    pub(crate) fn register_image(&mut self, name: &str) -> u32 {
        if let Some(index) = self.sources.iter().position(|source| matches!(source, MaterialSource::Named(existing, _) if existing == name)) {
            return index as u32;
        }
        let index = self.sources.len() as u32;
        self.sources.push(MaterialSource::Named(name.to_owned(), None));
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
        self.sources.is_empty() && self.decals.is_empty() && self.volumes.is_empty()
    }

    pub(crate) fn has_curved_warp(&self) -> bool {
        self.volumes.iter().any(|volume| volume.warp.is_some())
    }

    pub(crate) fn set_images(&mut self, images: &ImageStore) {
        for source in &mut self.sources {
            if let MaterialSource::Named(name, decoded) = source {
                *decoded = images.get(name).cloned();
            }
        }
    }

    pub(crate) fn sources(&self) -> impl Iterator<Item = ColorImage<'_>> {
        self.sources.iter().filter_map(MaterialSource::image)
    }

    pub(crate) fn append_words(&self, words: &mut Vec<u32>, atlas: Option<&TileImageAtlas>, mesh: Option<&Mesh>) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
        let mut index = 0;
        let image_offsets: Vec<_> = self
            .sources
            .iter()
            .map(|source| {
                if let Some((image, atlas)) = source.image().zip(atlas) {
                    let offset = atlas.append_rgba(words, index, image.srgb());
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
        (image_offsets, decal_offsets, volume_offsets)
    }
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
