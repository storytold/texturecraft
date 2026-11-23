//! Pixel data owned by a document: painted layer channels, painted mask effects and baked mesh
//! maps. Everything is copy-on-write, so cloning the store for undo is cheap.

use std::collections::HashMap;
use std::sync::OnceLock;

use texturecraft_core::{Channel, ChannelKind, Filtering, LayerId, Sampling};
use texturecraft_texel::{TileImage, from_u16, from_u16_signed, srgb_to_linear};

use crate::mip_level::MipLevel;

/// Painted data of one paint layer channel: premultiplied value(s) plus coverage, u16 per
/// component (height stored signed).
#[derive(Clone, Debug, Default)]
pub struct PixelStore {
    /// (paint layer or content paint effect, channel) → texels.
    pub layers: HashMap<(LayerId, Channel), TileImage>,
    /// Paint effect of a mask → (premultiplied value, coverage).
    pub masks: HashMap<LayerId, TileImage>,
}

impl PixelStore {
    pub fn layer(&self, id: LayerId, c: Channel) -> Option<&TileImage> {
        self.layers.get(&(id, c))
    }

    /// The image for `(id, c)` at `size` (width, height), created empty if missing or of
    /// another size.
    pub fn layer_mut(&mut self, id: LayerId, c: Channel, size: (u32, u32)) -> &mut TileImage {
        sized(self.layers.entry((id, c)).or_insert_with(|| TileImage::new(size.0, size.1, c.components() + 1)), size)
    }

    pub fn mask_mut(&mut self, id: LayerId, size: (u32, u32)) -> &mut TileImage {
        sized(self.masks.entry(id).or_insert_with(|| TileImage::new(size.0, size.1, 2)), size)
    }

    /// Drop pixels of layers that no longer exist.
    pub fn retain(&mut self, alive: impl Fn(LayerId) -> bool) {
        self.layers.retain(|(id, _), _| alive(*id));
        self.masks.retain(|id, _| alive(*id));
    }

    pub fn bytes(&self) -> usize {
        self.layers.values().chain(self.masks.values()).map(TileImage::bytes).sum()
    }
}

/// `img`, replaced by an empty image of its components when it is not `size`.
fn sized(img: &mut TileImage, size: (u32, u32)) -> &mut TileImage {
    if (img.width(), img.height()) != size {
        *img = TileImage::new(size.0, size.1, img.comps());
    }
    img
}

/// Decode a stored component of channel kind `kind` to its float value.
#[inline]
pub fn decode(kind: ChannelKind, v: u16) -> f32 {
    if kind == ChannelKind::Signed { from_u16_signed(v) } else { from_u16(v) }
}

pub use crate::baked_maps::BakedMaps;

/// A decoded image resource sampled by fills (values in the file's encoding; see `srgb`).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRes {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[f32; 4]>,
    /// 8/16-bit images are sRGB-encoded; color channels linearize them.
    pub srgb: bool,
    /// Halved copies of the image, built on first use by [`Filtering::BilinearHq`]
    pub mips: OnceLock<Vec<MipLevel>>,
    /// Mip levels for sRGB color reads. RGB is linear, alpha is unchanged.
    linear_mips: OnceLock<Vec<MipLevel>>,
}

/// Image resources by name.
pub type ImageStore = HashMap<String, std::sync::Arc<ImageRes>>;

/// Image levels below the full image at most: a 64k image halves 16 times to 1 texel
const MAX_MIP_LEVELS: usize = 16;

/// Texel (`x`, `y`) of `rgba`; an axis that does not `repeat` (across, down) stops at its edge
pub(crate) fn texel_of(rgba: &[[f32; 4]], width: u32, height: u32, x: i64, y: i64, repeat: (bool, bool)) -> [f32; 4] {
    let (w, h) = (width.max(1) as i64, height.max(1) as i64);
    let x = if repeat.0 { x.rem_euclid(w) } else { x.clamp(0, w - 1) };
    let y = if repeat.1 { y.rem_euclid(h) } else { y.clamp(0, h - 1) };
    let i = (y * w + x) as usize;
    rgba.get(i).copied().unwrap_or([0.0; 4])
}

/// Bilinear sample of `rgba` (texture coordinates, v down), wrapping around on the axes that `repeat`
fn bilinear(rgba: &[[f32; 4]], width: u32, height: u32, u: f32, v: f32, repeat: (bool, bool)) -> [f32; 4] {
    // Texel centres sit half a texel in from the texel's corner
    let texel_x = u * width as f32 - 0.5;
    let texel_y = v * height as f32 - 0.5;
    let (left, top) = (texel_x.floor() as i64, texel_y.floor() as i64);
    let (across, down) = (texel_x - texel_x.floor(), texel_y - texel_y.floor());

    // A huge coordinate saturates to the last texel instead of overflowing past it
    let (right, bottom) = (left.saturating_add(1), top.saturating_add(1));
    let texel = |x, y| texel_of(rgba, width, height, x, y, repeat);
    let (top_left, top_right) = (texel(left, top), texel(right, top));
    let (bottom_left, bottom_right) = (texel(left, bottom), texel(right, bottom));
    let mut blended = [0.0; 4];
    for k in 0..4 {
        let upper = top_left[k] + (top_right[k] - top_left[k]) * across;
        let lower = bottom_left[k] + (bottom_right[k] - bottom_left[k]) * across;
        blended[k] = upper + (lower - upper) * down;
    }
    blended
}

fn linearize_rgb([red, green, blue, alpha]: [f32; 4]) -> [f32; 4] {
    [srgb_to_linear(red), srgb_to_linear(green), srgb_to_linear(blue), alpha]
}

pub(crate) fn uses_mips(footprint: f32) -> bool {
    footprint.is_finite() && footprint > 1.0
}

impl ImageRes {
    pub fn new(width: u32, height: u32, rgba: Vec<[f32; 4]>, srgb: bool) -> ImageRes {
        ImageRes { width, height, rgba, srgb, mips: OnceLock::new(), linear_mips: OnceLock::new() }
    }

    /// Bytes held by the image's texel buffers, used to budget other image work.
    pub fn retained_texel_bytes(&self) -> u64 {
        let mip_levels = self.mips.get().into_iter().flatten().chain(self.linear_mips.get().into_iter().flatten());
        let capacity = mip_levels.fold(self.rgba.capacity() as u64, |texels, level| texels.saturating_add(level.rgba.capacity() as u64));
        capacity.saturating_mul(std::mem::size_of::<[f32; 4]>() as u64)
    }

    fn texel(&self, x: i64, y: i64, repeat: (bool, bool)) -> [f32; 4] {
        texel_of(&self.rgba, self.width, self.height, x, y, repeat)
    }

    /// Bilinear sample with wrap-around (texture coordinates, v down).
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }

        bilinear(&self.rgba, self.width, self.height, u, v, (true, true))
    }

    /// The texel under (`u`, `v`) as stored, wrapping (an ID map's colors must not blend)
    pub fn sample_nearest(&self, u: f32, v: f32) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }

        self.nearest(u, v, (true, true))
    }

    /// The texel under (`u`, `v`), on the axes that do not `repeat` the edge texel past the edge
    fn nearest(&self, u: f32, v: f32, repeat: (bool, bool)) -> [f32; 4] {
        self.texel((u * self.width as f32).floor() as i64, (v * self.height as f32).floor() as i64, repeat)
    }

    fn build_mips(&self, halve_first: impl FnOnce(&[[f32; 4]], u32, u32) -> MipLevel) -> Vec<MipLevel> {
        if self.width <= 1 && self.height <= 1 {
            return Vec::new();
        }

        // Decode while reducing the first level to avoid a full-size linear image
        let mut level = halve_first(&self.rgba, self.width, self.height);
        let mut levels = Vec::new();
        while (level.width > 1 || level.height > 1) && levels.len() + 1 < MAX_MIP_LEVELS {
            let next = MipLevel::halve(&level.rgba, level.width, level.height);
            levels.push(level);
            level = next;
        }

        levels.push(level);
        levels
    }

    fn mips(&self) -> &[MipLevel] {
        self.mips.get_or_init(|| self.build_mips(MipLevel::halve))
    }

    /// Sample an sRGB image as linear color, preserving the full-resolution filter.
    pub(crate) fn sample_srgb_color(&self, u: f32, v: f32, sampling: Sampling, footprint: f32) -> [f32; 4] {
        self.sample_filtered_with(u, v, sampling, footprint, linearize_rgb, || {
            self.linear_mips.get_or_init(|| self.build_mips(|rgba, width, height| MipLevel::halve_with(rgba, width, height, Some(linearize_rgb))))
        })
    }

    /// Read coverage from either mip chain since the RGB transfer leaves alpha unchanged.
    pub(crate) fn sample_alpha(&self, u: f32, v: f32, sampling: Sampling, footprint: f32) -> f32 {
        self.sample_filtered_with(u, v, sampling, footprint, std::convert::identity, || self.linear_mips.get().map_or_else(|| self.mips(), Vec::as_slice))[3]
    }

    /// Sample at (`u`, `v`) with `sampling`; `footprint` = image texels per surface texel (1 or less = not shrunk)
    pub fn sample_filtered(&self, u: f32, v: f32, sampling: Sampling, footprint: f32) -> [f32; 4] {
        self.sample_filtered_with(u, v, sampling, footprint, std::convert::identity, || self.mips())
    }

    fn sample_filtered_with<'a>(
        &'a self,
        u: f32,
        v: f32,
        sampling: Sampling,
        footprint: f32,
        decode_full: impl Fn([f32; 4]) -> [f32; 4],
        mips: impl FnOnce() -> &'a [MipLevel],
    ) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }

        let repeat = sampling.wrap.repeats();
        let full = || decode_full(bilinear(&self.rgba, self.width, self.height, u, v, repeat));
        match sampling.filtering {
            Filtering::Nearest => decode_full(self.nearest(u, v, repeat)),
            Filtering::BilinearSharp => full(),
            Filtering::BilinearHq => {
                if !uses_mips(footprint) {
                    return full();
                }

                let levels = mips();
                let level_of_detail = footprint.log2().min(levels.len() as f32);
                let finer_level = level_of_detail.floor() as usize;
                let toward_coarser = level_of_detail - finer_level as f32;

                // Level zero uses the file image; the first stored level is half-sized
                let level = |index: usize| {
                    index
                        .checked_sub(1)
                        .and_then(|mip_index| levels.get(mip_index))
                        .map_or_else(full, |mip| bilinear(&mip.rgba, mip.width, mip.height, u, v, repeat))
                };
                let finer = level(finer_level);
                if toward_coarser <= 0.0 {
                    return finer;
                }

                let coarser = level(finer_level + 1);
                [0, 1, 2, 3].map(|component| finer[component] + (coarser[component] - finer[component]) * toward_coarser)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::UvWrap;

    #[test]
    fn a_huge_texture_coordinate_samples_without_overflowing() {
        let image = ImageRes::new(2, 2, vec![[0.5; 4]; 4], false);

        let sampled = image.sample(1e30, -1e30);

        assert_eq!(sampled, [0.5; 4]);
    }

    /// A 4 × 1 image of columns 0, 1, 2, 3
    fn ramp() -> ImageRes {
        ImageRes::new(4, 1, (0..4).map(|column| [column as f32; 4]).collect(), false)
    }

    fn sampling(filtering: Filtering, wrap: UvWrap) -> Sampling {
        Sampling { filtering, wrap, ..Sampling::default() }
    }

    #[test]
    fn nearest_reads_the_texel_under_the_point_and_wraps_or_stops_at_the_edge() {
        let image = ramp();

        let inside = image.sample_filtered(0.6, 0.5, sampling(Filtering::Nearest, UvWrap::Repeat), 1.0);
        let wrapped = image.sample_filtered(1.1, 0.5, sampling(Filtering::Nearest, UvWrap::Repeat), 1.0);
        let stopped = image.sample_filtered(1.1, 0.5, sampling(Filtering::Nearest, UvWrap::None), 1.0);

        assert_eq!(inside, [2.0; 4]);
        assert_eq!(wrapped, [0.0; 4], "past the right edge a repeating image starts again");
        assert_eq!(stopped, [3.0; 4], "past the right edge a single image keeps its last column");
    }

    #[test]
    fn a_bilinear_read_past_a_single_images_edge_blends_no_far_side_texel() {
        let image = ramp();

        let repeating = image.sample_filtered(0.99, 0.5, sampling(Filtering::BilinearSharp, UvWrap::Repeat), 1.0);
        let single = image.sample_filtered(0.99, 0.5, sampling(Filtering::BilinearSharp, UvWrap::None), 1.0);

        assert!(repeating[0] < 3.0, "the far side's 0 blends in: {repeating:?}");
        assert_eq!(single, [3.0; 4]);
    }

    #[test]
    fn high_quality_mips_keep_odd_trailing_rows_and_columns() {
        let mut pixels = vec![[0.0; 4]; 9];
        pixels[8] = [1.0, 0.0, 0.0, 1.0];
        let image = ImageRes::new(3, 3, pixels, false);

        let [red, _, _, alpha] = image.sample_filtered(0.5, 0.5, sampling(Filtering::BilinearHq, UvWrap::None), 8.0);

        assert!((red - 1.0 / 9.0).abs() < 1e-6);
        assert_eq!(red, alpha);
    }

    #[test]
    fn high_quality_bilinear_reads_the_mip_the_footprint_asks_for() {
        let image = ramp();
        let hq = sampling(Filtering::BilinearHq, UvWrap::Repeat);

        let full = image.sample_filtered(0.375, 0.5, hq, 1.0);
        let halved = image.sample_filtered(0.25, 0.5, hq, 2.0);
        let quartered = image.sample_filtered(0.375, 0.5, hq, 4.0);
        let past_the_last = image.sample_filtered(0.375, 0.5, hq, 64.0);
        let between = image.sample_filtered(0.875, 0.5, hq, 2.0_f32.sqrt());

        assert_eq!(full, [1.0; 4], "a footprint of 1 reads the full image at column 1's centre");
        assert_eq!(halved, [0.5; 4], "level 1's first texel averages columns 0 and 1");
        assert_eq!(quartered, [1.5; 4], "a 1-texel level holds the mean");
        assert_eq!(past_the_last, quartered, "a footprint past the last level reads the last one");
        assert!(between[0] > 2.0 && between[0] < 3.0, "half a level blends the full image's 3 and level 1's 2: {between:?}");
    }

    #[test]
    fn an_unshrunk_high_quality_read_builds_no_mips() {
        let image = ramp();

        let full = image.sample_filtered(0.375, 0.5, sampling(Filtering::BilinearHq, UvWrap::Repeat), 1.0);

        assert_eq!(full, [1.0; 4]);
        assert!(image.mips.get().is_none(), "a footprint of 1 reads only the full image");
    }

    #[test]
    fn srgb_mip_transition_blends_linear_color_and_keeps_coverage() {
        let image = ImageRes::new(2, 1, vec![[0.0; 4], [1.0; 4]], true);
        let hq = sampling(Filtering::BilinearHq, UvWrap::None);
        let half_level_footprint = 2.0_f32.sqrt();

        let full = image.sample_srgb_color(0.25, 0.5, hq, 1.0);
        let midpoint = image.sample_srgb_color(0.25, 0.5, hq, half_level_footprint);
        let coverage = image.sample_alpha(0.25, 0.5, hq, half_level_footprint);

        assert_eq!(full, [0.0; 4], "full resolution reads the center of the black texel");
        assert!((midpoint[0] - 0.25).abs() < 1e-6, "mix linear black and the half-bright mip: {midpoint:?}");
        assert_eq!(midpoint[3], coverage);
        assert!(image.mips.get().is_none(), "coverage can use the same mip chain as color");
    }

    #[test]
    fn a_mixed_use_image_reports_the_texels_held_by_both_mip_chains() {
        let image = ImageRes::new(4, 4, vec![[0.5; 4]; 16], true);
        let hq = sampling(Filtering::BilinearHq, UvWrap::None);
        let base = image.retained_texel_bytes();

        image.sample_srgb_color(0.5, 0.5, hq, 4.0);
        let color_only = image.retained_texel_bytes();
        image.sample_filtered(0.5, 0.5, hq, 4.0);
        let mixed = image.retained_texel_bytes();

        assert!(color_only > base);
        assert_eq!(mixed - color_only, color_only - base);
    }
}
