use rayon::prelude::*;
use texturecraft_compose::PaddingMaps;
use texturecraft_core::{TextureSet, TileGrid};
use texturecraft_texel::ColorSpace;

use super::rendered_map::RenderedMap;
use super::{
    At, Dest, ExportInput, ExportOptions, FileFormat, Padding, PlannedFile, Sampler, check_scaled_block, diffuse_blocks, layout, resize, scale_export_side,
    tile_side,
};
use crate::{FormatError, MAX_IMAGE_SIZE, Result};

/// Renders the output maps of one texture set's export: the padding and sampling every map shares
pub(super) struct MapRenderer<'a> {
    options: &'a ExportOptions,
    set: &'a TextureSet,
    sampler: Sampler<'a>,
    padding: Option<PaddingMaps>,
    grid: Option<TileGrid>,
    /// The composite's size
    width: u32,
    height: u32,
    /// Dimensions of an atlas block (the whole texture for a set without UV tiles)
    block: (u32, u32),
}

impl<'a> MapRenderer<'a> {
    pub(super) fn new(input: &ExportInput<'a>, options: &'a ExportOptions, planned: &[PlannedFile]) -> Result<MapRenderer<'a>> {
        let set = input.project.texture_set(input.ts)?;
        let (width, height) = (input.composite.resolution, input.composite.height);
        let grid = set.tile_grid();
        let (cols, rows) = grid.map_or((1, 1), |grid| (grid.cols.max(1), grid.rows.max(1)));
        let block = (width / cols, height / rows);
        if block.0.min(block.1) == 0 || block.0.max(block.1) > MAX_IMAGE_SIZE || (block.0 * cols, block.1 * rows) != (width, height) {
            return Err(FormatError::Invalid(format!("composite size {width}×{height} does not fit texture set \"{}\"", set.name)));
        }

        if input.coverage.len() as u64 != u64::from(width) * u64::from(height) {
            return Err(FormatError::Invalid(format!("coverage has {} texels, expected {width}×{height}", input.coverage.len())));
        }

        for map in planned.iter().flat_map(PlannedFile::maps) {
            check_scaled_block(set, block, options.map_size(map))?;
        }

        // All maps share one padding lookup for this texture set
        let make_padding = |reach| PaddingMaps::new(set.uv_padding, input.coverage, (width, height), reach, block, input.seams);
        let padding = match options.padding {
            Padding::None => None,
            Padding::Dilation(reach) | Padding::DilationDefault(reach) | Padding::DilationDiffusion(reach) => Some(make_padding(Some(reach))),
            Padding::Infinite => Some(make_padding(None)),
        };
        let sampler = Sampler::new(input, options, set, block, false);
        Ok(MapRenderer { options, set, sampler, padding, grid, width, height, block })
    }

    /// The export size of map `map_index`'s file of `tile` (None: the whole set)
    pub(super) fn size(&self, map_index: usize, tile: Option<u32>) -> Result<(u32, u32)> {
        let (_, _, w, h) = self.region(tile)?;

        // An atlas scales by the requested size per tile
        let scale = |value| scale_export_side(value, self.block.0, self.options.map_size(map_index));
        Ok((scale(w), scale(h)))
    }

    /// The composite texels a file of `tile` covers: one tile, or the whole set
    fn region(&self, tile: Option<u32>) -> Result<(u32, u32, u32, u32)> {
        match (tile, self.grid) {
            (Some(t), Some(g)) => {
                let (bx, by) = g.block(t).ok_or_else(|| FormatError::Invalid(format!("UV tile {t} is not in the set's grid")))?;

                // A tile smaller than its block fills the block's top-left corner
                let (width, height) = if self.set.tile_resolutions.is_empty() {
                    self.block
                } else {
                    let side = tile_side(self.set, t, self.block.0);
                    (side, side)
                };
                Ok((bx * self.block.0, by * self.block.1, width, height))
            }
            _ => Ok((0, 0, self.width, self.height)),
        }
    }

    /// Map `map_index` of the preset for `tile`, as `format` at `bit_depth` holds it
    pub(super) fn render(&self, map_index: usize, tile: Option<u32>, format: FileFormat, bit_depth: u8) -> Result<RenderedMap> {
        let options = self.options;
        let map = options.preset.maps.get(map_index).ok_or_else(|| FormatError::Invalid("planned map out of range".into()))?;
        let channel_layout = layout(self.set, map);
        let component_count = channel_layout.len();
        let inverted: Vec<bool> = channel_layout.iter().map(|(d, _)| map.inverted.contains(d)).collect();
        let pads_flat: Vec<bool> = channel_layout.iter().map(|(_, source)| source.pads_flat()).collect();
        let (x0, y0, w, h) = self.region(tile)?;
        let size = self.size(map_index, tile)?;
        let sampler = &self.sampler;
        let padding = self.padding.as_ref();
        let beyond_default = matches!(options.padding, Padding::DilationDefault(_) | Padding::Infinite);

        let mut data = vec![0.0f32; w as usize * h as usize * component_count];
        data.par_chunks_mut(w as usize * component_count).enumerate().for_each(|(row_y, row)| {
            let y = y0 + row_y as u32;
            for (column, x) in (x0..x0 + w).enumerate() {
                let at = |cross_seams| {
                    padding
                        .and_then(|padding| padding.source(cross_seams, x, y))
                        .map(|(sx, sy)| At::Texel(sx, sy))
                        .or_else(|| beyond_default.then_some(At::Default))
                };
                let (padded, flat) = if sampler.covered(x, y) { (Some(At::Texel(x, y)), Some(At::Texel(x, y))) } else { (at(true), at(false)) };

                // The normal cache is shared by virtual normal components, which all use flat padding
                let mut cache = None;
                for (component, (_, source)) in channel_layout.iter().enumerate() {
                    let Some(at) = (if pads_flat.get(component).copied().unwrap_or(false) { flat } else { padded }) else { continue };
                    if let Some(value) = row.get_mut(column * component_count + component) {
                        let sample = sampler.sample(source, at, &mut cache);
                        *value = if inverted.get(component).copied().unwrap_or(false) { 1.0 - sample } else { sample };
                    }
                }
            }
        });

        // Diffuse into the texels the dilation did not reach
        if let (Padding::DilationDiffusion(_), Some(padding)) = (options.padding, padding) {
            let block = if tile.is_none() && self.grid.is_some() { self.block } else { (w, h) };
            diffuse_blocks(
                &mut data,
                (w as usize, h as usize),
                component_count,
                &padding.flat,
                (x0 as usize, y0 as usize),
                (block.0 as usize, block.1 as usize),
            );
        }

        let mut data = resize(data, (w, h), component_count, size);

        // Integer sRGB converts in `encode`; other color spaces convert here
        let space = options.space_for_map(map, format, bit_depth);
        let float = format.holds_float(bit_depth);
        let convert = (space != ColorSpace::Srgb || float) && space != ColorSpace::LinearRec709;
        if convert {
            data.par_chunks_mut(component_count).for_each(|px| {
                if let [r, g, b, ..] = px {
                    [*r, *g, *b] = space.from_linear_rec709([*r, *g, *b]);
                }
            });
        }
        let srgb = channel_layout.iter().map(|(d, _)| *d != Dest::A && !float && space == ColorSpace::Srgb).collect();

        // A colour space the user chose is written into the colour maps (data maps have none)
        let embed = (options.color_space.is_some() && map.is_color() && component_count >= 3).then_some(space);
        Ok(RenderedMap { data, size, component_count, srgb, embed })
    }
}
