//! Pixel data owned by a document: painted layer channels, painted mask effects and baked mesh
//! maps. Everything is copy-on-write, so cloning the store for undo is cheap.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

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

pub use crate::baked_maps::{BakedMaps, Texels};

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

/// Decodes an image resource from what it was made from (its file bytes, held by the caller's closure).
pub type ImageSource = Arc<dyn Fn() -> Option<ImageRes> + Send + Sync>;

/// What an image is made from (its file bytes, or the decoded image itself), held by allocation: it stays the same
/// while the content does, whether the image is decoded or released, and holding it keeps no bytes or pixels alive.
pub type ImageOrigin = Weak<dyn Any + Send + Sync>;

/// One image of an [`ImageStore`]: decoded at the first read when it has a source, or given decoded.
struct ImageSlot {
    decoded: OnceLock<Option<Arc<ImageRes>>>,
    source: Option<ImageSource>,
    /// What the image is made from: the same for as long as its content is (see [`ImageStore::origin`]).
    origin: ImageOrigin,
}

impl ImageSlot {
    fn get(&self) -> Option<&Arc<ImageRes>> {
        self.decoded.get_or_init(|| self.source.as_ref().and_then(|decode| decode()).map(Arc::new)).as_ref()
    }
}

/// Image resources by name. An image added with [`ImageStore::insert_source`] is decoded when something first reads
/// it, and [`ImageStore::release`] lets go of the decoded images that can be decoded again: a project with many large
/// images (one per texture set, say) need not hold them all. Clones share the images, decoded or not.
#[derive(Clone, Default)]
pub struct ImageStore {
    slots: HashMap<String, Arc<ImageSlot>>,
}

impl std::fmt::Debug for ImageStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.slots.keys()).finish()
    }
}

impl ImageStore {
    /// The image `name`, decoded now if it is not yet (`None` when there is none or it cannot be decoded).
    pub fn get(&self, name: &str) -> Option<&Arc<ImageRes>> {
        self.slots.get(name)?.get()
    }

    /// Whether there is an image `name` that decodes.
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Adds the decoded image `image` under `name`, in place of any other.
    pub fn insert(&mut self, name: String, image: Arc<ImageRes>) {
        let decoded: Arc<dyn Any + Send + Sync> = image.clone();
        let origin = Arc::downgrade(&decoded);
        let slot = ImageSlot { decoded: OnceLock::from(Some(image)), source: None, origin };
        self.slots.insert(name, Arc::new(slot));
    }

    /// Adds image `name`, decoded by `source` when it is first read; `origin` is what it is made from (two sources
    /// with the same origin decode the same image). The source should hold `origin` alive.
    pub fn insert_source(&mut self, name: String, origin: ImageOrigin, source: ImageSource) {
        self.slots.insert(name, Arc::new(ImageSlot { decoded: OnceLock::new(), source: Some(source), origin }));
    }

    /// Whether `name` is in the store, decoded or not.
    pub fn has_slot(&self, name: &str) -> bool {
        self.slots.contains_key(name)
    }

    /// What image `name` is made from, decoded or not, without decoding it: what a cache keyed by the image's content
    /// compares (with [`Weak::ptr_eq`]).
    pub fn origin(&self, name: &str) -> Option<(&String, ImageOrigin)> {
        self.slots.get_key_value(name).map(|(n, s)| (n, s.origin.clone()))
    }

    /// Whether `other` holds the very same images under the same names (a clone does, until either changes or lets
    /// go of an image), without decoding any.
    pub fn same_as(&self, other: &ImageStore) -> bool {
        self.slots.len() == other.slots.len() && self.slots.iter().all(|(n, s)| other.slots.get(n).is_some_and(|o| Arc::ptr_eq(s, o)))
    }

    /// Forgets the images `keep` does not name, decoded or not.
    pub fn retain_names(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.slots.retain(|n, _| keep(n));
    }

    pub fn remove(&mut self, name: &str) -> Option<Arc<ImageRes>> {
        self.slots.remove(name).and_then(|s| s.get().cloned())
    }

    /// The images that decode, by name (decoding those that are not yet).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Arc<ImageRes>)> {
        self.slots.iter().filter_map(|(n, s)| Some((n, s.get()?)))
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.iter().map(|(n, _)| n)
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<ImageRes>> {
        self.iter().map(|(_, i)| i)
    }

    /// How many images decode.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes of the decoded images held.
    pub fn decoded_bytes(&self) -> usize {
        self.slots.values().filter_map(|s| s.decoded.get().cloned().flatten()).map(|i| i.retained_texel_bytes() as usize).sum()
    }

    /// Lets go of the decoded images that have a source (they are decoded again when they are next read), once those
    /// held take more than `limit` bytes. Returns how many went.
    pub fn release(&mut self, limit: usize) -> usize {
        if self.decoded_bytes() <= limit {
            return 0;
        }
        let mut gone = 0;
        for slot in self.slots.values_mut() {
            if slot.source.is_some() && matches!(slot.decoded.get(), Some(Some(_))) {
                *slot = Arc::new(ImageSlot { decoded: OnceLock::new(), source: slot.source.clone(), origin: slot.origin.clone() });
                gone += 1;
            }
        }
        gone
    }
}

impl<const N: usize> From<[(String, Arc<ImageRes>); N]> for ImageStore {
    fn from(images: [(String, Arc<ImageRes>); N]) -> Self {
        images.into_iter().collect()
    }
}

impl FromIterator<(String, Arc<ImageRes>)> for ImageStore {
    fn from_iter<I: IntoIterator<Item = (String, Arc<ImageRes>)>>(iter: I) -> ImageStore {
        let mut store = ImageStore::default();
        store.extend(iter);
        store
    }
}

impl Extend<(String, Arc<ImageRes>)> for ImageStore {
    fn extend<I: IntoIterator<Item = (String, Arc<ImageRes>)>>(&mut self, iter: I) {
        for (name, image) in iter {
            self.insert(name, image);
        }
    }
}

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

    /// The halved levels Bilinear HQ reads (level 1 first; level 0 is the image), built once.
    /// The GPU compositor uploads the same levels.
    pub fn mip_chain(&self) -> &[MipLevel] {
        self.mips()
    }

    /// The halved levels of an sRGB image's colour read: RGB linear, alpha unchanged (the chain
    /// `sample_srgb_color` reads), built once.
    pub fn linear_mip_chain(&self) -> &[MipLevel] {
        self.linear_mips.get_or_init(|| self.build_mips(|rgba, width, height| MipLevel::halve_with(rgba, width, height, Some(linearize_rgb))))
    }

    /// Sample an sRGB image as linear color, preserving the full-resolution filter.
    pub(crate) fn sample_srgb_color(&self, u: f32, v: f32, sampling: Sampling, footprint: f32) -> [f32; 4] {
        self.sample_filtered_with(u, v, sampling, footprint, linearize_rgb, || self.linear_mip_chain())
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

    /// [`ImageRes::sample`] repeating only along the axes `wrap` names (u, v): along the others the edge texels extend
    /// and coordinates outside 0..1 have no value (`None`).
    pub fn sample_wrapped(&self, u: f32, v: f32, wrap: [bool; 2]) -> Option<[f32; 4]> {
        if wrap == [true, true] {
            return Some(self.sample(u, v));
        }
        if !u.is_finite() || !v.is_finite() || !(wrap[0] || (0.0..=1.0).contains(&u)) || !(wrap[1] || (0.0..=1.0).contains(&v)) {
            return None;
        }

        Some(bilinear(&self.rgba, self.width, self.height, u, v, (wrap[0], wrap[1])))
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
    fn the_public_mip_chains_are_the_levels_bilinear_hq_reads() {
        let image = ImageRes::new(8, 8, (0..64).map(|i| [i as f32 / 63.0, 0.25, 0.75, 1.0]).collect(), false);
        let sampling = Sampling { filtering: Filtering::BilinearHq, ..Sampling::SAVED_BEFORE };
        // Footprint 2: exactly the first halved level
        let level = &image.mip_chain()[0];
        assert_eq!(image.sample_filtered(0.3, 0.6, sampling, 2.0), bilinear(&level.rgba, level.width, level.height, 0.3, 0.6, (true, true)));
        let srgb = ImageRes::new(8, 8, image.rgba.clone(), true);
        let level = &srgb.linear_mip_chain()[0];
        assert_eq!(srgb.sample_srgb_color(0.3, 0.6, sampling, 2.0), bilinear(&level.rgba, level.width, level.height, 0.3, 0.6, (true, true)));
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

#[cfg(test)]
mod image_store_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn counting_source(decodes: &Arc<AtomicUsize>) -> ImageSource {
        let decodes = decodes.clone();
        Arc::new(move || {
            decodes.fetch_add(1, Ordering::Relaxed);
            Some(ImageRes::new(2, 1, vec![[0.25, 0.5, 0.75, 1.0], [1.0; 4]], true))
        })
    }

    #[test]
    fn an_image_is_decoded_when_first_read_and_again_after_it_is_let_go() {
        let decodes = Arc::new(AtomicUsize::new(0));
        let mut store = ImageStore::default();
        let (bytes, broken): (Arc<dyn Any + Send + Sync>, Arc<dyn Any + Send + Sync>) = (Arc::new(vec![7u8]), Arc::new(vec![8u8]));
        store.insert_source("a.png".into(), Arc::downgrade(&bytes), counting_source(&decodes));
        store.insert_source("broken.png".into(), Arc::downgrade(&broken), Arc::new(|| None));
        let is_a = |store: &ImageStore| store.origin("a.png").is_some_and(|(n, origin)| n == "a.png" && origin.ptr_eq(&Arc::downgrade(&bytes)));
        assert!(store.has_slot("a.png") && is_a(&store));
        assert_eq!((decodes.load(Ordering::Relaxed), store.decoded_bytes()), (0, 0), "nothing is decoded before it is read");

        let first = store.get("a.png").cloned().unwrap();
        let shared = store.clone();
        assert!(Arc::ptr_eq(shared.get("a.png").unwrap(), &first), "clones share what is decoded");
        assert!(shared.same_as(&store));
        assert_eq!((decodes.load(Ordering::Relaxed), store.decoded_bytes()), (1, 32));
        assert!(store.get("broken.png").is_none() && !store.contains_key("broken.png") && store.has_slot("broken.png"));
        assert_eq!(store.len(), 1, "an image that does not decode is not counted");

        assert_eq!(store.release(1 << 20), 0, "within the limit nothing goes");
        assert_eq!(store.release(0), 1);
        assert_eq!(store.decoded_bytes(), 0);
        assert!(!shared.same_as(&store), "a clone keeps what it decoded");
        assert!(is_a(&store), "the image's origin stays");
        assert_eq!(**store.get("a.png").unwrap(), *first, "decoded again, the same image");
        assert_eq!(decodes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn an_image_given_decoded_stays() {
        let image = Arc::new(ImageRes::new(1, 1, vec![[1.0; 4]], false));
        let mut store: ImageStore = [("x.exr".to_string(), image.clone())].into_iter().collect();
        assert_eq!(store.release(0), 0, "it cannot be decoded again");
        assert!(Arc::ptr_eq(store.get("x.exr").unwrap(), &image));
        store.retain_names(|n| n != "x.exr");
        assert!(store.is_empty());
    }
}
