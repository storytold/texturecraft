//! What a Blur Slope filter reads for one texel: its source's slope, then the values downhill of it

use texturecraft_core::{SlopeBlending, SourceSlopeBlur, SourceWarp};

use crate::eval::LINE_SAMPLES;
use crate::warp_source_sample::WarpSourceSample;
use crate::warp_texel::{Input, Sample, keep_extreme_read};

/// A source rising less than this across a texel is flat
const FLAT_STEEPNESS: f32 = 1e-6;

/// Blur Slope `slope` at texel `(x, y)` into `acc`: the reads downhill of it, blended
pub(crate) fn slope_texel(
    slope: &SourceSlopeBlur,
    (x, y): (f32, f32),
    (res, comps): (u32, usize),
    tile_sizes: ([u32; 2], [u32; 2]),
    acc: &mut [f32; 5],
    (sample, input): (Sample<'_>, Input<'_>),
) {
    let source = WarpSourceSample::new(&slope.source, slope.seed, [x, y], (res, comps), tile_sizes, (sample, input));
    let probe = SourceWarp::probe_texels(res);
    let downhill = [source.at(x - probe, y) - source.at(x + probe, y), source.at(x, y - probe) - source.at(x, y + probe)];
    let steepness = downhill[0].hypot(downhill[1]);

    // A flat spot keeps its value
    if steepness.is_nan() || steepness <= FLAT_STEEPNESS {
        sample(x, y, acc);
        return;
    }

    let [dx, dy] = downhill.map(|rise| rise / steepness);
    let reach = slope.move_texels(res);
    let read_count = slope.samples((reach.ceil() as u32).clamp(1, LINE_SAMPLES.unsigned_abs()));
    let step = reach / read_count as f32;
    let along = (0..=read_count).map(|read| read as f32 * step);
    if slope.blending == SlopeBlending::Blur {
        for t in along {
            sample(x + dx * t, y + dy * t, acc);
        }
        return;
    }

    let reads = along.map(|t| (x + dx * t, y + dy * t));
    keep_extreme_read(reads, slope.blending == SlopeBlending::Max, comps, acc, sample);
}
