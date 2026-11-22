//! Settings and authored images for the GPU Tile Generator. Placement and random draws
//! are constant per cell. The shader evaluates shapes and blends at each material texel.

use crate::gpu_composite::{CompositeTiming, time_composite};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use texturecraft_compose::{ImageRes, ImageStore, TileLayout, prepare_tile_layout};
use texturecraft_core::{TileBlending, TileGenerator, TileShape};
use texturecraft_texel::{TILE, TileImage};

pub(crate) const TILE_GENERATOR_OPCODE: u32 = 80;
const NO_IMAGE: u32 = u32::MAX;
// Keep uploads banded even for an image that fills an entire atlas slice.
const IMAGE_ROWS: usize = 64;
// Put a bound on packing overhead as well as authored pixels.
const MAX_ATLAS_BYTES: u64 = 256 << 20;
const MAX_COLOR_ATLAS_BYTES: u64 = 1 << 30;
const MAX_PENDING_UPLOAD_BYTES: u64 = 64 << 20;

#[derive(Debug, Default)]
pub(crate) struct TilePatterns {
    patterns: Vec<TilePattern>,
    images: BTreeMap<String, Arc<ImageRes>>,
}

#[derive(Debug)]
struct TilePattern {
    settings: TileGenerator,
    seed: u32,
    layout: TileLayout,
    work_per_sample: u64,
}

impl TilePatterns {
    pub(crate) fn register(&mut self, settings: TileGenerator, seed: u32) -> usize {
        if let Some(index) = self.patterns.iter().position(|p| p.seed == seed && p.settings == settings) {
            return index;
        }
        let layout = prepare_tile_layout(&settings, seed);
        let work_per_sample = placement_work(&layout);
        let index = self.patterns.len();
        self.patterns.push(TilePattern { settings, seed, layout, work_per_sample });
        index
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub(crate) fn work_per_sample(&self, index: usize) -> Option<u64> {
        self.patterns.get(index).map(|pattern| pattern.work_per_sample)
    }

    pub(crate) fn set_images(&mut self, images: &ImageStore) {
        self.images = self
            .patterns
            .iter()
            .flat_map(|pattern| {
                let settings = &pattern.settings;
                let count = if settings.shape == TileShape::Image { settings.inputs.len().min(TileGenerator::MAX_INPUTS) } else { 0 };
                settings.inputs[..count].iter().chain(settings.background_image.iter())
            })
            .filter_map(|name| images.get(name).map(|image| (name.clone(), image.clone())))
            .collect();
    }

    pub(crate) fn image_sources(&self) -> impl Iterator<Item = &Arc<ImageRes>> {
        self.images.values()
    }

    /// Append image descriptors and prepared shapes to the compositor's packed parameter buffer.
    /// All references are word offsets from its beginning, following the sRGB lookup.
    pub(crate) fn append_words(&self, words: &mut Vec<u32>, atlas: Option<&TileImageAtlas>) -> Vec<u32> {
        let mut images = BTreeMap::new();
        if let Some(atlas) = atlas {
            for (name, region) in self.images.keys().zip(&atlas.regions) {
                images.insert(name, words.len() as u32);
                words.extend([region.origin[0], region.origin[1], region.origin[2], region.size[0], region.size[1]]);
            }
        }
        let image = |name: &String| images.get(name).copied().unwrap_or(NO_IMAGE);
        self.patterns
            .iter()
            .map(|pattern| {
                let settings = &pattern.settings;
                let layout = &pattern.layout;
                let shift_offset = words.len() as u32;
                words.extend(layout.line_shifts.iter().map(|x| x.to_bits()));
                let placement_offset = words.len() as u32;
                for placement in &layout.placements {
                    let Some(tile) = placement else {
                        words.extend([0; 11]);
                        continue;
                    };
                    let input = tile.input.and_then(|index| settings.inputs.get(index)).map_or(NO_IMAGE, image);
                    let flags = tile.quarter_turns | (u32::from(tile.mirrors[0]) << 2) | (u32::from(tile.mirrors[1]) << 3);
                    words.extend([
                        tile.center.x.to_bits(),
                        tile.center.y.to_bits(),
                        tile.size.x.to_bits(),
                        tile.size.y.to_bits(),
                        tile.radius.x.to_bits(),
                        tile.radius.y.to_bits(),
                        tile.rotation[0].to_bits(),
                        tile.rotation[1].to_bits(),
                        flags,
                        input,
                        tile.luminance.to_bits(),
                    ]);
                }

                let angle = settings.shape_angle / 360.0 * std::f32::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                let low = (1.0 - 2.0 * settings.position).clamp(0.0, 1.0);
                let high = (2.0 - 2.0 * settings.position).clamp(0.0, 1.0);
                let middle = (low + high) * 0.5;
                let half = (high - low) * 0.5 * (1.0 - settings.contrast);
                let shape = TileShape::ALL.iter().position(|&shape| shape == settings.shape).unwrap_or_default() as u32;
                let blending = match settings.blending {
                    TileBlending::Max => 0,
                    TileBlending::Add => 1,
                    TileBlending::AddSub => 2,
                };
                let offset = words.len() as u32;
                words.extend([
                    layout.amount[0],
                    layout.amount[1],
                    layout.reach.x.to_bits(),
                    layout.reach.y.to_bits(),
                    u32::from(settings.vertical_offset),
                    shift_offset,
                    placement_offset,
                    shape,
                    settings.shape_resolution[0],
                    settings.shape_resolution[1],
                    settings.specific.to_bits(),
                    settings.shape_tiling,
                    (settings.shape_size[0] * settings.shape_scale).to_bits(),
                    (settings.shape_size[1] * settings.shape_scale).to_bits(),
                    sin.to_bits(),
                    cos.to_bits(),
                    u32::from(settings.shape_diagonal),
                    settings.background.to_bits(),
                    settings.background_image.as_ref().map_or(NO_IMAGE, image),
                    settings.color.to_bits(),
                    blending,
                    settings.opacity.to_bits(),
                    middle.to_bits(),
                    half.to_bits(),
                    settings.global_offset[0].to_bits(),
                    settings.global_offset[1].to_bits(),
                ]);
                offset
            })
            .collect()
    }
}

/// Upper bound on cell visits and periodic copies per sample in tile_generator.wgsl.
fn placement_work(layout: &TileLayout) -> u64 {
    let cells = |reach: f32, count: u32| ((reach * 2.0).ceil() + 2.0).min(count as f32) as u64;
    // The shader checks offsets −3 through 3 on each axis for a wrapped shape.
    let copies = |radius: f32| ((radius * 2.0).floor() + 1.0).clamp(1.0, 7.0) as u64;
    let wrapped_copies = layout.placements.iter().flatten().map(|tile| copies(tile.radius.x) * copies(tile.radius.y)).max().unwrap_or(1);
    cells(layout.reach.x, layout.amount[0]) * cells(layout.reach.y, layout.amount[1]) * wrapped_copies
}

#[derive(Clone, Copy, Default)]
struct ImageRegion {
    origin: [u32; 3],
    size: [u32; 2],
}

#[derive(Clone, Copy)]
enum RgbaEncoding {
    Bytes = 1,
    Shorts = 2,
    Floats = 4,
    Gray16 = 5,
}

impl RgbaEncoding {
    fn for_image(image: &ImageRes) -> Self {
        let exact = |v: f32, scale: f32| {
            let integer = (v * scale).round();
            integer >= 0.0 && integer <= scale && (v == integer / scale || v == integer * (1.0 / scale))
        };
        if image.rgba.par_iter().all(|pixel| pixel.iter().all(|&v| exact(v, 255.0))) {
            Self::Bytes
        } else if image.rgba.par_iter().all(|pixel| pixel.iter().all(|&v| exact(v, 65535.0))) {
            Self::Shorts
        } else {
            Self::Floats
        }
    }

    fn planes(self) -> usize {
        if matches!(self, Self::Gray16) { 1 } else { self as usize }
    }

    fn word(self, [r, g, b, a]: [f32; 4], lane: usize) -> u32 {
        match self {
            Self::Bytes => {
                let byte = |v: f32| (v * 255.0).round() as u8;
                u32::from_le_bytes([byte(r), byte(g), byte(b), byte(a)])
            }
            Self::Shorts => {
                let short = |v: f32| (v * 65535.0).round() as u32;
                let [low, high] = if lane == 0 { [r, g] } else { [b, a] };
                short(low) | (short(high) << 16)
            }
            Self::Floats => [r, g, b, a][lane].to_bits(),
            Self::Gray16 => (r * 65535.0).round() as u32,
        }
    }
}

struct RgbaRegion {
    encoding: RgbaEncoding,
    planes: Vec<ImageRegion>,
}

#[derive(Clone, Copy)]
pub(crate) enum ColorImage<'a> {
    Authored(&'a Arc<ImageRes>),
    Cooked { key: &'a Arc<str>, name: &'a str, image: &'a TileImage, srgb: bool },
}

impl ColorImage<'_> {
    fn size(self) -> [u32; 2] {
        match self {
            Self::Authored(image) => [image.width, image.height],
            Self::Cooked { image, .. } => [image.width(), image.height()],
        }
        .map(|side| side.max(1))
    }

    pub(crate) fn srgb(self) -> bool {
        match self {
            Self::Authored(image) => image.srgb,
            Self::Cooked { srgb, .. } => srgb,
        }
    }

    fn encoding(self) -> RgbaEncoding {
        match self {
            Self::Authored(image) => RgbaEncoding::for_image(image),
            Self::Cooked { image, .. } if image.comps() == 1 => RgbaEncoding::Gray16,
            Self::Cooked { .. } => RgbaEncoding::Shorts,
        }
    }

    fn pack_rows(self, encoding: RgbaEncoding, lane: usize, top: u32, rows: u32, width: u32, words: &mut Vec<u32>) {
        let width = width as usize;
        match self {
            Self::Authored(image) => {
                let first = top as usize * width;
                let end = first + rows as usize * width;
                words.extend((first..end).map(|index| encoding.word(image.rgba.get(index).copied().unwrap_or_default(), lane)));
            }
            Self::Cooked { image, .. } => {
                let side = TILE as usize;
                let components = image.comps();
                words.resize(rows as usize * width, 0);
                // A graph has 64×64 stored tiles. Read each tile once per band and
                // write directly to the upload rows, without expanding the image to floats.
                for (tx, left) in (0..width).step_by(side).enumerate() {
                    for (ty, output_top) in (top..top + rows).step_by(side).enumerate() {
                        let Some(tile) = image.tile(tx as u32, top / TILE + ty as u32) else { continue };
                        let row_count = (top + rows - output_top).min(TILE) as usize;
                        let row_width = side.min(width - left);
                        for r in 0..row_count {
                            let local = ((output_top % TILE) as usize + r) * side * components;
                            let at = ((output_top - top) as usize + r) * width + left;
                            let Some(source) = tile.get(local..local + row_width * components) else { continue };
                            let destination = &mut words[at..at + row_width];
                            if components == 1 {
                                destination.iter_mut().zip(source).for_each(|(dst, val)| *dst = u32::from(*val));
                            } else {
                                let offset = lane * 2;
                                destination.iter_mut().zip(source.chunks_exact(components)).for_each(|(dst, texel)| {
                                    let low = u32::from(texel.get(offset).copied().unwrap_or(0));
                                    let high = u32::from(texel.get(offset + 1).copied().unwrap_or(0));
                                    *dst = low | (high << 16);
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

enum AtlasIdentity {
    Authored(Weak<ImageRes>),
    Cooked(Arc<str>, String),
}

impl AtlasIdentity {
    fn of(image: ColorImage<'_>) -> Self {
        match image {
            ColorImage::Authored(image) => Self::Authored(Arc::downgrade(image)),
            ColorImage::Cooked { key, name, .. } => Self::Cooked(key.clone(), name.to_owned()),
        }
    }

    fn matches(&self, image: ColorImage<'_>) -> bool {
        match (self, image) {
            (Self::Authored(old), ColorImage::Authored(new)) => old.ptr_eq(&Arc::downgrade(new)),
            (Self::Cooked(k, n), ColorImage::Cooked { key, name, .. }) => k == key && n == name,
            _ => false,
        }
    }

    fn is_live(&self) -> bool {
        match self {
            Self::Authored(image) => image.strong_count() != 0,
            // A cooked image can be evicted from host memory while its atlas is still useful.
            Self::Cooked(..) => true,
        }
    }
}

pub(crate) struct TileImageAtlas {
    images: Vec<AtlasIdentity>,
    regions: Vec<ImageRegion>,
    color: Vec<RgbaRegion>,
    pub(crate) view: wgpu::TextureView,
}

impl TileImageAtlas {
    pub(crate) fn matches(&self, gray: &[&Arc<ImageRes>], color: &[ColorImage<'_>]) -> bool {
        self.regions.len() == gray.len()
            && self.color.len() == color.len()
            && self.images.iter().zip(gray.iter().map(|&image| ColorImage::Authored(image)).chain(color.iter().copied())).all(|(old, new)| old.matches(new))
    }

    pub(crate) fn is_live(&self) -> bool {
        self.images.iter().all(AtlasIdentity::is_live)
    }

    pub(crate) fn empty_view(device: &wgpu::Device) -> wgpu::TextureView {
        atlas_texture(device, 1, 1, 2).create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
    }

    pub(crate) fn append_rgba(&self, words: &mut Vec<u32>, color_index: usize, srgb: bool) -> u32 {
        let region = &self.color[color_index];
        let offset = words.len() as u32;
        let [width, height] = region.planes[0].size;
        let flags = region.encoding as u32 | (u32::from(srgb) << 8);
        words.extend([width, height, flags]);
        for plane in &region.planes {
            words.extend(plane.origin);
        }
        offset
    }

    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gray: &[&Arc<ImageRes>],
        color: &[ColorImage<'_>],
        mut timing: Option<&mut CompositeTiming>,
    ) -> Result<Self, String> {
        let limits = device.limits();
        let encodings: Vec<_> = color.iter().copied().map(ColorImage::encoding).collect();
        let sizes: Vec<_> = gray
            .iter()
            .map(|&image| ColorImage::Authored(image).size())
            .chain(color.iter().zip(&encodings).flat_map(|(&image, &encoding)| std::iter::repeat_n(image.size(), encoding.planes())))
            .collect();
        let budget = if color.is_empty() { MAX_ATLAS_BYTES } else { MAX_COLOR_ATLAS_BYTES };
        let (extent, regions) =
            arrange_images(&sizes, limits.max_texture_dimension_2d, limits.max_texture_array_layers, device.adapter_info().backend, budget)?;
        let texture = atlas_texture(device, extent[0], extent[1], extent[2]);
        let mut pixels = Vec::new();
        let mut pending = 0;
        type PackRows<'a> = &'a mut dyn FnMut(u32, u32, u32, &mut Vec<u32>);
        let mut upload = |region: &ImageRegion, pack: PackRows<'_>| -> Result<(), String> {
            let [width, height] = region.size;
            for top in (0..height).step_by(IMAGE_ROWS) {
                let rows = (height - top).min(IMAGE_ROWS as u32);
                pixels.clear();
                time_composite(timing.as_deref_mut().map(|t| &mut t.packing), || pack(top, rows, width, &mut pixels));
                time_composite(timing.as_deref_mut().map(|t| &mut t.upload), || {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            origin: wgpu::Origin3d { x: region.origin[0], y: region.origin[1] + top, z: region.origin[2] },
                            ..texture.as_image_copy()
                        },
                        bytemuck::cast_slice(&pixels),
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: None },
                        wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
                    );
                });
                pending += u64::from(width) * u64::from(rows) * 4;
                if pending >= MAX_PENDING_UPLOAD_BYTES {
                    time_composite(timing.as_deref_mut().map(|t| &mut t.upload_wait), || {
                        let submission = queue.submit([]);
                        device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|error| error.to_string())
                    })?;
                    pending = 0;
                }
            }
            Ok(())
        };
        let (gray_regions, mut color_regions) = regions.split_at(gray.len());
        for (image, region) in gray.iter().zip(gray_regions) {
            upload(region, &mut |top, rows, width, words| {
                let first = top as usize * width as usize;
                let end = first + rows as usize * width as usize;
                words.extend((first..end).map(|index| {
                    let [r, g, b, _] = image.rgba.get(index).copied().unwrap_or_default();
                    ((r + g + b) / 3.0).to_bits()
                }));
            })?;
        }
        let mut color_layout = Vec::new();
        for (&image, encoding) in color.iter().zip(encodings) {
            let (planes, remaining) = color_regions.split_at(encoding.planes());
            color_regions = remaining;
            for (lane, plane) in planes.iter().enumerate() {
                upload(plane, &mut |top, rows, width, words| image.pack_rows(encoding, lane, top, rows, width, words))?;
            }
            color_layout.push(RgbaRegion { encoding, planes: planes.to_vec() });
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Ok(Self {
            images: gray.iter().map(|&image| ColorImage::Authored(image)).chain(color.iter().copied()).map(AtlasIdentity::of).collect(),
            regions: gray_regions.to_vec(),
            color: color_layout,
            view,
        })
    }
}

fn arrange_images(images: &[[u32; 2]], side_limit: u32, layer_limit: u32, backend: wgpu::Backend, budget: u64) -> Result<([u32; 3], Vec<ImageRegion>), String> {
    let mut order: Vec<_> = images.iter().enumerate().map(|(i, &[w, h])| (i, w, h)).collect();
    order.sort_by_key(|&(_, _, height)| std::cmp::Reverse(height));
    let widest = order.iter().map(|&(_, width, _)| width).max().unwrap_or(1);
    let tallest = order.first().map_or(1, |&(_, _, height)| height);
    if widest.max(tallest) > side_limit {
        return Err("An authored image exceeds the GPU texture dimension limit.".into());
    }
    let area: u64 = order.iter().map(|&(_, width, height)| u64::from(width) * u64::from(height)).sum();
    let width = (area as f64).sqrt().ceil().min(f64::from(side_limit)) as u32;
    let width = width.max(widest);
    let (mut x, mut y, mut layer) = (0, 0, 0);
    let (mut used_w, mut used_h, mut row_height) = (1, 1, 0);
    let mut regions = vec![ImageRegion::default(); images.len()];
    for (index, w, h) in order {
        if x + w > width {
            x = 0;
            y += row_height;
            row_height = 0;
        }
        if y + h > side_limit {
            x = 0;
            y = 0;
            layer += 1;
            row_height = 0;
        }
        if layer >= layer_limit {
            return Err("The authored image atlas exceeds the GPU array limit.".into());
        }
        regions[index] = ImageRegion { origin: [x, y, layer], size: [w, h] };
        x += w;
        row_height = row_height.max(h);
        used_w = used_w.max(x);
        used_h = used_h.max(y + row_height);
    }
    let mut layers = layer + 1;
    // The GL backend derives the texture target from its extent. A view alone
    // cannot turn a single-layer texture or a cube into a 2D array.
    if backend == wgpu::Backend::Gl && (layers == 1 || (used_w == used_h && layers.is_multiple_of(6))) {
        layers += 1;
        if layers > layer_limit {
            return Err("The authored image atlas exceeds the GPU array limit.".into());
        }
    }
    let extent = [used_w, used_h, layers];
    let bytes = extent.into_iter().map(u64::from).product::<u64>() * size_of::<u32>() as u64;
    if bytes > budget {
        return Err("The authored image atlas exceeds the GPU memory budget.".into());
    }
    Ok((extent, regions))
}

fn atlas_texture(device: &wgpu::Device, width: u32, height: u32, layers: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tc_tile_images"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_atlas_counts_unoccupied_space_in_every_array_slice() {
        let large = [8192; 2];
        let small = [16; 2];
        let (extent, _) = arrange_images(&[large], 8192, 16, wgpu::Backend::Vulkan, MAX_ATLAS_BYTES).unwrap();
        assert_eq!(extent, [8192, 8192, 1]);
        let error = arrange_images(&[large, small], 8192, 16, wgpu::Backend::Vulkan, MAX_ATLAS_BYTES).err();
        assert!(error.is_some_and(|error| error.contains("memory budget")));
    }

    #[test]
    fn gl_image_atlas_uses_a_2d_array_target() {
        let image = [4; 2];
        let (extent, _) = arrange_images(&[image], 4, 16, wgpu::Backend::Gl, MAX_ATLAS_BYTES).unwrap();
        assert_eq!(extent, [4, 4, 2]);
        let (extent, _) = arrange_images(&[image; 6], 4, 16, wgpu::Backend::Gl, MAX_ATLAS_BYTES).unwrap();
        assert_eq!(extent, [4, 4, 7]);
    }
}
