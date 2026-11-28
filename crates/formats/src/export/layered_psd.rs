//! A texture set's PSD: each output map a layer, a `/` in its name a layer group

use craft_psd::{Compression, GroupSpec, LayerSpec, PixelData, PsdBuilder};
use texturecraft_core::TextureSet;
use texturecraft_texel::linear_to_srgb;

use super::encoding::Encoding;
use super::map_renderer::MapRenderer;
use super::{
    ExportOptions, ExportedFile, FileFormat, PSD_MAX_FILE_BYTES, PSD_RGBA_PLANES, Padding, PlannedFile, PlannedLayer, check_psd_size, psd_size,
    quantize_integer, quantize_u8, quantize_u16,
};
use crate::{FormatError, Result};

/// Where alpha sits in an RGBA texel
const ALPHA_INDEX: usize = 3;

/// Check the file limits and the working memory against `working_limit` (None: unlimited), returning the canvas size and estimated encoded bytes
pub(super) fn preflight<'a>(
    set: &TextureSet,
    options: &ExportOptions,
    tile: Option<u32>,
    depth: u8,
    layers: impl Iterator<Item = &'a PlannedLayer> + Clone,
    working_limit: Option<u64>,
) -> Result<((u32, u32), u64)> {
    let (canvas, file_bytes, working_bytes) = working_set(set, options, tile, depth, layers)?;
    if working_limit.is_some_and(|limit| working_bytes >= limit) {
        let (width, height) = canvas;
        return Err(FormatError::Invalid(format!(
            "a PSD at {width}×{height} would exceed the working memory budget; use a smaller export size, fewer PSD maps or another format"
        )));
    }

    Ok((canvas, file_bytes))
}

/// The canvas size, estimated encoded bytes and peak working bytes of one PSD, after checking the file limits
fn working_set<'a>(
    set: &TextureSet,
    options: &ExportOptions,
    tile: Option<u32>,
    depth: u8,
    layers: impl Iterator<Item = &'a PlannedLayer> + Clone,
) -> Result<((u32, u32), u64, u64)> {
    let texels = |(width, height)| u64::from(width) * u64::from(height);
    let folder_count = layers.clone().map(|layer| layer.name.matches('/').count()).sum();
    let sizes = layers.map(|layer| psd_size(set, tile, options.map_size(layer.map)));
    let (canvas, file_bytes) = check_psd_size(sizes.clone(), folder_count, depth)?;
    let float_bytes_per_texel = (PSD_RGBA_PLANES * size_of::<f32>()) as u64;
    let source_bytes = texels(psd_size(set, tile, None)).saturating_mul(float_bytes_per_texel);
    let largest_layer_bytes = sizes.map(texels).max().unwrap_or(0) * float_bytes_per_texel;

    // The merged image is built a row at a time: a canvas row and a layer row in floating point
    let merge_bytes = 2 * u64::from(canvas.0) * float_bytes_per_texel;

    // Resizing overlaps source and destination, and RGBA conversion can overlap two output buffers
    let mut scratch_bytes = source_bytes.saturating_add(largest_layer_bytes).max(2 * largest_layer_bytes);
    if matches!(options.padding, Padding::DilationDiffusion(_)) {
        // A copied block, its weights, and the smaller pyramid levels fit within this source allowance
        scratch_bytes = scratch_bytes.max(source_bytes.saturating_mul(3));
    }

    let mut padding_bytes = 0;
    let mut padding_peak = 0;
    if options.padding != Padding::None {
        let atlas_texels = texels(set.size());
        padding_bytes = atlas_texels.saturating_mul(size_of::<u32>() as u64);

        // The atlas keeps its dilation indices after the frontiers are freed
        padding_peak = padding_bytes.saturating_add(atlas_texels.saturating_mul((4 * size_of::<usize>()) as u64));
    }

    // Budget for the retained layers and merged image alongside encoder planes, staging and output
    let rendering = scratch_bytes.max(merge_bytes).saturating_add(file_bytes);
    let encoding = 3 * file_bytes;
    let working_bytes = padding_bytes.saturating_add(rendering.max(encoding)).max(padding_peak);
    Ok((canvas, file_bytes, working_bytes))
}

/// Render each layer at its requested size, aligned to the upper-left corner of the PSD canvas.
pub(super) fn render_psd(renderer: &MapRenderer, planned: PlannedFile, size: (u32, u32)) -> Result<ExportedFile> {
    let PlannedFile { name, tile, bit_depth, mut warnings, layers, .. } = planned;

    // Only the current layer is in floating point; the others are kept as the file stores them
    let mut pixels = Vec::with_capacity(layers.len());
    let mut embedded = None;
    for layer in layers.iter().rev() {
        let map = renderer.render(layer.map, tile, FileFormat::Psd, bit_depth)?;
        embedded = map.embed.or(embedded);
        let encoding = Encoding { format: FileFormat::Psd, depth: bit_depth, dither: layer.dither };
        let layer_pixels = rgba_pixel_data(map.data, map.component_count, &map.srgb, map.size.0, encoding)?;
        pixels.push((layer.name.as_str(), map.size, layer_pixels));
    }
    if let Some(space) = embedded {
        warnings.push(format!("PSD export does not embed a colour space; the colours are in {}", space.name()));
    }

    let composite = merged_image(size, bit_depth, &pixels)?;

    let bytes = write_layers(size, bit_depth, pixels, composite).map_err(|error| FormatError::Invalid(format!("psd encoding failed: {error}")))?;
    if bytes.len() as u64 >= PSD_MAX_FILE_BYTES {
        return Err(FormatError::Invalid(format!("\"{name}\" would reach the 2 GiB PSD file limit; use a smaller export size or fewer PSD maps")));
    }

    Ok(ExportedFile { name, bytes, width: size.0, height: size.1, warnings })
}

/// The merged image of `layers` (bottom first, as the file stores them), one row at a time
fn merged_image((width, height): (u32, u32), depth: u8, layers: &[(&str, (u32, u32), PixelData)]) -> Result<PixelData> {
    let row_len = width as usize * PSD_RGBA_PLANES;
    let sample_count = row_len * height as usize;
    let mut merged_row = vec![0.0f32; row_len];
    let mut layer_row = Vec::with_capacity(row_len);
    let reserve_error = |error| FormatError::Invalid(format!("PSD merged image allocation failed: {error}"));
    if depth == 16 {
        let mut merged = Vec::new();
        merged.try_reserve_exact(sample_count).map_err(reserve_error)?;
        for y in 0..height as usize {
            merge_row(y, width, layers, &mut merged_row, &mut layer_row);
            merged.extend(merged_row.iter().map(|&value| quantize_u16(value)));
        }
        return Ok(PixelData::Rgba16(merged));
    }

    let mut merged = Vec::new();
    merged.try_reserve_exact(sample_count).map_err(reserve_error)?;
    for y in 0..height as usize {
        merge_row(y, width, layers, &mut merged_row, &mut layer_row);
        merged.extend(merged_row.iter().copied().map(quantize_u8));
    }

    Ok(PixelData::Rgba8(merged))
}

/// Row `y` of the merged image into `merged_row`, each layer's row decoded into `layer_row` in turn
fn merge_row(y: usize, width: u32, layers: &[(&str, (u32, u32), PixelData)], merged_row: &mut [f32], layer_row: &mut Vec<f32>) {
    merged_row.fill(0.0);
    for (_, (layer_width, layer_height), samples) in layers {
        if y >= *layer_height as usize {
            continue;
        }

        let span = *layer_width as usize * PSD_RGBA_PLANES;
        let range = y * span..(y + 1) * span;
        layer_row.clear();
        match samples {
            PixelData::Rgba8(samples) => layer_row.extend(samples.get(range).unwrap_or_default().iter().map(|&v| f32::from(v) / f32::from(u8::MAX))),
            PixelData::Rgba16(samples) => layer_row.extend(samples.get(range).unwrap_or_default().iter().map(|&v| f32::from(v) / f32::from(u16::MAX))),
            _ => continue,
        }
        over(merged_row, width, layer_row, *layer_width);
    }
}

/// Composite a straight-alpha RGBA layer at the top left of the canvas.
fn over(canvas: &mut [f32], canvas_width: u32, layer: &[f32], layer_width: u32) {
    let canvas_rows = canvas.chunks_exact_mut(canvas_width as usize * PSD_RGBA_PLANES);
    let layer_rows = layer.chunks_exact(layer_width as usize * PSD_RGBA_PLANES);
    for (canvas_row, layer_row) in canvas_rows.zip(layer_rows) {
        let canvas_texels = canvas_row.as_chunks_mut::<PSD_RGBA_PLANES>().0;
        let layer_texels = layer_row.as_chunks::<PSD_RGBA_PLANES>().0;
        for (under, top) in canvas_texels.iter_mut().zip(layer_texels) {
            let (top_alpha, under_alpha) = (top[ALPHA_INDEX], under[ALPHA_INDEX]);
            let alpha = top_alpha + under_alpha * (1.0 - top_alpha);
            if alpha <= 0.0 {
                continue;
            }

            for (value, top_value) in under.iter_mut().zip(top).take(ALPHA_INDEX) {
                *value = (top_value * top_alpha + *value * under_alpha * (1.0 - top_alpha)) / alpha;
            }
            under[ALPHA_INDEX] = alpha;
        }
    }
}

/// L, LA, RGB or RGBA samples quantized as the RGBA the PSD builder accepts: gray repeated, opaque without alpha, sRGB transfer applied to the components `srgb` marks, values clamped to 0-1
fn rgba_pixel_data(mut data: Vec<f32>, component_count: usize, srgb: &[bool], width: u32, encoding: Encoding) -> Result<PixelData> {
    let component_count = component_count.max(1);
    for texel in data.chunks_exact_mut(component_count) {
        for (component, value) in texel.iter_mut().enumerate() {
            *value = if srgb.get(component).copied().unwrap_or(false) { linear_to_srgb(*value) } else { value.clamp(0.0, 1.0) };
        }
    }

    // Widen each texel while quantizing, so no floating-point RGBA copy of the layer is made
    let texel_count = data.len() / component_count;
    let rgba = (0..texel_count * PSD_RGBA_PLANES).map(|index| {
        let (texel, component) = (index / PSD_RGBA_PLANES, index % PSD_RGBA_PLANES);
        let source = match component_count {
            1 | 2 if component < ALPHA_INDEX => Some(0),
            1 | 3 if component == ALPHA_INDEX => None,
            2 => Some(1),
            _ => Some(component),
        };
        source.map_or(1.0, |source| data.get(texel * component_count + source).copied().unwrap_or(0.0))
    });

    if encoding.depth == 16 {
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(rgba.len()).map_err(|error| FormatError::Invalid(format!("PSD sample buffer allocation failed: {error}")))?;
        pixels.extend(rgba.map(quantize_u16));
        return Ok(PixelData::Rgba16(pixels));
    }

    quantize_integer(encoding, width, PSD_RGBA_PLANES, rgba).map(PixelData::Rgba8)
}

/// An RGB PSD of `layers` (bottom first, each named by its `/`-separated group path) over the merged `composite`, uncompressed so its size is known up front
pub(super) fn write_layers(
    (width, height): (u32, u32),
    depth: u8,
    layers: Vec<(&str, (u32, u32), PixelData)>,
    composite: PixelData,
) -> craft_psd::Result<Vec<u8>> {
    let mut builder = PsdBuilder::new(width, height).depth(u16::from(depth)).compression(Compression::Raw);

    // The builder stacks bottom up, so the groups open and close in that order too
    let mut open: Vec<&str> = Vec::new();
    for (path, (layer_width, layer_height), pixels) in layers {
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        let shared = open.iter().zip(&parts).take_while(|(open, part)| open == part).count();
        for _ in shared..open.len() {
            builder.end_group()?;
        }
        open.truncate(shared);
        for group in parts.iter().skip(shared) {
            builder.begin_group(GroupSpec::new(*group));
            open.push(group);
        }

        builder.push_layer(LayerSpec::new(name, 0, 0, layer_width, layer_height, pixels));
    }
    for _ in open {
        builder.end_group()?;
    }

    builder.composite(composite);
    builder.to_bytes()
}
