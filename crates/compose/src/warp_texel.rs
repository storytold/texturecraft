//! What a Warp filter reads for one texel: its source, then the moves its mode makes

use texturecraft_core::{SourceWarp, WarpBlending, WarpMode};

use crate::warp_source_sample::WarpSourceSample;

/// Bilinear sample at an absolute texel position into an accumulator (components, then weight), as [`crate::eval`]'s gather reads
pub(crate) type Sample<'a> = &'a dyn Fn(f32, f32, &mut [f32; 5]);

/// The filter's Custom Noise image input at an absolute texel position; None where the values it filters cover nothing
pub(crate) type Input<'a> = &'a dyn Fn(f32, f32) -> Option<f32>;

/// A read weighing less than this covers nothing
pub(crate) const MIN_READ_WEIGHT: f32 = 1e-6;

/// Warp `warp` at texel `(x, y)` into `acc`: where its source sends it, or those reads blended
pub(crate) fn warp_texel(
    (warp, directions): (&SourceWarp, &[[f32; 2]]),
    (x, y): (f32, f32),
    (res, comps): (u32, usize),
    tile_sizes: ([u32; 2], [u32; 2]),
    acc: &mut [f32; 5],
    (sample, input): (Sample<'_>, Input<'_>),
) {
    let source = WarpSourceSample::new(&warp.source, warp.seed, [x, y], (res, comps), tile_sizes, (sample, input));
    let reach = warp.move_texels(res);
    if warp.mode == WarpMode::Warp {
        let probe = SourceWarp::probe_texels(res);
        let rise = [source.at(x + probe, y) - source.at(x - probe, y), source.at(x, y + probe) - source.at(x, y - probe)];
        let [dx, dy] = warp.slope_move(rise, res);
        sample(x + dx, y + dy, acc);
        return;
    }

    let directions = directions.iter().copied();
    let displacement = |at_x, at_y| SourceWarp::amount(source.at(at_x, at_y)) * reach;
    if warp.mode == WarpMode::MultiDirectional && warp.blending == WarpBlending::Chain {
        let (end_x, end_y) = directions.fold((x, y), |(at_x, at_y), [dx, dy]| {
            let along = displacement(at_x, at_y);
            (at_x + dx * along, at_y + dy * along)
        });
        sample(end_x, end_y, acc);
        return;
    }

    // Independent directions share the source at the original texel
    let along = displacement(x, y);
    let reads = directions.map(|[dx, dy]| (x + dx * along, y + dy * along));
    if warp.mode == WarpMode::MultiDirectional && matches!(warp.blending, WarpBlending::Max | WarpBlending::Min) {
        keep_extreme_read(reads, warp.blending == WarpBlending::Max, comps, acc, sample);
    } else {
        for (at_x, at_y) in reads {
            sample(at_x, at_y, acc);
        }
    }
}

/// Keeps in `acc` the largest (or smallest) of the reads at `positions`, component by component; reads that cover nothing are skipped
pub(crate) fn keep_extreme_read(positions: impl Iterator<Item = (f32, f32)>, larger: bool, comps: usize, acc: &mut [f32; 5], sample: Sample<'_>) {
    let mut picked: Option<[f32; 5]> = None;
    for (at_x, at_y) in positions {
        let mut read = [0.0f32; 5];
        sample(at_x, at_y, &mut read);
        if read[4] <= MIN_READ_WEIGHT {
            continue;
        }

        let read = read.map(|component| component / read[4]);
        picked = Some(match picked {
            Some(so_far) => std::array::from_fn(|k| if larger { so_far[k].max(read[k]) } else { so_far[k].min(read[k]) }),
            None => read,
        });
    }

    if let Some(picked) = picked {
        let kept = comps.min(4);
        acc[..kept].copy_from_slice(&picked[..kept]);
        acc[4] = 1.0;
    }
}
