//! A warp's or slope blur's source as read around one texel

use texturecraft_core::{WarpSource, WarpSourceMode, lum};

use crate::eval::{MIN_ALPHA, texel_noise};
use crate::warp_texel::{Input, MIN_READ_WEIGHT, Sample};

/// A warp's or slope blur's source at one texel's neighbourhood
pub(crate) struct WarpSourceSample<'a> {
    source: &'a WarpSource,
    /// The default noise's seed
    seed: u32,
    res: u32,
    comps: usize,
    sample: Sample<'a>,
    input: Input<'a>,
    /// Half the texels the source is blurred across, worked out once per texel
    half_blur: f32,
    tiling: f32,
    tile_origin: [f32; 2],
    tile_size: [f32; 2],
}

impl<'a> WarpSourceSample<'a> {
    /// Read the source around `texel`. Atlas blocks retain their full stride even when a UV tile has fewer texels.
    pub(crate) fn new(
        source: &'a WarpSource,
        seed: u32,
        texel: [f32; 2],
        (res, comps): (u32, usize),
        (block_size, tile_size): ([u32; 2], [u32; 2]),
        (sample, input): (Sample<'a>, Input<'a>),
    ) -> Self {
        let tiling = source.tiling_factor();
        let tile_origin = if source.mode != WarpSourceMode::DefaultNoise && tiling != 1.0 {
            std::array::from_fn(|axis| {
                let stride = block_size[axis].max(1) as f32;
                (texel[axis] / stride).floor() * stride
            })
        } else {
            [0.0; 2]
        };

        Self {
            source,
            seed,
            res,
            comps,
            sample,
            input,
            half_blur: source.blur_texels(res) / 2.0,
            tiling,
            tile_origin,
            tile_size: tile_size.map(|size| size.max(1) as f32),
        }
    }

    /// The source at texel position `(x, y)` after its blur, Balance and Contrast
    pub(crate) fn at(&self, x: f32, y: f32) -> f32 {
        let half = self.half_blur;
        let value = if half > 0.0 {
            let corners = [(-half, -half), (half, -half), (-half, half), (half, half)];
            let (sum, count) = corners.iter().filter_map(|(dx, dy)| self.raw(x + dx, y + dy)).fold((0.0f32, 0u8), |(sum, count), read| (sum + read, count + 1));
            if count == 0 { WarpSource::MID_GRAY } else { sum / f32::from(count) }
        } else {
            self.raw(x, y).unwrap_or(WarpSource::MID_GRAY)
        };
        self.source.shape(value)
    }

    /// The source at texel position `(x, y)` before it is shaped; None where the values it filters cover nothing
    fn raw(&self, x: f32, y: f32) -> Option<f32> {
        if self.source.mode == WarpSourceMode::DefaultNoise {
            return Some(WarpSource::stretch_noise(texel_noise(x, y, [0.0; 2], self.res, self.source.noise_scale(), self.seed)));
        }

        let [x, y] = if self.tiling == 1.0 {
            [x, y]
        } else {
            let repeat = |axis: usize, position: f32| {
                let origin = self.tile_origin[axis];
                let size = self.tile_size[axis];
                let repeated = ((position - origin + 0.5) * self.tiling).rem_euclid(size) - 0.5;

                // Keep the bilinear taps in this UV tile at the ends of each repeat
                origin + repeated.clamp(0.0, size - 1.0)
            };
            [repeat(0, x), repeat(1, y)]
        };

        if self.source.mode == WarpSourceMode::CustomNoise {
            return (self.input)(x, y);
        }

        let mut read = [0.0f32; 5];
        (self.sample)(x, y, &mut read);
        match (self.comps, read) {
            // Content stays premultiplied by its own coverage
            (4, [r, g, b, alpha, _]) if alpha > MIN_ALPHA => Some(lum([r / alpha, g / alpha, b / alpha])),
            (1, [value, .., weight]) if weight > MIN_READ_WEIGHT => Some(value / weight),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiled_input_reads_stay_within_the_current_atlas_block() {
        let source = WarpSource { mode: WarpSourceMode::CustomNoise, tiling: 2.0, ..WarpSource::LIBRARY };
        let sample = |_x, _y, _read: &mut [f32; 5]| {};
        let input = |x: f32, y: f32| {
            assert!((64.0..=95.0).contains(&x) && (128.0..=159.0).contains(&y), "read at {x}, {y}");
            Some(((x - 64.0) + (y - 128.0)) / 64.0)
        };
        let tiled = WarpSourceSample::new(&source, 0, [94.0, 158.0], (32, 1), ([64; 2], [32; 2]), (&sample, &input));

        let first_repeat = tiled.at(69.0, 135.0);
        let second_repeat = tiled.at(85.0, 151.0);

        assert_eq!(first_repeat, second_repeat);
        assert_ne!(tiled.at(70.0, 135.0), first_repeat);
        assert!(tiled.at(63.0, 160.0).is_finite());
    }
}
