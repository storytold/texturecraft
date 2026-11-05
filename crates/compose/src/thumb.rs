//! Small previews of one layer's content, one mask, or one effect stage, computed without
//! touching the main composite.
//!
//! Every thumbnail is the ordinary compositor run on a *cut* copy of the project: the texture
//! set's stack is replaced by the one layer, the layer is forced visible at Normal / 100 %, and
//! the layer's effect list is truncated after the effect being previewed. The evaluator is the
//! same code the viewport uses (just on a small G-buffer), so a thumbnail always agrees with
//! what the stack really computes. Nothing here mutates the caller's project.

use texturecraft_core::{BlendMode, Channel, ChannelBlend, ChannelKind, LayerId, Project};
use texturecraft_texel::{TILE, from_u16, from_u16_signed, linear_to_srgb};

use crate::eval::{Composite, FlatTile, Inputs, all_tiles, composite_tiles, flatten_layer, mask_image};

/// Edge length (texels) the thumbnails are computed at.
pub const THUMB_TEXELS: u32 = 64;

/// What a thumbnail shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThumbTarget {
    /// The content of a layer (content effects included, mask ignored) in `channel`.
    Content { layer: LayerId, channel: Channel },
    /// The mask of a layer (an instance without its own mask shows its source's).
    Mask { layer: LayerId },
    /// The result up to and including `effect`, in the layer's content (`channel`) or in its
    /// mask (the channel is ignored). An effect that is hidden shows the bypassed result.
    Stage { effect: LayerId, channel: Channel },
}

/// An 8-bit RGBA thumbnail (straight alpha). Alpha is 0 where the mesh does not cover the
/// texture and, for a paint or fill layer's content, where the layer has nothing; the view
/// paints its own background there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The thumbnail of `target`, computed at `inp.gbuf`'s size, which must be square (use a
/// [`THUMB_TEXELS`] G-buffer; None otherwise). None for an unknown layer or effect, a layer without that channel, or a
/// mask that does not exist.
pub fn render(inp: &Inputs, target: ThumbTarget) -> Option<ThumbImage> {
    if inp.gbuf.width != inp.gbuf.height {
        return None;
    }
    match target {
        ThumbTarget::Content { layer, channel } => content(inp, layer, channel, None),
        ThumbTarget::Mask { layer } => mask(inp, layer, None),
        ThumbTarget::Stage { effect, channel } => {
            let (owner, _) = inp.project.effect(effect)?;
            let in_mask = inp.project.layers.get(&owner)?.mask.as_ref().is_some_and(|m| m.effects.iter().any(|e| e.id == effect));
            if in_mask { mask(inp, owner, Some(effect)) } else { content(inp, owner, channel, Some(effect)) }
        }
    }
}

/// `project` cut down to the single layer `id` of texture set `ts`: only that layer is drawn,
/// visible, Normal at full opacity in `channel` (when given).
fn solo(project: &Project, ts: usize, id: LayerId, channel: Option<Channel>) -> Option<Project> {
    let mut cut = project.clone();
    cut.texture_sets.get_mut(ts)?.stack = vec![id];
    let l = cut.layers.get_mut(&id)?;
    l.visible = true;
    if let Some(c) = channel {
        *l.channels.get_mut(&c)? = ChannelBlend { mode: BlendMode::Normal, opacity: 1.0 };
    }
    Some(cut)
}

/// Keep the effects of `list` up to and including `last`.
fn truncate_after(list: &mut Vec<texturecraft_core::Effect>, last: LayerId) -> bool {
    match list.iter().position(|e| e.id == last) {
        Some(k) => {
            list.truncate(k + 1);
            true
        }
        None => false,
    }
}

fn content(inp: &Inputs, layer: LayerId, channel: Channel, upto: Option<LayerId>) -> Option<ThumbImage> {
    let set = inp.project.texture_set(inp.ts).ok()?;
    if !set.channel_list().contains(&channel) {
        return None;
    }
    // The layer whose content is drawn: an instance draws its source's effects.
    let mut cut = solo(inp.project, inp.ts, layer, Some(channel))?;
    let owner = cut.content_layer(layer)?.id;
    for id in [layer, owner] {
        let l = cut.layers.get_mut(&id)?;
        l.mask = None;
        l.visible = true;
        if let Some(b) = l.channels.get_mut(&channel) {
            *b = ChannelBlend { mode: BlendMode::Normal, opacity: 1.0 };
        }
    }
    if let Some(effect) = upto
        && !truncate_after(&mut cut.layers.get_mut(&owner)?.content_effects, effect)
    {
        return None;
    }
    let cut_inp = Inputs { project: &cut, preview: None, ..*inp };
    let (w, h) = (inp.gbuf.width, inp.gbuf.height);
    // A paint or fill layer shows its own coverage (transparent where nothing is painted); a
    // folder shows its children composited over the channel's default.
    if !cut.layers.get(&owner)?.is_group() {
        let flat = flatten_layer(&cut_inp, layer, &all_tiles(w, h))?;
        return Some(encode_flat(&flat, channel, w, h));
    }
    let mut comp = Composite::with_size(&set.channel_list(), w, h);
    composite_tiles(&cut_inp, &mut comp, &all_tiles(w, h));
    Some(encode_channel(&comp, channel, inp))
}

/// Encode the flattened (premultiplied value, coverage) tiles of `channel`: alpha is coverage.
fn encode_flat(flat: &[FlatTile], channel: Channel, w: u32, h: u32) -> ThumbImage {
    let mut rgba = vec![0u8; (w * h) as usize * 4];
    let comps = channel.components();
    let signed = channel.kind() == ChannelKind::Signed;
    for (c, (tx, ty), buf) in flat {
        if *c != channel {
            continue;
        }
        for (o, px) in buf.chunks_exact(comps + 1).enumerate() {
            let (x, y) = (tx * TILE + o as u32 % TILE, ty * TILE + o as u32 / TILE);
            let a = px.get(comps).map_or(0.0, |v| from_u16(*v));
            if x >= w || y >= h || a <= 0.0 {
                continue;
            }
            let mut v = [0.0f32; 3];
            for (k, slot) in v.iter_mut().enumerate() {
                let raw = px.get(k.min(comps - 1)).copied().unwrap_or(0);
                *slot = if signed { from_u16_signed(raw) } else { from_u16(raw) } / a;
            }
            let p = display(channel, v);
            let i = ((y * w + x) * 4) as usize;
            if let Some(d) = rgba.get_mut(i..i + 4) {
                d.copy_from_slice(&[p[0], p[1], p[2], to_byte(a)]);
            }
        }
    }
    ThumbImage { width: w, height: h, rgba }
}

/// A channel value as display bytes: colors linear → sRGB, signed values with 0 at mid gray,
/// normal maps and scalars as stored.
fn display(channel: Channel, v: [f32; 3]) -> [u8; 3] {
    match channel.kind() {
        ChannelKind::Color => v.map(|c| to_byte(linear_to_srgb(c.clamp(0.0, 1.0)))),
        ChannelKind::Signed => v.map(|c| to_byte(c * 0.5 + 0.5)),
        ChannelKind::Scalar | ChannelKind::NormalMap => v.map(to_byte),
    }
}

fn mask(inp: &Inputs, layer: LayerId, upto: Option<LayerId>) -> Option<ThumbImage> {
    let mut cut = solo(inp.project, inp.ts, layer, None)?;
    let owner = cut.mask_owner(layer);
    let m = cut.layers.get_mut(&owner)?.mask.as_mut()?;
    m.enabled = true;
    if let Some(effect) = upto
        && !truncate_after(&mut m.effects, effect)
    {
        return None;
    }
    let cut_inp = Inputs { project: &cut, preview: None, ..*inp };
    let (w, h) = (inp.gbuf.width, inp.gbuf.height);
    let gray = mask_image(&cut_inp, layer, w)?;
    let mut rgba = Vec::with_capacity((w * h) as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let g = gray.get(i).copied().unwrap_or(0);
            rgba.extend_from_slice(&[g, g, g, if inp.gbuf.covered(i) { 255 } else { 0 }]);
        }
    }
    Some(ThumbImage { width: w, height: h, rgba })
}

/// Encode the composited `channel` for display: colors linear → sRGB, signed values 0 → mid
/// gray, normal maps and scalars as stored (data channels raw).
fn encode_channel(comp: &Composite, channel: Channel, inp: &Inputs) -> ThumbImage {
    let (w, h) = (comp.resolution, comp.height);
    let mut rgba = Vec::with_capacity((w * h) as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let v = comp.value(channel, x, y);
            let px = display(channel, v);
            let covered = inp.gbuf.covered((y * w + x) as usize);
            rgba.extend_from_slice(&[px[0], px[1], px[2], if covered { 255 } else { 0 }]);
        }
    }
    ThumbImage { width: w, height: h, rgba }
}

fn to_byte(v: f32) -> u8 {
    if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 }
}
