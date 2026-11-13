//! Pixel data owned by a document: painted layer channels, painted mask effects and baked mesh
//! maps. Everything is copy-on-write, so cloning the store for undo is cheap.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use texturecraft_core::{Channel, ChannelKind, LayerId};
use texturecraft_texel::{TileImage, from_u16, from_u16_signed};

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
}

/// Decodes an image resource from what it was made from (its file bytes, held by the caller's closure).
pub type ImageSource = Arc<dyn Fn() -> Option<ImageRes> + Send + Sync>;

/// One image of an [`ImageStore`]: decoded at the first read when it has a source, or given decoded.
struct ImageSlot {
    decoded: OnceLock<Option<Arc<ImageRes>>>,
    source: Option<ImageSource>,
    /// What the image is made from: the same for as long as its content is (see [`ImageStore::ids`]).
    id: usize,
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
        let id = Arc::as_ptr(&image) as usize;
        let slot = ImageSlot { decoded: OnceLock::from(Some(image)), source: None, id };
        self.slots.insert(name, Arc::new(slot));
    }

    /// Adds image `name`, decoded by `source` when it is first read; `id` names what it is made from (two sources
    /// with the same id decode the same image).
    pub fn insert_source(&mut self, name: String, id: usize, source: ImageSource) {
        self.slots.insert(name, Arc::new(ImageSlot { decoded: OnceLock::new(), source: Some(source), id }));
    }

    /// Whether `name` is in the store, decoded or not.
    pub fn has_slot(&self, name: &str) -> bool {
        self.slots.contains_key(name)
    }

    /// Names and ids of every image, decoded or not, without decoding any: what a cache keyed by the images' contents
    /// compares.
    pub fn ids(&self) -> impl Iterator<Item = (&String, usize)> {
        self.slots.iter().map(|(n, s)| (n, s.id))
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
        self.slots.values().filter_map(|s| s.decoded.get().cloned().flatten()).map(|i| i.rgba.len() * std::mem::size_of::<[f32; 4]>()).sum()
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
                *slot = Arc::new(ImageSlot { decoded: OnceLock::new(), source: slot.source.clone(), id: slot.id });
                gone += 1;
            }
        }
        gone
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

impl ImageRes {
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let (w, h) = (self.width.max(1) as i64, self.height.max(1) as i64);
        let i = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize;
        self.rgba.get(i).copied().unwrap_or([0.0; 4])
    }

    /// Bilinear sample with wrap-around (texture coordinates, v down).
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let (a, b, c, d) = (self.texel(x0, y0), self.texel(x0 + 1, y0), self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            out[k] = top + (bot - top) * ty;
        }
        out
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
        let (w, h) = (i64::from(self.width.max(1)), i64::from(self.height.max(1)));
        let at = |x: i64, y: i64| self.texel(if wrap[0] { x } else { x.clamp(0, w - 1) }, if wrap[1] { y } else { y.clamp(0, h - 1) });
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
        Some(std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            top + (bot - top) * ty
        }))
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
            Some(ImageRes { width: 2, height: 1, rgba: vec![[0.25, 0.5, 0.75, 1.0], [1.0; 4]], srgb: true })
        })
    }

    #[test]
    fn an_image_is_decoded_when_first_read_and_again_after_it_is_let_go() {
        let decodes = Arc::new(AtomicUsize::new(0));
        let mut store = ImageStore::default();
        store.insert_source("a.png".into(), 7, counting_source(&decodes));
        store.insert_source("broken.png".into(), 8, Arc::new(|| None));
        assert!(store.has_slot("a.png") && store.ids().any(|(n, id)| n == "a.png" && id == 7));
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
        assert!(store.ids().any(|(n, id)| n == "a.png" && id == 7), "the image's id stays");
        assert_eq!(**store.get("a.png").unwrap(), *first, "decoded again, the same image");
        assert_eq!(decodes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn an_image_given_decoded_stays() {
        let image = Arc::new(ImageRes { width: 1, height: 1, rgba: vec![[1.0; 4]], srgb: false });
        let mut store: ImageStore = [("x.exr".to_string(), image.clone())].into_iter().collect();
        assert_eq!(store.release(0), 0, "it cannot be decoded again");
        assert!(Arc::ptr_eq(store.get("x.exr").unwrap(), &image));
        store.retain_names(|n| n != "x.exr");
        assert!(store.is_empty());
    }
}
