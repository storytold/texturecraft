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

impl MipLevel {
    /// `rgba` halved: each texel averages the 2 × 2 texels it covers (a size of 1 wraps; an odd size above leaves out its last row or column)
    pub(crate) fn halve(rgba: &[[f32; 4]], width: u32, height: u32) -> MipLevel {
        let (w, h) = ((width / 2).max(1), (height / 2).max(1));
        let mut halved = Vec::with_capacity(w as usize * h as usize);
        for y in 0..i64::from(h) {
            for x in 0..i64::from(w) {
                let mut sum = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let texel = texel_of(rgba, width, height, x * 2 + dx, y * 2 + dy, (true, true));
                    for (total, value) in sum.iter_mut().zip(texel) {
                        *total += value * TEXEL_SHARE;
                    }
                }
                halved.push(sum);
            }
        }
        MipLevel { width: w, height: h, rgba: halved }
    }
}
