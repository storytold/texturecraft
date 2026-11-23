//! Settings and authored images for the GPU Tile Generator. Placement and random draws
//! are constant per cell. The shader evaluates shapes and blends at each material texel.

use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use texturecraft_compose::{ImageRes, ImageStore, TileLayout, prepare_tile_layout};
use texturecraft_core::{TileBlending, TileGenerator, TileShape};

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
}

impl RgbaEncoding {
    fn for_texels(rgba: &[[f32; 4]]) -> Self {
        let exact = |v: f32, scale: f32| {
            let integer = (v * scale).round();
            integer >= 0.0 && integer <= scale && (v == integer / scale || v == integer * (1.0 / scale))
        };
        if rgba.par_iter().all(|pixel| pixel.iter().all(|&v| exact(v, 255.0))) {
            Self::Bytes
        } else if rgba.par_iter().all(|pixel| pixel.iter().all(|&v| exact(v, 65535.0))) {
            Self::Shorts
        } else {
            Self::Floats
        }
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
        }
    }
}

struct RgbaRegion {
    encoding: RgbaEncoding,
    planes: Vec<ImageRegion>,
    /// The halved levels held besides the image, level 1 first: size, encoding and planes each
    levels: Vec<RgbaLevel>,
}

struct RgbaLevel {
    size: [u32; 2],
    encoding: RgbaEncoding,
    planes: Vec<ImageRegion>,
}

/// Which halved levels of a colour image the atlas holds besides the image. P1 (fill sampling)
/// chooses per image through `MaterialImages::atlas_images`; `None` uploads exactly what the
/// atlas held before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AtlasMips {
    #[default]
    None,
    /// `ImageRes::mip_chain`: the levels as stored (Bilinear HQ of scalar and alpha reads).
    Stored,
    /// `ImageRes::linear_mip_chain`: RGB linear, alpha unchanged (an sRGB image's colour read).
    LinearColor,
}

/// One exact-f32 plane of the atlas (P1: a projector depth grid), row by row. The atlas keeps a
/// weak pointer to `texels`, so a plane rebuilt for every program rebuilds the atlas.
#[allow(dead_code)] // Built by P1's depth culling
#[derive(Clone, Debug)]
pub(crate) struct RawPlane {
    pub(crate) size: [u32; 2],
    pub(crate) texels: Arc<Vec<f32>>,
}

pub(crate) struct TileImageAtlas {
    images: Vec<Weak<ImageRes>>,
    regions: Vec<ImageRegion>,
    color: Vec<RgbaRegion>,
    color_mips: Vec<AtlasMips>,
    raw: Vec<(Weak<Vec<f32>>, ImageRegion)>,
    pub(crate) view: wgpu::TextureView,
}

impl TileImageAtlas {
    pub(crate) fn matches(&self, gray: &[&Arc<ImageRes>], color: &[(&Arc<ImageRes>, AtlasMips)], raw: &[RawPlane]) -> bool {
        self.regions.len() == gray.len()
            && self.color.len() == color.len()
            && self.images.iter().zip(gray.iter().copied().chain(color.iter().map(|(image, _)| *image))).all(|(old, new)| old.ptr_eq(&Arc::downgrade(new)))
            && self.color_mips.iter().eq(color.iter().map(|(_, mips)| mips))
            && self.raw.len() == raw.len()
            && self.raw.iter().zip(raw).all(|((old, region), new)| old.ptr_eq(&Arc::downgrade(&new.texels)) && region.size == new.size)
    }

    pub(crate) fn is_live(&self) -> bool {
        self.images.iter().all(|image| image.strong_count() != 0) && self.raw.iter().all(|(texels, _)| texels.strong_count() != 0)
    }

    pub(crate) fn empty_view(device: &wgpu::Device) -> wgpu::TextureView {
        atlas_texture(device, 1, 1, 2).create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
    }

    pub(crate) fn append_rgba(&self, words: &mut Vec<u32>, color_index: usize, srgb: bool) -> u32 {
        let region = &self.color[color_index];
        let offset = words.len() as u32;
        let [width, height] = region.planes[0].size;
        // Bit 9: halved levels follow the planes (count, then per level width, height,
        // encoding and plane origins). Without levels the words are exactly as before.
        let flags = region.encoding as u32 | (u32::from(srgb) << 8) | (u32::from(!region.levels.is_empty()) << 9);
        words.extend([width, height, flags]);
        for plane in &region.planes {
            words.extend(plane.origin);
        }
        if !region.levels.is_empty() {
            words.push(region.levels.len() as u32);
            for level in &region.levels {
                words.extend([level.size[0], level.size[1], level.encoding as u32]);
                for plane in &level.planes {
                    words.extend(plane.origin);
                }
            }
        }
        offset
    }

    /// The words of raw plane `index`: width, height and its origin (x, y, layer), read as f32 bits.
    #[allow(dead_code)] // Read by P1's depth culling
    pub(crate) fn append_raw(&self, words: &mut Vec<u32>, index: usize) -> u32 {
        let offset = words.len() as u32;
        let (_, region) = &self.raw[index];
        words.extend([region.size[0], region.size[1]]);
        words.extend(region.origin);
        offset
    }

    /// Upload `gray` (one plane each), `color` (one to four planes each by encoding, plus the
    /// chosen halved levels) and `raw` (one exact-f32 plane each) into one array texture.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gray: &[&Arc<ImageRes>],
        color: &[(&Arc<ImageRes>, AtlasMips)],
        raw: &[RawPlane],
    ) -> Result<Self, String> {
        let limits = device.limits();
        let size = |image: &Arc<ImageRes>| [image.width.max(1), image.height.max(1)];
        // Per colour image: the image, then each level, with its encoding
        let color_parts: Vec<Vec<ColorPart>> = color
            .iter()
            .map(|&(image, mips)| {
                std::iter::once((image.rgba.as_slice(), size(image)))
                    .chain(levels_of(image, mips))
                    .map(|(texels, size)| (texels, size, RgbaEncoding::for_texels(texels)))
                    .collect()
            })
            .collect();
        let sizes: Vec<_> = gray
            .iter()
            .map(|image| size(image))
            .chain(color_parts.iter().flatten().flat_map(|&(_, size, encoding)| std::iter::repeat_n(size, encoding as usize)))
            .chain(raw.iter().map(|plane| plane.size.map(|side| side.max(1))))
            .collect();
        let budget = if color.is_empty() && raw.is_empty() { MAX_ATLAS_BYTES } else { MAX_COLOR_ATLAS_BYTES };
        let (extent, regions) =
            arrange_images(&sizes, limits.max_texture_dimension_2d, limits.max_texture_array_layers, device.adapter_info().backend, budget)?;
        let texture = atlas_texture(device, extent[0], extent[1], extent[2]);
        let mut pixels = Vec::new();
        let mut pending = 0;
        let mut upload = |region: &ImageRegion, word: &dyn Fn(usize) -> u32| -> Result<(), String> {
            let [width, height] = region.size;
            for top in (0..height).step_by(IMAGE_ROWS) {
                let rows = (height - top).min(IMAGE_ROWS as u32);
                let first = top as usize * width as usize;
                let n = rows as usize * width as usize;
                pixels.clear();
                pixels.extend((first..first + n).map(word));
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        origin: wgpu::Origin3d { x: region.origin[0], y: region.origin[1] + top, z: region.origin[2] },
                        ..texture.as_image_copy()
                    },
                    bytemuck::cast_slice(&pixels),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: None },
                    wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
                );
                pending += u64::from(width) * u64::from(rows) * 4;
                if pending >= MAX_PENDING_UPLOAD_BYTES {
                    let submission = queue.submit([]);
                    device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|error| error.to_string())?;
                    pending = 0;
                }
            }
            Ok(())
        };
        let (gray_regions, mut remaining) = regions.split_at(gray.len());
        for (image, region) in gray.iter().zip(gray_regions) {
            upload(region, &|index| {
                let sample = image.rgba.get(index).copied().unwrap_or_default();
                ((sample[0] + sample[1] + sample[2]) / 3.0).to_bits()
            })?;
        }
        let mut color_layout = Vec::new();
        for parts in &color_parts {
            let mut uploaded = Vec::new();
            for &(texels, size, encoding) in parts {
                let (planes, rest) = remaining.split_at(encoding as usize);
                remaining = rest;
                for (lane, plane) in planes.iter().enumerate() {
                    upload(plane, &|index| encoding.word(texels.get(index).copied().unwrap_or_default(), lane))?;
                }
                uploaded.push(RgbaLevel { size, encoding, planes: planes.to_vec() });
            }
            let mut uploaded = uploaded.into_iter();
            let Some(image) = uploaded.next() else { continue };
            color_layout.push(RgbaRegion { encoding: image.encoding, planes: image.planes, levels: uploaded.collect() });
        }
        let mut raw_layout = Vec::new();
        for (plane, region) in raw.iter().zip(remaining) {
            upload(region, &|index| plane.texels.get(index).copied().unwrap_or_default().to_bits())?;
            raw_layout.push((Arc::downgrade(&plane.texels), *region));
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Ok(Self {
            images: gray.iter().copied().chain(color.iter().map(|(image, _)| *image)).map(Arc::downgrade).collect(),
            regions: gray_regions.to_vec(),
            color: color_layout,
            color_mips: color.iter().map(|&(_, mips)| mips).collect(),
            raw: raw_layout,
            view,
        })
    }
}

/// The texels, size and encoding of a colour image or one of its halved levels.
type ColorPart<'a> = (&'a [[f32; 4]], [u32; 2], RgbaEncoding);

/// The halved levels `mips` asks for, as texels and size.
fn levels_of(image: &ImageRes, mips: AtlasMips) -> Vec<(&[[f32; 4]], [u32; 2])> {
    let chain = match mips {
        AtlasMips::None => &[][..],
        AtlasMips::Stored => image.mip_chain(),
        AtlasMips::LinearColor => image.linear_mip_chain(),
    };
    chain.iter().map(|level| (level.rgba.as_slice(), [level.width.max(1), level.height.max(1)])).collect()
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

    /// Without mips or raw planes the atlas words are exactly the old ones and the cache still
    /// matches; mip levels and raw planes add their words and invalidate the cache.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn atlas_mips_and_raw_planes_extend_the_old_layout() {
        let Some(gpu) = crate::gpu_composite::oracle::headless() else { return };
        let image = Arc::new(ImageRes::new(8, 4, (0..32).map(|i| [i as f32 / 31.0, 0.5, 0.25, 1.0]).collect(), true));
        let bytes = Arc::new(ImageRes::new(2, 2, vec![[1.0, 0.0, 0.0, 1.0]; 4], true));
        let plain = [(&image, AtlasMips::None), (&bytes, AtlasMips::None)];
        let atlas = TileImageAtlas::new(&gpu.device, &gpu.queue, &[], &plain, &[]).unwrap();
        assert!(atlas.matches(&[], &plain, &[]));
        for (index, (image, _)) in plain.iter().enumerate() {
            let mut words = Vec::new();
            atlas.append_rgba(&mut words, index, true);
            let region = &atlas.color[index];
            let mut old = vec![image.width, image.height, region.encoding as u32 | (1 << 8)];
            old.extend(region.planes.iter().flat_map(|plane| plane.origin));
            assert_eq!(words, old, "image {index}");
        }

        let with_mips = [(&image, AtlasMips::LinearColor), (&bytes, AtlasMips::None)];
        assert!(!atlas.matches(&[], &with_mips, &[]));
        let plane = RawPlane { size: [3, 2], texels: Arc::new(vec![0.5, -1.0, f32::NEG_INFINITY, 2.0, 3.0, 4.0]) };
        let atlas = TileImageAtlas::new(&gpu.device, &gpu.queue, &[], &with_mips, std::slice::from_ref(&plane)).unwrap();
        assert!(atlas.matches(&[], &with_mips, std::slice::from_ref(&plane)));
        assert!(!atlas.matches(&[], &with_mips, &[RawPlane { texels: Arc::new(plane.texels.to_vec()), ..plane.clone() }]));
        let mut words = Vec::new();
        atlas.append_rgba(&mut words, 0, true);
        assert_ne!(words[2] & (1 << 9), 0);
        let levels = image.linear_mip_chain();
        let planes = atlas.color[0].planes.len();
        assert_eq!(words[3 + planes * 3] as usize, levels.len());
        assert_eq!(words[4 + planes * 3..6 + planes * 3], [levels[0].width, levels[0].height]);
        let mut raw = Vec::new();
        atlas.append_raw(&mut raw, 0);
        assert_eq!(raw[..2], [3, 2]);
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
