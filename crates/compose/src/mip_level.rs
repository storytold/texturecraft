//! One halved copy of an image resource, read by high-quality bilinear filtering when an image is shrunk

use crate::store::texel_of;

/// One halved copy of an image (box-filtered; see `halve` for its edges)
#[derive(Clone, Debug, PartialEq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[f32; 4]>,
}

/// Share of each of the 2 × 2 texels in a halved texel
const TEXEL_SHARE: f32 = 0.25;

/// A halved interval can overlap at most three source texels on each axis
const AREA_FILTER_ROWS: usize = 3;

impl MipLevel {
    /// Downsample the whole image by area, including odd trailing rows and columns.
    pub(crate) fn halve(rgba: &[[f32; 4]], width: u32, height: u32) -> MipLevel {
        Self::halve_with(rgba, width, height, None::<fn([f32; 4]) -> [f32; 4]>)
    }

    /// Decode stored texels before averaging the first level of an sRGB color chain.
    pub(crate) fn halve_with(rgba: &[[f32; 4]], width: u32, height: u32, decode: Option<impl Fn([f32; 4]) -> [f32; 4] + Copy>) -> MipLevel {
        let (w, h) = ((width / 2).max(1), (height / 2).max(1));
        let mut halved = Vec::with_capacity(w as usize * h as usize);
        if (width <= 1 || width.is_multiple_of(2)) && (height <= 1 || height.is_multiple_of(2)) {
            for y in 0..i64::from(h) {
                for x in 0..i64::from(w) {
                    let mut sum = [0.0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let stored = texel_of(rgba, width, height, x * 2 + dx, y * 2 + dy, (true, true));
                        let texel = decode.map_or(stored, |decode| decode(stored));
                        for (total, value) in sum.iter_mut().zip(texel) {
                            *total += value * TEXEL_SHARE;
                        }
                    }
                    halved.push(sum);
                }
            }
        } else {
            let row_stride = u64::from(width.max(1));
            let read = |row_start, source_x| {
                let index = usize::try_from(row_start + source_x).ok();
                index.and_then(|index| rgba.get(index)).copied().unwrap_or_default()
            };
            let mut decoded_rows = decode.map(|_| std::array::from_fn::<_, AREA_FILTER_ROWS, _>(|_| (u64::MAX, Vec::with_capacity(row_stride as usize))));

            for y in 0..h {
                let source_rows = source_weights(y, height, h);
                if let Some((decode, rows)) = decode.zip(decoded_rows.as_mut()) {
                    // Keep decoded rows for adjacent output intervals that share their texels
                    for (source_y, _) in source_rows.clone() {
                        let (row, texels) = &mut rows[source_y as usize % AREA_FILTER_ROWS];
                        if *row != source_y {
                            let row_start = source_y * row_stride;
                            texels.clear();
                            texels.extend((0..row_stride).map(|source_x| decode(read(row_start, source_x))));
                            *row = source_y;
                        }
                    }
                }

                for x in 0..w {
                    let source_columns = source_weights(x, width, w);
                    let mut sum = [0.0; 4];
                    for (source_y, vertical_weight) in source_rows.clone() {
                        let row_start = source_y * row_stride;
                        let row = decoded_rows.as_ref().map(|rows| &rows[source_y as usize % AREA_FILTER_ROWS].1);
                        for (source_x, horizontal_weight) in source_columns.clone() {
                            let texel = row.map_or_else(|| read(row_start, source_x), |row| row[source_x as usize]);
                            let weight = vertical_weight * horizontal_weight;
                            for (total, value) in sum.iter_mut().zip(texel) {
                                *total += value * weight;
                            }
                        }
                    }
                    halved.push(sum);
                }
            }
        }

        MipLevel { width: w, height: h, rgba: halved }
    }
}

/// Source texels intersecting one output interval and their fractional coverage.
fn source_weights(index: u32, source_size: u32, output_size: u32) -> impl Iterator<Item = (u64, f32)> + Clone {
    let source = u64::from(source_size.max(1));
    let output = u64::from(output_size);
    let inverse_source = 1.0 / source as f32;

    // Scale both intervals to integer boundaries for exact overlap at either edge
    let start = u64::from(index) * source;
    let end = start + source;

    (start / output..end.div_ceil(output)).map(move |texel| {
        let overlap = end.min((texel + 1) * output) - start.max(texel * output);
        (texel, overlap as f32 * inverse_source)
    })
}
