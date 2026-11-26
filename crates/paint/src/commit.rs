//! Committing a stroke: premultiplied "over" (paint) or scale-down (erase) of the stroke alpha
//! into a layer's channels or a mask's paint effect. Only touched tiles are un-shared, so the
//! previous state stays intact in undo snapshots. A [`Staging`] does most of that work while
//! the stroke is drawn, so the commit itself is quick even at 8K.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Weak};

use rayon::prelude::*;
use texturecraft_compose::{ImageRes, MaskGray, PixelStore};
use texturecraft_core::{Channel, ChannelKind, LayerId};
use texturecraft_texel::{TILE, TILE_AREA, TileData, TileImage, from_u16, from_u16_signed, to_u16, to_u16_signed};

pub(crate) fn dec(kind: ChannelKind, v: u16) -> f32 {
    if kind == ChannelKind::Signed { from_u16_signed(v) } else { from_u16(v) }
}

pub(crate) fn enc(kind: ChannelKind, v: f32) -> u16 {
    if kind == ChannelKind::Signed { to_u16_signed(v) } else { to_u16(v) }
}

/// What one image receives: its channel kind, the painted value (unused when erasing or for coverage images), whether a stencil's color replaces it (base color) and the values a clone or smudge stroke carries for it, and the image whose gray a mask stroke paints in place of `value`
struct Blend<'a> {
    kind: ChannelKind,
    value: &'a [f32],
    stencil_color: bool,
    carried: Option<&'a TileImage>,
    gray: Option<&'a MaskGray>,
}

/// Blend `alpha × opacity` into one image of `n` value components plus coverage. With a
/// per-texel source, its alpha factor scales the stroke and, when `stencil_color`, its color
/// replaces `value`; `carried` (clone, smudge) replaces `value` per texel. `staged` holds the
/// destination tiles prepared while the stroke was drawn.
fn over(img: &mut TileImage, b: &Blend, src: &CommitSource, mut staged: Option<StagedImage>) {
    let comps = img.comps();
    let empty = empty_texel(b.kind, comps);
    let key = BlendKey::new(b, src);

    // Compute replacement tiles in parallel while undo snapshots share the old tiles
    let image = &*img;
    let tiles: Vec<_> = src.tiles.iter().map(|&t| (t, staged.as_mut().and_then(|s| s.take(image, src.alpha, &key, &empty, t)))).collect();
    let updated: Vec<((u32, u32), Vec<u16>)> = tiles
        .into_par_iter()
        .filter_map(|((tx, ty), staged)| {
            let a = src.alpha.tile(tx, ty)?;
            if let Some((data, true)) = staged {
                return Some(((tx, ty), data));
            }

            if src.erase && !image.is_allocated(tx, ty) {
                return None;
            }

            let carried = match b.carried.map(|c| c.tile(tx, ty)) {
                Some(None) => return None,
                Some(Some(t)) => Some(t),
                None => None,
            };
            let mut dst = match (staged, image.tile(tx, ty)) {
                (Some((d, _)), _) => d,
                (None, Some(t)) => t.to_vec(),
                (None, None) => empty.repeat(TILE_AREA),
            };
            blend_tile(&mut dst, comps, a, (tx, ty), b, src, carried);
            Some(((tx, ty), dst))
        })
        .collect();
    for ((tx, ty), data) in updated {
        img.set_tile(tx, ty, Some(Arc::new(data)));
    }
}

/// Blend the stroke's alpha tile `stroke_alpha` into tile `(tx, ty)` of an image (`dst`, `components` per texel)
fn blend_tile(dst: &mut [u16], components: usize, stroke_alpha: &[u16], (tx, ty): (u32, u32), blend: &Blend, source: &CommitSource, carried: Option<&[u16]>) {
    // The last component is the coverage, the ones before it the value
    let value_count = components.saturating_sub(1);
    let opacity = source.opacity.clamp(0.0, 1.0);
    let gray_source = if source.erase || carried.is_some() { None } else { blend.gray };

    for i in 0..TILE_AREA {
        let mut weight = from_u16(stroke_alpha.get(i).copied().unwrap_or(0)) * opacity;
        if weight <= 0.0 {
            continue;
        }

        let mut stencil = None;
        let (x, y) = (tx * TILE + i as u32 % TILE, ty * TILE + i as u32 / TILE);
        if let Some(per_texel) = source.per_texel {
            match per_texel(x, y) {
                Some((color, alpha_factor)) => {
                    weight *= alpha_factor;
                    stencil = Some(color);
                }
                None => continue,
            }
            if weight <= 0.0 {
                continue;
            }
        }

        let Some((coverage_slot, values)) = dst.get_mut(i * components..(i + 1) * components).and_then(<[u16]>::split_last_mut) else { continue };
        let gray = gray_source.map(|gray| gray.at(x, y, (source.alpha.width(), source.alpha.height())));
        let coverage = from_u16(*coverage_slot);
        for (component, slot) in values.iter_mut().enumerate() {
            let previous = dec(blend.kind, *slot);
            let target = match (carried, stencil) {
                (Some(carried_tile), _) => dec(blend.kind, carried_tile.get(i * value_count + component).copied().unwrap_or(0)),
                (None, _) if let Some(gray) = gray => gray,
                (None, Some(color)) if blend.stencil_color => color.get(component).copied().unwrap_or(0.0),
                _ => blend.value.get(component).copied().unwrap_or(0.0),
            };
            let blended = if source.erase { previous * (1.0 - weight) } else { target * weight + previous * (1.0 - weight) };
            *slot = enc(blend.kind, blended);
        }
        *coverage_slot = to_u16(if source.erase { coverage * (1.0 - weight) } else { weight + coverage * (1.0 - weight) });
    }
}

/// One empty texel of an image with `comps` components: zero coverage and a zero value (32768
/// in signed encoding). New tiles start as this.
fn empty_texel(kind: ChannelKind, comps: usize) -> Vec<u16> {
    let mut t = vec![enc(kind, 0.0); comps.saturating_sub(1)];
    t.push(0);
    t
}

/// `dst` as a copy of tile `from` (None: an empty tile of `empty` texels), reusing its memory.
fn fill_from(dst: &mut Vec<u16>, from: Option<&[u16]>, empty: &[u16]) {
    dst.clear();
    match from {
        Some(t) => dst.extend_from_slice(t),
        None => {
            for _ in 0..TILE_AREA {
                dst.extend_from_slice(empty);
            }
        }
    }
}

/// What a pre-blended tile was blended with, besides the stroke alpha.
#[derive(Debug)]
struct BlendKey {
    value: Vec<f32>,
    image: Option<(Weak<ImageRes>, (u32, u32))>,
    opacity: f32,
    erase: bool,
}

impl BlendKey {
    fn new(blend: &Blend, source: &CommitSource) -> Self {
        let image = match blend.gray {
            Some(MaskGray::Image { image, .. }) if !source.erase => Some((Arc::downgrade(image), (source.alpha.width(), source.alpha.height()))),
            _ => None,
        };
        Self { value: blend.value.to_vec(), image, opacity: source.opacity, erase: source.erase }
    }
}

impl PartialEq for BlendKey {
    fn eq(&self, other: &Self) -> bool {
        let same_image = match (&self.image, &other.image) {
            (Some((image, size)), Some((other_image, other_size))) => Weak::ptr_eq(image, other_image) && size == other_size,
            (None, None) => true,
            _ => false,
        };
        same_image && self.value == other.value && self.opacity == other.opacity && self.erase == other.erase
    }
}

/// One destination tile prepared while the stroke is drawn.
#[derive(Debug)]
struct StagedTile {
    /// The layer tile it was copied from (None: the layer had no tile there).
    from: Option<TileData>,
    /// The stroke alpha tile it is blended with (None: a plain copy of `from`).
    alpha: Option<TileData>,
    data: Vec<u16>,
}

/// The staged tiles of one image and what pre-blended ones were blended with.
#[derive(Debug, Default)]
struct StagedImage {
    key: Option<BlendKey>,
    tiles: HashMap<(u32, u32), StagedTile>,
}

impl StagedImage {
    /// The staged tile `t` for a commit with `key` and the stroke `alpha`, when it still
    /// matches `image` (the document can change while a stroke is drawn, e.g. through an
    /// agent's command): `(data, true)` when it is final, `(copy of the image tile, false)`
    /// when it still needs blending.
    fn take(&mut self, image: &TileImage, alpha: &TileImage, key: &BlendKey, empty: &[u16], t: (u32, u32)) -> Option<(Vec<u16>, bool)> {
        let mut e = self.tiles.remove(&t)?;
        let current = image.tile_handle(t.0, t.1);
        let same = |a: Option<&TileData>, b: Option<&TileData>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if !same(e.from.as_ref(), current) || e.data.len() != TILE_AREA * image.comps() {
            return None;
        }

        if e.alpha.is_some() && self.key.as_ref() == Some(key) && same(e.alpha.as_ref(), alpha.tile_handle(t.0, t.1)) {
            return Some((e.data, true));
        }

        if e.alpha.is_some() {
            // Discard blends for superseded stroke alpha
            fill_from(&mut e.data, e.from.as_deref().map(Vec::as_slice), empty);
        }
        Some((e.data, false))
    }

    /// Prepare the tiles `src.tiles` of `img` (re-blending those staged before): a copy of the
    /// image tile, blended with the stroke when it has no stencil or carried values
    /// (those blend at the commit)
    fn stage(&mut self, img: Option<&TileImage>, comps: usize, b: &Blend, src: &CommitSource) {
        let key = BlendKey::new(b, src);
        if self.key.as_ref() != Some(&key) {
            // A changed dependency invalidates the staged blend
            self.tiles.clear();
            self.key = Some(key);
        }
        let blend = src.per_texel.is_none() && b.carried.is_none();
        let empty = empty_texel(b.kind, comps);
        let todo: Vec<((u32, u32), Option<StagedTile>)> = src.tiles.iter().map(|t| (*t, self.tiles.remove(t))).collect();
        let staged: Vec<((u32, u32), StagedTile)> = todo
            .into_par_iter()
            .filter_map(|((tx, ty), old)| {
                let from = img.and_then(|i| i.tile_handle(tx, ty));

                // Erasing can only change an existing tile
                if from.is_none() && src.erase {
                    return None;
                }

                let mut data = old.map(|o| o.data).unwrap_or_default();
                fill_from(&mut data, from.map(|f| f.as_slice()), &empty);
                let alpha = src.alpha.tile_handle(tx, ty).filter(|_| blend).cloned();
                if let Some(a) = &alpha {
                    blend_tile(&mut data, comps, a, (tx, ty), b, src, None);
                }
                Some(((tx, ty), StagedTile { from: from.cloned(), alpha, data }))
            })
            .collect();
        self.tiles.extend(staged);
    }
}

/// Destination tiles prepared while a stroke is drawn, so the commit only swaps them in. At 8K
/// a commit of 1300 tiles of five channels spends ~45 ms on fresh memory alone; staged as the
/// stroke reaches the tiles (and re-blended when it passes again) it costs a few milliseconds.
#[derive(Debug, Default)]
pub struct Staging {
    /// By layer channel (None: a mask paint effect).
    images: BTreeMap<Option<Channel>, StagedImage>,
}

impl Staging {
    /// Prepare tiles `src.tiles` of every image a [`commit_layer`] of `layer` with the same
    /// arguments will write (call after the stroke alpha of those tiles changed).
    pub fn stage_layer(&mut self, pixels: &PixelStore, layer: LayerId, values: &BTreeMap<Channel, [f32; 3]>, src: &CommitSource) {
        for c in commit_channels(pixels, layer, values, src.erase) {
            let b = layer_blend(c, values, src);
            self.images.entry(Some(c)).or_default().stage(pixels.layer(layer, c), c.components() + 1, &b, src);
        }
    }

    /// Prepare tiles `src.tiles` of the mask paint effect a [`commit_mask`] of `effect` with the same arguments will write
    pub fn stage_mask(&mut self, pixels: &PixelStore, effect: LayerId, gray: &MaskGray, src: &CommitSource) {
        let value = mask_value(gray);
        let b = Blend { carried: src.carried_mask, ..mask_blend(gray, &value) };
        self.images.entry(None).or_default().stage(pixels.masks.get(&effect), 2, &b, src);
    }

    fn take(&mut self, key: Option<Channel>) -> Option<StagedImage> {
        self.images.remove(&key)
    }
}

/// The channels a layer commit writes: the painted ones, or every painted channel of the layer
/// when erasing (only those in `values` when it is not empty).
fn commit_channels(pixels: &PixelStore, layer: LayerId, values: &BTreeMap<Channel, [f32; 3]>, erase: bool) -> Vec<Channel> {
    if erase {
        let painted: BTreeSet<Channel> = pixels.layers.keys().filter(|(id, _)| *id == layer).map(|(_, c)| *c).collect();
        painted.into_iter().filter(|c| values.is_empty() || values.contains_key(c)).collect()
    } else {
        values.keys().copied().collect()
    }
}

/// How channel `c` of a layer commit blends.
fn layer_blend<'a>(c: Channel, values: &'a BTreeMap<Channel, [f32; 3]>, src: &CommitSource<'a>) -> Blend<'a> {
    static ZERO: [f32; 3] = [0.0; 3];
    let value = values.get(&c).map_or(&ZERO[..], |v| &v[..]);
    Blend {
        kind: c.kind(),
        value: value.get(..c.components()).unwrap_or(value),
        stencil_color: c == Channel::BaseColor,
        carried: src.carried.and_then(|m| m.get(&c)),
        gray: None,
    }
}

/// The one value a mask stroke painting `gray` blends (0 under an image, which gives its own per texel)
fn mask_value(gray: &MaskGray) -> [f32; 1] {
    match gray {
        MaskGray::Value(value) => [value.clamp(0.0, 1.0)],
        MaskGray::Image { .. } => [0.0],
    }
}

/// How a mask stroke painting `gray` blends (`value` from [`mask_value`])
fn mask_blend<'a>(gray: &'a MaskGray, value: &'a [f32; 1]) -> Blend<'a> {
    let gray = matches!(gray, MaskGray::Image { .. }).then_some(gray);
    Blend { kind: ChannelKind::Scalar, value, stencil_color: false, carried: None, gray }
}

/// Per-texel override of a stroke: `(x, y)` → (base color, alpha factor), None = no paint.
pub type TexelFn<'a> = &'a (dyn Fn(u32, u32) -> Option<([f32; 3], f32)> + Sync);

/// The stroke being committed: its alpha, touched tiles, opacity cap and mode.
#[derive(Clone, Copy)]
pub struct CommitSource<'a> {
    pub alpha: &'a TileImage,
    pub tiles: &'a BTreeSet<(u32, u32)>,
    pub opacity: f32,
    pub erase: bool,
    /// Projection tool: base color and alpha per texel from a stencil.
    pub per_texel: Option<TexelFn<'a>>,
    /// Clone and smudge: straight values per channel carried by the stroke (replace `values`).
    pub carried: Option<&'a BTreeMap<Channel, TileImage>>,
    /// Scalar values sampled from a mask by clone or smudge.
    pub carried_mask: Option<&'a TileImage>,
}

/// Commit a stroke into paint layer `layer` (channels in `values`; erase clears every painted
/// channel of the layer, or only those in `values` when it is not empty). `staging` holds the
/// tiles prepared while the stroke was drawn (empty for one-shot fills).
pub fn commit_layer(pixels: &mut PixelStore, layer: LayerId, size: (u32, u32), values: &BTreeMap<Channel, [f32; 3]>, src: CommitSource, staging: &mut Staging) {
    let channels = commit_channels(pixels, layer, values, src.erase);
    for &c in &channels {
        pixels.layer_mut(layer, c, size);
    }

    // The channels blend in parallel too: an 8K stroke touches hundreds of tiles per channel
    let images: Vec<(Channel, &mut TileImage, Option<StagedImage>)> =
        pixels.layers.iter_mut().filter(|((id, c), _)| *id == layer && channels.contains(c)).map(|((_, c), img)| (*c, img, staging.take(Some(*c)))).collect();
    images.into_par_iter().for_each(|(c, img, staged)| over(img, &layer_blend(c, values, &src), &src, staged));
}

/// Commit a stroke into the paint effect `effect` of a mask, painting `gray`
pub fn commit_mask(pixels: &mut PixelStore, effect: LayerId, size: (u32, u32), gray: &MaskGray, src: CommitSource, staging: &mut Staging) {
    let img = pixels.mask_mut(effect, size);
    let value = mask_value(gray);
    over(img, &Blend { carried: src.carried_mask, ..mask_blend(gray, &value) }, &src, staging.take(None));
}

/// Commit a stroke into a coverage-only image (1 component), e.g. the quick mask
pub fn commit_coverage(img: &mut TileImage, src: CommitSource) {
    over(img, &Blend { kind: ChannelKind::Scalar, value: &[], stencil_color: false, carried: None, gray: None }, &src, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src<'a>(alpha: &'a TileImage, tiles: &'a BTreeSet<(u32, u32)>, opacity: f32, erase: bool) -> CommitSource<'a> {
        CommitSource { alpha, tiles, opacity, erase, per_texel: None, carried: None, carried_mask: None }
    }

    fn alpha(v: f32) -> (TileImage, BTreeSet<(u32, u32)>) {
        let mut a = TileImage::new(64, 64, 1);
        a.set(5, 5, &[to_u16(v)]);
        (a, [(0, 0)].into_iter().collect())
    }

    #[test]
    fn paint_then_paint_then_erase_is_premultiplied_over() {
        let mut px = PixelStore::default();
        let id = LayerId(1);
        let vals: BTreeMap<Channel, [f32; 3]> = [(Channel::Roughness, [0.8; 3])].into_iter().collect();
        let (a, tiles) = alpha(0.5);
        commit_layer(&mut px, id, (64, 64), &vals, src(&a, &tiles, 1.0, false), &mut Staging::default());
        let t = px.layer(id, Channel::Roughness).unwrap().get(5, 5);
        assert!((from_u16(t[0]) - 0.4).abs() < 1e-3, "premul v·a");
        assert!((from_u16(t[1]) - 0.5).abs() < 1e-3, "coverage a");
        commit_layer(&mut px, id, (64, 64), &vals, src(&a, &tiles, 1.0, false), &mut Staging::default());
        let t = px.layer(id, Channel::Roughness).unwrap().get(5, 5);
        assert!((from_u16(t[1]) - 0.75).abs() < 1e-3);
        assert!((from_u16(t[0]) - 0.6).abs() < 1e-3);
        commit_layer(&mut px, id, (64, 64), &BTreeMap::new(), src(&a, &tiles, 1.0, true), &mut Staging::default());
        let t = px.layer(id, Channel::Roughness).unwrap().get(5, 5);
        assert!((from_u16(t[1]) - 0.375).abs() < 1e-3);
        assert!((from_u16(t[0]) - 0.3).abs() < 1e-3);
    }

    #[test]
    fn signed_height_and_stroke_opacity() {
        let mut px = PixelStore::default();
        let id = LayerId(2);
        let vals: BTreeMap<Channel, [f32; 3]> = [(Channel::Height, [-0.5; 3])].into_iter().collect();
        let (a, tiles) = alpha(1.0);
        commit_layer(&mut px, id, (64, 64), &vals, src(&a, &tiles, 0.5, false), &mut Staging::default());
        let t = px.layer(id, Channel::Height).unwrap().get(5, 5);
        assert!((from_u16_signed(t[0]) + 0.25).abs() < 1e-3);
        assert!((from_u16(t[1]) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn mask_commit_and_untouched_tiles_stay_shared() {
        let mut px = PixelStore::default();
        let (a, tiles) = alpha(1.0);
        commit_mask(&mut px, LayerId(9), (64, 64), &MaskGray::Value(1.0), src(&a, &tiles, 1.0, false), &mut Staging::default());
        let before = px.clone();
        commit_mask(&mut px, LayerId(9), (64, 64), &MaskGray::Value(0.0), src(&a, &tiles, 1.0, true), &mut Staging::default());

        assert_eq!(from_u16(before.masks[&LayerId(9)].get(5, 5)[1]), 1.0);
        assert_eq!(px.masks[&LayerId(9)].get(5, 5)[1], 0);
    }

    #[test]
    fn carried_values_replace_the_tool_value_and_coverage_images_work() {
        let mut px = PixelStore::default();
        let (a, tiles) = alpha(1.0);
        let mut carried = TileImage::new(64, 64, 1);
        carried.set(5, 5, &[to_u16(0.25)]);
        let carried: BTreeMap<Channel, TileImage> = [(Channel::Roughness, carried)].into_iter().collect();
        let vals: BTreeMap<Channel, [f32; 3]> = [(Channel::Roughness, [0.8; 3])].into_iter().collect();
        commit_layer(&mut px, LayerId(3), (64, 64), &vals, CommitSource { carried: Some(&carried), ..src(&a, &tiles, 1.0, false) }, &mut Staging::default());
        assert!((from_u16(px.layer(LayerId(3), Channel::Roughness).unwrap().get(5, 5)[0]) - 0.25).abs() < 1e-3);
        let mut cov = TileImage::new(64, 64, 1);
        commit_coverage(&mut cov, src(&a, &tiles, 0.5, false));
        assert!((from_u16(cov.get(5, 5)[0]) - 0.5).abs() < 1e-3);
    }

    fn source<'a>(a: &'a TileImage, tiles: &'a BTreeSet<(u32, u32)>, carried: Option<&'a BTreeMap<Channel, TileImage>>, erase: bool) -> CommitSource<'a> {
        CommitSource { carried, ..src(a, tiles, 0.8, erase) }
    }

    #[test]
    fn layer_roughness_does_not_override_mask_coverage() {
        let (alpha, tiles) = alpha(1.0);
        let mut roughness = TileImage::new(64, 64, 1);
        roughness.set(5, 5, &[to_u16(0.25)]);
        let carried = [(Channel::Roughness, roughness)].into();
        let source = CommitSource { carried: Some(&carried), ..src(&alpha, &tiles, 1.0, false) };
        let mut pixels = PixelStore::default();

        commit_mask(&mut pixels, LayerId(1), (64, 64), &MaskGray::Value(1.0), source, &mut Staging::default());

        assert_eq!(pixels.masks[&LayerId(1)].get(5, 5)[0], u16::MAX);
    }

    #[test]
    fn staged_commits_match_and_stale_copies_are_ignored() {
        let vals: BTreeMap<Channel, [f32; 3]> = [(Channel::BaseColor, [0.2, 0.4, 0.6]), (Channel::Height, [-0.5; 3])].into_iter().collect();
        let (a, tiles) = alpha(0.7);
        let mut base = PixelStore::default();
        commit_layer(&mut base, LayerId(1), (64, 64), &vals, src(&a, &tiles, 1.0, false), &mut Staging::default());
        commit_mask(&mut base, LayerId(2), (64, 64), &MaskGray::Value(0.5), src(&a, &tiles, 1.0, false), &mut Staging::default());
        let carried: BTreeMap<Channel, TileImage> = vals.keys().map(|c| (*c, a.clone())).collect();

        // The final unstaged alpha change still belongs in the commit
        for (erase, carry, stage_last) in [(false, false, true), (true, false, true), (false, false, false), (false, true, true)] {
            let mut a = a.clone();
            let mut staged = base.clone();
            let mut staging = Staging::default();
            let c = carry.then_some(&carried);
            staging.stage_layer(&staged, LayerId(1), &vals, &source(&a, &tiles, c, erase));
            staging.stage_mask(&staged, LayerId(2), &MaskGray::Value(1.0), &source(&a, &tiles, c, erase));
            a.set(7, 9, &[to_u16(0.3)]);
            staging.stage_layer(&staged, LayerId(1), &vals, &source(&a, &tiles, c, erase));
            staging.stage_mask(&staged, LayerId(2), &MaskGray::Value(1.0), &source(&a, &tiles, c, erase));
            a.set(8, 9, &[to_u16(0.9)]);
            if stage_last {
                staging.stage_layer(&staged, LayerId(1), &vals, &source(&a, &tiles, c, erase));
                staging.stage_mask(&staged, LayerId(2), &MaskGray::Value(1.0), &source(&a, &tiles, c, erase));
            }
            let mut plain = base.clone();
            commit_layer(&mut plain, LayerId(1), (64, 64), &vals, source(&a, &tiles, c, erase), &mut Staging::default());
            commit_mask(&mut plain, LayerId(2), (64, 64), &MaskGray::Value(1.0), source(&a, &tiles, c, erase), &mut Staging::default());
            commit_layer(&mut staged, LayerId(1), (64, 64), &vals, source(&a, &tiles, c, erase), &mut staging);
            commit_mask(&mut staged, LayerId(2), (64, 64), &MaskGray::Value(1.0), source(&a, &tiles, c, erase), &mut staging);

            assert_eq!((&staged.layers, &staged.masks), (&plain.layers, &plain.masks), "erase {erase}, carried {carry}, staged last {stage_last}");
        }

        // The commit must read the current tile when its staged values are stale
        let mut px = base.clone();
        let mut staging = Staging::default();
        staging.stage_layer(&px, LayerId(1), &vals, &src(&a, &tiles, 1.0, false));
        staging.stage_mask(&px, LayerId(2), &MaskGray::Value(0.25), &src(&a, &tiles, 1.0, false));
        commit_layer(&mut px, LayerId(1), (64, 64), &vals, src(&a, &tiles, 1.0, false), &mut Staging::default());
        let mut expected = px.clone();
        commit_layer(&mut expected, LayerId(1), (64, 64), &vals, src(&a, &tiles, 1.0, false), &mut Staging::default());
        commit_mask(&mut expected, LayerId(2), (64, 64), &MaskGray::Value(1.0), src(&a, &tiles, 1.0, false), &mut Staging::default());
        commit_layer(&mut px, LayerId(1), (64, 64), &vals, src(&a, &tiles, 1.0, false), &mut staging);
        commit_mask(&mut px, LayerId(2), (64, 64), &MaskGray::Value(1.0), src(&a, &tiles, 1.0, false), &mut staging);

        assert_eq!((&px.layers, &px.masks), (&expected.layers, &expected.masks));
    }

    #[test]
    fn a_mask_stroke_paints_an_images_gray_per_texel_staged_or_not() {
        use std::sync::Arc;
        use texturecraft_compose::ImageRes;

        // A 2 × 1 image: black on the left half of the texture, white on the right
        let image = |name: &str, left: f32| MaskGray::Image { name: name.into(), image: Arc::new(ImageRes::new(2, 1, vec![[left; 4], [1.0; 4]], false)) };
        let mut a = TileImage::new(64, 64, 1);
        for (x, y) in [(16, 5), (48, 5)] {
            a.set(x, y, &[to_u16(1.0)]);
        }
        let tiles: BTreeSet<(u32, u32)> = [(0, 0)].into_iter().collect();
        let gray_at = |px: &PixelStore, x| from_u16(px.masks[&LayerId(4)].get(x, 5)[0]);

        let mut plain = PixelStore::default();
        commit_mask(&mut plain, LayerId(4), (64, 64), &image("scratches", 0.0), src(&a, &tiles, 1.0, false), &mut Staging::default());

        assert!(gray_at(&plain, 16) < 0.05 && gray_at(&plain, 48) > 0.95, "left {} right {}", gray_at(&plain, 16), gray_at(&plain, 48));

        // A resource replaced under the same name must invalidate its staged blend
        let mut staged = PixelStore::default();
        let mut staging = Staging::default();
        let previous_image = image("scratches", 0.5);
        staging.stage_mask(&staged, LayerId(4), &image("other", 0.5), &src(&a, &tiles, 1.0, false));
        staging.stage_mask(&staged, LayerId(4), &previous_image, &src(&a, &tiles, 1.0, false));

        commit_mask(&mut staged, LayerId(4), (64, 64), &image("scratches", 0.0), src(&a, &tiles, 1.0, false), &mut staging);

        assert_eq!(staged.masks, plain.masks);
    }
}
