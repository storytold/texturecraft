//! Area-averaged downsampling of dense float images.
//!
//! A destination texel covers a rectangle of the source; its value is the source averaged over
//! that rectangle, each source texel weighted by the area it shares with it (a box filter). The
//! ratio need not be an integer, and each axis has its own ratio, so a rectangular image can
//! shrink along one axis only.

use rayon::prelude::*;

/// The source texels one destination texel covers along one axis: from `first`, one weight
/// each (the share of the destination texel each covers; the weights sum to 1).
#[derive(Debug, PartialEq)]
struct Span {
    first: usize,
    weights: Vec<f64>,
}

/// The spans of `to` destination texels over `from` source texels (`1 ≤ to ≤ from`).
///
/// Destination texel `i` covers `[i·from/to, (i+1)·from/to)` in source texels. Bounds are
/// compared scaled by `to`, so the overlaps are exact integers.
fn spans(from: u32, to: u32) -> Vec<Span> {
    let (s, d) = (u64::from(from), u64::from(to));
    (0..d)
        .map(|i| {
            let (lo, hi) = (i * s, (i + 1) * s);
            let (first, last) = (lo / d, hi.div_ceil(d));
            let weights = (first..last).map(|j| (hi.min((j + 1) * d) - lo.max(j * d)) as f64 / s as f64).collect();
            Span { first: first as usize, weights }
        })
        .collect()
}

/// `src`, a row-major `from` (width, height) image of `comps` floats per texel, area-averaged
/// down to `to` (see the [module docs](self)). An axis of the same size is copied texel for
/// texel, so `to == from` returns the image unchanged.
///
/// Linear in the source texels; rows are averaged in parallel.
///
/// Returns `None` when `to` is empty or larger than `from` on either axis, `comps` is 0, or
/// `src` does not hold exactly `from.0 × from.1 × comps` values.
pub fn area_downsample(src: &[f32], from: (u32, u32), comps: usize, to: (u32, u32)) -> Option<Vec<f32>> {
    if comps == 0 || to.0 == 0 || to.1 == 0 || to.0 > from.0 || to.1 > from.1 {
        return None;
    }
    let src_row = (from.0 as usize).checked_mul(comps)?;
    if src.len() != src_row.checked_mul(from.1 as usize)? {
        return None;
    }
    if to == from {
        return Some(src.to_vec());
    }
    let dst_row = to.0 as usize * comps;
    let mut out = vec![0.0f32; dst_row.checked_mul(to.1 as usize)?];
    let (xs, ys) = (spans(from.0, to.0), spans(from.1, to.1));
    let rows: Vec<&[f32]> = src.chunks_exact(src_row).collect();
    out.par_chunks_exact_mut(dst_row).zip(&ys).for_each(|(dst, ys)| {
        let mut acc = vec![0.0f64; dst_row];
        let mut line = vec![0.0f64; dst_row];
        for (row, wy) in rows.iter().skip(ys.first).zip(&ys.weights) {
            // The source row averaged across, then weighted into the destination row.
            for (texel, xs) in line.chunks_exact_mut(comps).zip(&xs) {
                texel.fill(0.0);
                for (s, wx) in row.chunks_exact(comps).skip(xs.first).zip(&xs.weights) {
                    for (t, v) in texel.iter_mut().zip(s) {
                        *t += wx * f64::from(*v);
                    }
                }
            }
            for (a, l) in acc.iter_mut().zip(&line) {
                *a += wy * l;
            }
        }
        for (d, a) in dst.iter_mut().zip(&acc) {
            *d = *a as f32;
        }
    });
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn integer_ratio_is_an_exact_box_average() {
        // 4×4 → 2×2: each output is the mean of a 2×2 block.
        let src: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let out = area_downsample(&src, (4, 4), 1, (2, 2)).unwrap();
        assert_eq!(out, [2.5, 4.5, 10.5, 12.5]);
        // 8× with a 1-texel line: the line's share, not a pick of it or of its neighbours.
        let mut line = vec![0.0f32; 64 * 64];
        for y in 0..64 {
            line[y * 64 + 3] = 1.0;
        }
        let out = area_downsample(&line, (64, 64), 1, (8, 8)).unwrap();
        for (i, v) in out.iter().enumerate() {
            assert_eq!(*v, if i % 8 == 0 { 0.125 } else { 0.0 }, "texel {i}");
        }
    }

    #[test]
    fn non_integer_ratio_weights_by_overlap() {
        // 3 → 2: output 0 covers [0, 1.5): texel 0 fully, texel 1 half (weights 2/3, 1/3).
        assert_eq!(spans(3, 2), [Span { first: 0, weights: vec![2.0 / 3.0, 1.0 / 3.0] }, Span { first: 1, weights: vec![1.0 / 3.0, 2.0 / 3.0] }]);
        let out = area_downsample(&[0.0, 3.0, 6.0], (3, 1), 1, (2, 1)).unwrap();
        assert!(close(&out, &[1.0, 5.0]), "{out:?}");
        // 5 → 3 (spans straddle texels on both sides), and every row of weights sums to 1.
        for s in spans(5, 3).iter().chain(&spans(8192, 3000)) {
            assert!((s.weights.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }
        let out = area_downsample(&[0.0, 5.0, 10.0, 15.0, 20.0], (5, 1), 1, (3, 1)).unwrap();
        // [0, 5/3): 0·3/5 + 5·2/5 = 2; [5/3, 10/3): 5·1/5 + 10·3/5 + 15·1/5 = 10; mirror: 18.
        assert!(close(&out, &[2.0, 10.0, 18.0]), "{out:?}");
    }

    #[test]
    fn rectangular_images_shrink_per_axis() {
        // 4×2, 3 components, down to 2×1: each output averages a 2×2 block per component.
        let src: Vec<f32> = (0..24).map(|i| i as f32).collect();
        let out = area_downsample(&src, (4, 2), 3, (2, 1)).unwrap();
        // Texel (x, y) component k is (y·4 + x)·3 + k: block means are 7.5+k and 13.5+k.
        assert!(close(&out, &[7.5, 8.5, 9.5, 13.5, 14.5, 15.5]), "{out:?}");
        // Only one axis shrinks: rows are averaged, columns kept.
        let out = area_downsample(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], (2, 3), 1, (2, 1)).unwrap();
        assert!(close(&out, &[3.0, 4.0]), "{out:?}");
        let out = area_downsample(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], (3, 2), 1, (1, 2)).unwrap();
        assert!(close(&out, &[2.0, 5.0]), "{out:?}");
    }

    #[test]
    fn one_texel_output_is_the_mean() {
        let src: Vec<f32> = (0..7 * 5).map(|i| i as f32).collect();
        let out = area_downsample(&src, (7, 5), 1, (1, 1)).unwrap();
        assert!(close(&out, &[17.0]), "{out:?}");
    }

    #[test]
    fn same_size_is_identity() {
        let src: Vec<f32> = (0..30).map(|i| (i as f32 * 0.37).sin()).collect();
        assert_eq!(area_downsample(&src, (5, 2), 3, (5, 2)).unwrap(), src);
    }

    #[test]
    fn hostile_sizes_are_refused() {
        let src = vec![0.0f32; 16];
        assert_eq!(area_downsample(&src, (4, 4), 1, (8, 2)), None, "growing");
        assert_eq!(area_downsample(&src, (4, 4), 1, (0, 2)), None, "empty");
        assert_eq!(area_downsample(&src, (4, 4), 0, (2, 2)), None, "no components");
        assert_eq!(area_downsample(&src, (4, 5), 1, (2, 2)), None, "short data");
        assert_eq!(area_downsample(&src, (u32::MAX, u32::MAX), usize::MAX, (1, 1)), None, "overflow");
    }
}
