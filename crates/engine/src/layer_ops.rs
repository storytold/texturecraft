//! Layer clipboard (copy, cut, paste across texture sets), flatten, and resampling of painted
//! pixels when a texture set changes size.

use serde_json::{Value, json};
use texturecraft_core::{Channel, ChannelKind, LayerId, LayerKind};
use texturecraft_texel::TileImage;

use crate::commands::{CommandSpec, boolp, has_layer, layer_param};
use crate::session::{Document, StackTarget};
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("layer.copy", "Copy layer(s)", ["Layers"], Some("Ctrl+C"), "{layer?}", has_layer, copy),
        cmd!("layer.cut", "Cut layer(s)", ["Layers"], Some("Ctrl+X"), "{layer?}", has_layer, cut),
        cmd!("layer.paste", "Paste layer(s)", ["Layers"], Some("Ctrl+V"), "{} — above the selection, in the active texture set", can_paste, paste),
        cmd!(
            "layer.flatten",
            "Flatten layer",
            ["Layers"],
            Some("Ctrl+M"),
            "{layer?} — a fill or masked paint layer becomes a plain paint layer",
            has_layer,
            flatten
        ),
        cmd!(
            "layer.instantiateAll",
            "Instantiate across texture sets",
            ["Layers"],
            Some("Ctrl+Shift+D"),
            "{layer?, copy?: bool} — a linked instance of the layer on top of every other texture set (`copy`: independent copies)",
            has_layer,
            to_all_sets
        ),
    ]
}

fn can_paste(s: &Session) -> std::result::Result<(), String> {
    match &s.clipboard {
        Some((doc, id)) if doc.project.layers.contains_key(id) => Ok(()),
        Some(_) => Err("the clipboard holds an effect (effect.paste)".into()),
        None => Err("the layer clipboard is empty".into()),
    }
}

/// Encoded "nothing painted" texel for a channel image (value 0, coverage 0).
pub(crate) fn empty_texel(c: Channel) -> Vec<u16> {
    let zero = if c.kind() == ChannelKind::Signed { texturecraft_texel::to_u16_signed(0.0) } else { 0 };
    let mut v = vec![zero; c.components()];
    v.push(0);
    v
}

pub(crate) fn fit(img: &TileImage, (w, h): (u32, u32), empty: &[u16]) -> TileImage {
    if (img.width(), img.height()) == (w, h) { img.clone() } else { img.resampled(w, h, empty) }
}

/// Copy layer `id` of document `src` (recursively, with pixels) into the session's texture set
/// `ts`, above `above` (or inside it when `inside`). Pixels are resampled to the target set's
/// resolution. Returns the new id.
pub(crate) fn copy_tree(src: &Document, id: LayerId, s: &mut Document, ts: usize, above: Option<LayerId>, inside: bool, depth: usize) -> Result<LayerId> {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return Err(EngineError::Other("layer groups nested too deeply".into()));
    }

    let size = s.project.texture_set(ts)?.size();
    let channels = s.project.texture_set(ts)?.channel_list();
    let layer = src.project.layer(id)?;
    let new = s.project.alloc_id();
    let mut copy = layer.clone();
    copy.id = new;

    // Channels absent from the target set stay disabled
    copy.channels.retain(|channel, _| channels.contains(channel));
    for effect in copy.mask.iter_mut().flat_map(|mask| &mut mask.effects).chain(&mut copy.content_effects) {
        let old = effect.id;
        effect.id = s.project.alloc_id();
        if let Some(image) = src.pixels.masks.get(&old) {
            s.pixels.masks.insert(effect.id, fit(image, size, &[0, 0]));
        }

        for &channel in &channels {
            if let Some(image) = src.pixels.layer(old, channel) {
                s.pixels.layers.insert((effect.id, channel), fit(image, size, &empty_texel(channel)));
            }
        }
    }

    if let LayerKind::Group { children, .. } = &mut copy.kind {
        children.clear();
    }

    for &channel in &channels {
        if let Some(image) = src.pixels.layer(id, channel) {
            s.pixels.layers.insert((new, channel), fit(image, size, &empty_texel(channel)));
        }
    }

    s.project.insert_layer(ts, copy, above, inside)?;
    let mut previous = None;
    for &child in layer.children() {
        let placed = copy_tree(src, child, s, ts, previous.or(Some(new)), previous.is_none(), depth + 1)?;
        previous = Some(placed);
    }

    if matches!(&layer.kind, LayerKind::Group { randomized, .. } if !randomized.is_empty()) {
        remap_seed_records(&src.project, &mut s.project, id, new)?;
    }

    Ok(new)
}

/// Retarget saved seed resets to the layers and effects of the new folder.
fn remap_seed_records(source: &texturecraft_core::Project, destination: &mut texturecraft_core::Project, original: LayerId, copy: LayerId) -> Result<()> {
    let mut pairs = vec![(original, copy)];
    let mut ids = std::collections::HashMap::new();
    while let Some((original, copy)) = pairs.pop() {
        let old = source.layer(original)?;
        let new = destination.layer(copy)?;
        ids.insert(original, copy);
        pairs.extend(old.children().iter().copied().zip(new.children().iter().copied()));

        fn effect_ids(layer: &texturecraft_core::Layer) -> impl Iterator<Item = LayerId> {
            layer.content_effects.iter().chain(layer.mask.iter().flat_map(|mask| &mask.effects)).map(|effect| effect.id)
        }

        ids.extend(effect_ids(old).zip(effect_ids(new)));
    }

    if let LayerKind::Group { randomized, .. } = &mut destination.layer_mut(copy)?.kind {
        randomized.retain_mut(|record| {
            let Some(layer) = ids.get(&record.layer) else { return false };
            record.layer = *layer;
            if let Some((source, _)) = &mut record.target {
                let Some(target) = ids.get(source) else { return false };
                *source = *target;
            }

            true
        });
    }

    Ok(())
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.copy", p)?;
    s.clipboard = Some((s.doc.clone(), id));
    Ok(json!({ "copied": id.0 }))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.cut", p)?;
    s.clipboard = Some((s.doc.clone(), id));
    crate::execute(s, "layer.remove", &json!({ "layer": id.0 }))?;
    Ok(json!({ "cut": id.0 }))
}

fn paste(s: &mut Session, _: &Value) -> Result<Value> {
    let (src, id) = s.clipboard.clone().ok_or_else(|| EngineError::Other("the layer clipboard is empty".into()))?;
    let new = s.edit("Paste layer", |s| {
        let ts = s.ts();
        let sel = s.selected_layer();
        let inside = sel.and_then(|l| s.doc.project.layer(l).ok()).is_some_and(|l| l.is_group());
        let new = copy_tree(&src, id, &mut s.doc, ts, sel, inside, 0)?;
        s.select_one(ts, new);
        s.target = StackTarget::Content;
        Ok(new)
    })?;
    Ok(json!({ "layer": new.0 }))
}

fn to_all_sets(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.instantiateAll", p)?;
    let (from, _, _) = s.doc.project.locate(id).ok_or_else(|| EngineError::Other("layer not in a stack".into()))?;
    let src = s.doc.clone();
    let copy = boolp(p, "copy").unwrap_or(false);
    let label = if copy { "Copy layer to all texture sets" } else { "Instantiate across texture sets" };
    let made = s.edit(label, |s| {
        let mut made = Vec::new();
        for ts in (0..s.set_count()).filter(|ts| *ts != from) {
            let new = if copy {
                copy_tree(&src, id, &mut s.doc, ts, None, false, 0)?
            } else {
                // On top of the set: clear its selection first
                s.select_none(ts);
                crate::instance_cmds::make_instance(s, id, ts)?
            };
            made.push(new.0);
        }
        Ok(made)
    })?;
    Ok(json!({ "layers": made }))
}

fn flatten(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.flatten", p)?;
    let (ts, _, _) = s.doc.project.locate(id).ok_or_else(|| EngineError::Other("layer not in a stack".into()))?;
    let layer = s.doc.project.layer(id)?.clone();
    // An instance flattens what it draws into a plain layer of its own.
    let content = s.doc.project.content_layer(id).cloned().ok_or_else(|| EngineError::Other("the instance's source is missing".into()))?;
    if content.is_group() {
        return Err(EngineError::Other("flattening a folder is not supported yet; flatten its layers".into()));
    }
    s.update();
    let size = s.caches.get(ts).ok_or_else(|| EngineError::Other("texture set not ready".into()))?.size;
    let flat = s.flatten(ts, id).ok_or_else(|| EngineError::Other("nothing to flatten".into()))?;
    s.edit("Flatten layer", |s| {
        for c in Channel::ALL {
            s.doc.pixels.layers.remove(&(id, c));
        }
        let effects: Vec<LayerId> = layer.mask.iter().flat_map(|m| m.effects.iter()).chain(layer.content_effects.iter()).map(|e| e.id).collect();
        s.doc.pixels.masks.retain(|e, _| !effects.contains(e));
        s.doc.pixels.layers.retain(|(e, _), _| !effects.contains(e));
        for (c, (tx, ty), data) in flat {
            let img = s.doc.pixels.layer_mut(id, c, size);
            img.set_tile(tx, ty, Some(std::sync::Arc::new(data)));
        }
        let channels = s.doc.project.texture_set(ts)?.channel_list();
        let l = s.doc.project.layer_mut(id)?;
        l.kind = LayerKind::Paint;
        l.mask = None;
        l.content_effects.clear();
        l.height_mask = Default::default();
        l.channels = content.channels.into_iter().filter(|(c, _)| channels.contains(c)).collect();
        s.target = StackTarget::Content;
        Ok(())
    })?;
    Ok(json!({ "layer": id.0 }))
}

impl Session {
    /// Layer `id` of texture set `ts` alone (content times mask) as premultiplied tiles per
    /// channel over the set's covered tiles. None for folders or a set not ready.
    pub(crate) fn flatten(&self, ts: usize, id: LayerId) -> Option<Vec<texturecraft_compose::eval::FlatTile>> {
        let tiles: Vec<(u32, u32)> = self.caches.get(ts)?.covered_tiles.iter().copied().collect();
        self.with_inputs(ts, |inp| texturecraft_compose::flatten_layer(inp, id, &tiles))
    }
}

/// Resample every painted image of texture set `ts` to `size` (after a size change).
pub(crate) fn resample_set(doc: &mut Document, ts: usize, size: (u32, u32)) {
    map_set_images(doc, ts, |img, empty| fit(img, size, empty));
}

/// Replace every painted image of texture set `ts` (layers, content and mask paint effects) by
/// `f(image, empty texel)`.
pub(crate) fn map_set_images(doc: &mut Document, ts: usize, f: impl Fn(&TileImage, &[u16]) -> TileImage) {
    let ids: Vec<LayerId> = doc.project.walk(ts).into_iter().map(|(id, _)| id).collect();
    let effect_ids: Vec<LayerId> = ids
        .iter()
        .filter_map(|id| doc.project.layers.get(id))
        .flat_map(|l| l.mask.iter().flat_map(|m| m.effects.iter()).chain(l.content_effects.iter()).map(|e| e.id))
        .collect();
    for ((id, c), img) in doc.pixels.layers.iter_mut() {
        if ids.contains(id) || effect_ids.contains(id) {
            *img = f(img, &empty_texel(*c));
        }
    }
    for (id, img) in doc.pixels.masks.iter_mut() {
        if effect_ids.contains(id) {
            *img = f(img, &[0, 0]);
        }
    }
}
