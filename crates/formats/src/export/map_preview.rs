//! A small picture of one output map, sampled from the texture set as the export samples it

use rayon::prelude::*;
use texturecraft_texel::linear_to_srgb;

use super::{At, Dest, ExportInput, ExportOptions, FileFormat, Sampler, SourceRef, layout, list_outputs, rec709_luminance};
use crate::{FormatError, Result};

/// The largest preview side: a preview is a glance, never a second export
pub const MAX_PREVIEW_SIDE: u32 = 512;

/// One output map, small: its channels in file order and the 8-bit values an sRGB file of it stores (colour channels sRGB-encoded, data as is)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapPreview {
    pub width: u32,
    pub height: u32,
    pub channels: Vec<Dest>,
    /// `channels.len()` values per pixel, row by row
    pub pixels: Vec<u8>,
}

impl MapPreview {
    /// Output map `map` of `options`' template for `input`'s set, with each side at most `side` pixels.
    /// Returns None when the map writes no file. Texels off the UV islands show the default.
    pub fn sample(input: &ExportInput, options: &ExportOptions, map: usize, side: u32) -> Result<Option<MapPreview>> {
        Self::sample_coverage(input, options, map, side, false)
    }

    /// Sample the engine's triangle indices directly, using the UV rasterizer's uncovered sentinel.
    pub fn sample_triangles(input: &ExportInput<u32>, options: &ExportOptions, map: usize, side: u32) -> Result<Option<MapPreview>> {
        Self::sample_coverage(input, options, map, side, texturecraft_geom::NO_TRI)
    }

    fn sample_coverage<Coverage: PartialEq + Sync>(
        input: &ExportInput<Coverage>,
        options: &ExportOptions,
        map: usize,
        side: u32,
        uncovered: Coverage,
    ) -> Result<Option<MapPreview>> {
        let set = input.project.texture_set(input.ts)?;
        let spec = options.preset.maps.get(map).ok_or_else(|| FormatError::Invalid(format!("the template has no output map {map}")))?;
        let planned = list_outputs(input.project, input.ts, input.mesh_name, options)?;
        let Some(file) = planned.iter().find(|file| file.maps().any(|written| written == map)) else {
            return Ok(None);
        };

        let (composite_width, height) = (input.composite.resolution, input.composite.height);
        if composite_width == 0 || height == 0 || input.coverage.len() as u64 != u64::from(composite_width) * u64::from(height) {
            return Err(FormatError::Invalid(format!("composite size {composite_width}×{height} does not match its coverage")));
        }

        // The file's longer side bounds the preview: the map's own size, else the export size, else the set's
        let (cols, grid_rows) = set.tile_grid().map_or((1, 1), |grid| (grid.cols.max(1), grid.rows.max(1)));
        let block = ((composite_width / cols).max(1), (height / grid_rows).max(1));
        let file_side = |source| super::scale_export_side(source, block.0, options.map_size(map));
        let source_side = composite_width.max(height);
        let preview_side = side.clamp(1, MAX_PREVIEW_SIDE).min(file_side(composite_width).max(file_side(height))).min(source_side);
        let width = super::scale_export_side(composite_width, source_side, Some(preview_side));
        let rows = super::scale_export_side(height, source_side, Some(preview_side));
        let sampler = Sampler::new(input, options, set, block, uncovered);
        let mut channel_layout = layout(set, spec);
        let grayscale = file.format == FileFormat::Pgm;
        if grayscale || matches!(file.format, FileFormat::Jpeg | FileFormat::Hdr) {
            channel_layout.retain(|(dest, _)| *dest != Dest::A);
        }

        let gray_source = channel_layout.first().filter(|(dest, _)| *dest == Dest::L);
        let psd = file.format == FileFormat::Psd;
        let rgb_from_gray = gray_source.filter(|_| {
            matches!(file.format, FileFormat::Psd | FileFormat::Hdr | FileFormat::Webp)
                || (file.format == FileFormat::Tiff && (file.bit_depth == 32 || channel_layout.len() == 2))
        });
        let channels = if grayscale {
            vec![Dest::L]
        } else if psd {
            vec![Dest::R, Dest::G, Dest::B, Dest::A]
        } else if rgb_from_gray.is_some() {
            [Dest::R, Dest::G, Dest::B].into_iter().chain(channel_layout.iter().skip(1).map(|(dest, _)| *dest)).collect()
        } else {
            channel_layout.iter().map(|(dest, _)| *dest).collect()
        };

        let component_count = channels.len();
        let color = spec.is_color();
        let max_sample = f32::from(u8::MAX);
        let preview_sample = |value: f32, dest| {
            let stored = if color && dest != Dest::A { linear_to_srgb(value) } else { value.clamp(0.0, 1.0) };
            (stored * max_sample + 0.5) as u8
        };

        // Sample at the centre of each preview pixel
        let source_texel = |pixel: u32, preview_side: u32, source_side: u32| {
            ((u64::from(pixel) * 2 + 1) * u64::from(source_side) / (2 * u64::from(preview_side))).min(u64::from(source_side) - 1) as u32
        };

        let mut pixels = vec![0u8; width as usize * rows as usize * component_count];
        pixels.par_chunks_mut(width as usize * component_count).enumerate().for_each(|(row, values)| {
            let y = source_texel(row as u32, rows, height);
            for (column, pixel) in values.chunks_exact_mut(component_count).enumerate() {
                let x = source_texel(column as u32, width, composite_width);
                let at = if sampler.covered(x, y) { At::Texel(x, y) } else { At::Default };
                let mut cache = None;
                let mut sample = |&(dest, source): &(Dest, &SourceRef)| {
                    let sample = sampler.sample(source, at, &mut cache);
                    if spec.inverted.contains(&dest) { 1.0 - sample } else { sample }
                };

                if grayscale && let [luminance] = pixel {
                    let value = match channel_layout.as_slice() {
                        [red, green, blue] => rec709_luminance(sample(red), sample(green), sample(blue)),
                        [gray] => sample(gray),
                        _ => 0.0,
                    };
                    *luminance = preview_sample(value, Dest::L);
                } else if let (Some(gray), [red, green, blue, alpha @ ..]) = (rgb_from_gray, &mut *pixel) {
                    let value = preview_sample(sample(gray), Dest::L);
                    (*red, *green, *blue) = (value, value, value);
                    if let [alpha] = alpha {
                        *alpha = channel_layout.get(1).map_or(u8::MAX, |source| preview_sample(sample(source), Dest::A));
                    }
                } else {
                    for (output, channel) in pixel.iter_mut().zip(&channel_layout) {
                        *output = preview_sample(sample(channel), channel.0);
                    }
                    if psd && channel_layout.len() < component_count {
                        pixel[component_count - 1] = u8::MAX;
                    }
                }
            }
        });

        Ok(Some(MapPreview { width, height: rows, channels, pixels }))
    }
}
