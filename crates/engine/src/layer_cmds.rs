//! Layer stack commands: add, remove, duplicate, select, rename, visibility, reorder, group,
//! per-channel blending and fill sources.

use serde_json::{Value, json};
use texturecraft_core::{
    BlendMode, Channel, ChannelBlend, ChannelKind, EffectKind, Fill, Layer, LayerId, LayerKind, NormalFormat, Procedural, ProceduralKind, Project, Projection,
    Source,
};

use crate::commands::{
    CommandSpec, always, bad, blend_param, boolp, channel_param, color_param, content_param, f32p, has_layer, layer_param, normal_format_param, strp, u64p,
};
use crate::layer_selection::{batch_param, ids_json};
use crate::{EngineError, Result, Session, cmd, session::StackTarget};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("layer.add.paint", "Add paint layer", ["Layers"], None, "{name?}", always, add_paint),
        cmd!(
            "layer.add.fill",
            "Add fill layer",
            ["Layers"],
            None,
            "{name?, color?: [r,g,b] linear or \"#rrggbb\" sRGB, roughness?, metallic?}",
            always,
            add_fill
        ),
        cmd!("layer.add.group", "Add folder", ["Layers"], None, "{name?}", always, add_group),
        cmd!(
            "layer.remove",
            "Remove layer(s)",
            ["Layers"],
            Some("Delete"),
            "{layer?, layers?: [id] (wins over `layer`)} — default: every selected layer",
            has_layer,
            remove
        ),
        cmd!(
            "layer.duplicate",
            "Duplicate layer(s)",
            ["Layers"],
            Some("Ctrl+D"),
            "{layer?, layers?: [id] (wins over `layer`)} — default: every selected layer",
            has_layer,
            duplicate
        ),
        cmd!(
            "layer.group",
            "Group layer(s)",
            ["Layers"],
            Some("Ctrl+G"),
            "{layer?, layers?: [id] (wins over `layer`)} — default: every selected layer",
            has_layer,
            group
        ),
        cmd!(
            "layer.select",
            "Select layer",
            [],
            None,
            "{layer, target?: content|mask|<effect id>} | {layer, toggle: true} (Ctrl+click: add or take out one layer) | {layer, range: true} (Shift+click: the rows from the last clicked layer) | {all: true} (Ctrl+A: every shown row of the active texture set, folder rows and the layers of open folders included) | {layers: [id], layer?} (select several; `layer` is the active one)",
            always,
            select
        ),
        cmd!("layer.rename", "Rename layer", [], None, "{layer?, name}", has_layer, rename),
        cmd!(
            "layer.visible",
            "Toggle layer visibility",
            [],
            None,
            "{layer?, layers?: [id], visible?} — default: every selected layer; with `layers`, `layer` is the one the others follow",
            has_layer,
            visible
        ),
        cmd!(
            "layer.move",
            "Move layer",
            [],
            None,
            "{layer? | layers?: [id], index?} | {layer? | layers?: [id], above?: id|null, inside?: bool} | {layer?, delta: ±n}",
            has_layer,
            move_layer
        ),
        cmd!("layer.blend", "Set channel blending", [], None, "{layer?, channel?: id|all, mode?, opacity?: 0..1, enabled?}", has_layer, blend),
        cmd!(
            "layer.blending.set",
            "Set blending of several channels",
            [],
            None,
            "{layer?, channels: {<channel id>: {mode?, opacity?: 0..1}}} (only channels the layer has enabled; one undo step)",
            has_layer,
            blending_set
        ),
        cmd!(
            "layer.fill.set",
            "Set fill source",
            [],
            None,
            "{layer? | effect?: content fill effect id, channel, color?: ([r,g,b] linear or \"#rrggbb\" sRGB)|value?|procedural?: kind, scale?, seed?, contrast?, balance?, invert?, low?, high?, resolution?: auto|textureSet|uvTile|custom, customResolution?, resolutionFactor?: -3..3, tile?: {Tile Generator settings in camelCase, as project.inspect shows them}, tileDefaults?: true (alone, they edit the channel's pattern)|image?, normalFormat?: openGl|directX (the image's normal map convention; alone, it changes the channel's image)|anchor?: anchor effect id, anchorChannel?, colors?: gradient map of a color channel's procedural or anchor, 1 to 16 colors dark to light ([r, g, b] linear or \"#rrggbb\" sRGB; [] removes it; absent keeps it; alone, it changes the channel's source)|gradientMap?: true (turns the color stops on from the channel's current procedural or anchor: a procedural's low, their midpoint and high; an anchor's black, gray and white; stops already there stay), enabled?, solo?: true (this channel on, every other one off)}",
            has_layer,
            fill_set
        ),
        cmd!(
            "layer.fill.projection",
            "Set fill projection",
            [],
            None,
            "{layer? | effect?: content fill effect id, projection?: uv|triplanar|planar|spherical|cylindrical (default: the current one), scale?: number | [u, v] (uv), rotation?: degrees (uv), offset?: [u, v] (uv) | [x, y, z] (triplanar, planar), sharpness?: 1..64 (triplanar), axis?: 0|1|2 (planar, cylindrical)} — what is not given keeps its value",
            has_layer,
            fill_projection
        ),
        cmd!("layer.colorTag", "Set layer color tag", [], None, "{layer?, tag?: 0..7 (absent clears)}", has_layer, color_tag),
        cmd!("layer.list", "List layers", [], None, "{ts?}", always, list),
    ]
}

/// Insert a layer above the selection (inside it when it is a group) and select it.
pub(crate) fn insert_selected(s: &mut Session, layer: Layer) -> Result<LayerId> {
    let ts = s.ts();
    let sel = s.selected_layer();
    let inside = sel.and_then(|id| s.doc.project.layer(id).ok()).is_some_and(|l| l.is_group());
    let id = s.doc.project.insert_layer(ts, layer, sel, inside)?;
    s.select_one(ts, id);
    s.target = StackTarget::Content;
    Ok(id)
}

fn channels_of(s: &Session) -> Vec<Channel> {
    s.doc.project.texture_set(s.ts()).map(|t| t.channel_list()).unwrap_or_default()
}

fn add_paint(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = s.ts();
    let name = strp(p, "name").map(str::to_string).unwrap_or_else(|| s.doc.project.next_layer_name(ts, "Layer"));
    let id = s.edit("Add paint layer", |s| {
        let id = s.doc.project.alloc_id();
        insert_selected(s, Layer::new(id, name, LayerKind::Paint, &channels_of(s)))
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// A uniform fill from the `color`, `roughness`, `metallic`, `height` and `opacity` params.
pub(crate) fn fill_from(cmd: &str, p: &Value) -> Result<Fill> {
    let mut fill = Fill::default();
    if let Some(c) = color_param(cmd, p, "color")? {
        fill.sources.insert(Channel::BaseColor, Source::Uniform(c));
    }
    for (key, ch) in [("roughness", Channel::Roughness), ("metallic", Channel::Metallic), ("height", Channel::Height), ("opacity", Channel::Opacity)] {
        if let Some(v) = f32p(p, key) {
            fill.sources.insert(ch, Source::Uniform([v; 3]));
        }
    }
    Ok(fill)
}

fn add_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = s.ts();
    let name = strp(p, "name").map(str::to_string).unwrap_or_else(|| s.doc.project.next_layer_name(ts, "Fill layer"));
    let fill = fill_from("layer.add.fill", p)?;
    let id = s.edit("Add fill layer", |s| {
        let id = s.doc.project.alloc_id();
        insert_selected(s, Layer::new(id, name, LayerKind::Fill(fill), &channels_of(s)))
    })?;
    Ok(json!({ "layer": id.0 }))
}

fn add_group(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = s.ts();
    let name = strp(p, "name").map(str::to_string).unwrap_or_else(|| s.doc.project.next_layer_name(ts, "Folder"));
    let id = s.edit("Add folder", |s| {
        let id = s.doc.project.alloc_id();
        let mut l = Layer::new(id, name, LayerKind::Group { children: Vec::new(), collapsed: false }, &channels_of(s));
        for b in l.channels.values_mut() {
            b.mode = BlendMode::Passthrough;
        }
        insert_selected(s, l)
    })?;
    Ok(json!({ "layer": id.0 }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = batch_param(s, "layer.remove", p, true)?;
    let before = s.doc.clone();
    let removed = s.edit("Remove layer", |s| {
        let mut removed = Vec::new();

        // Top row first, so the layer under the lowest removed one is never removed after it is chosen
        for id in &ids {
            let (ts, parent, i) = s.doc.project.locate(*id).ok_or(EngineError::Other("layer not in a stack".into()))?;
            removed.extend(s.doc.project.remove_layer(*id)?);

            // Select the layer below (or above) the removed one
            let list = match parent {
                Some(g) => s.doc.project.layer(g).map(|l| l.children().to_vec()).unwrap_or_default(),
                None => s.doc.project.texture_set(ts).map(|t| t.stack.clone()).unwrap_or_default(),
            };
            let next = list.get(i.saturating_sub(1)).or(list.first()).copied().or(parent);
            match next {
                Some(n) => s.select_one(ts, n),
                None => s.select_none(ts),
            }
        }

        // Instances of what was removed keep their look as independent layers
        crate::instance_cmds::detach_orphans(s, &before)?;
        Ok(removed)
    })?;
    Ok(json!({ "removed": removed.iter().map(|l| l.0).collect::<Vec<_>>() }))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = batch_param(s, "layer.duplicate", p, true)?;
    let first = ids.first().copied().ok_or_else(|| bad("layer.duplicate", "no layer to duplicate"))?;
    let (ts, _, _) = s.doc.project.locate(first).ok_or(EngineError::Other("layer not in a stack".into()))?;
    let src = s.doc.clone();
    let copies = s.edit("Duplicate layer", |s| {
        // Insert copies in order above the topmost selected layer, at its folder level
        let mut copies = Vec::new();
        let mut above = first;
        for id in ids.iter().rev() {
            let new = crate::layer_ops::copy_tree(&src, *id, &mut s.doc, ts, Some(above), false, 0)?;
            let name = format!("{} copy", src.project.layer(*id)?.name);
            s.doc.project.layer_mut(new)?.name = name;
            copies.insert(0, new);
            above = new;
        }
        Ok(copies)
    })?;

    // The copies are selected; the active layer's copy stays the active one
    let active =
        s.selected.get(&ts).and_then(|active| ids.iter().position(|id| id == active)).and_then(|i| copies.get(i)).or(copies.first()).copied().unwrap_or(first);
    s.select_several(ts, active, active, copies.clone());
    Ok(json!({ "layer": active.0, "layers": ids_json(&copies) }))
}

fn group(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = batch_param(s, "layer.group", p, true)?;
    let g = s.edit("Group layers", |s| {
        // The folder takes the top layer's place and parent; the others follow it in, in stack order
        let id = ids.first().copied().ok_or_else(|| bad("layer.group", "no layer to group"))?;
        let (ts, _, _) = s.doc.project.locate(id).ok_or(EngineError::Other("layer not in a stack".into()))?;
        let gid = s.doc.project.alloc_id();
        let name = s.doc.project.next_layer_name(ts, "Folder");
        let channels = s.doc.project.texture_set(ts)?.channel_list();
        let mut gl = Layer::new(gid, name, LayerKind::Group { children: Vec::new(), collapsed: false }, &channels);
        for b in gl.channels.values_mut() {
            b.mode = BlendMode::Passthrough;
        }
        s.doc.project.insert_layer(ts, gl, Some(id), false)?;
        move_stacked(&mut s.doc.project, &ids, Some(gid), true)?;
        s.select_one(ts, gid);
        Ok(gid)
    })?;
    Ok(json!({ "layer": g.0 }))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    if boolp(p, "all") == Some(true) {
        return crate::layer_selection::select_all(s);
    }

    if p.get("layers").is_some() {
        return crate::layer_selection::select_list(s, p);
    }

    let id = layer_param(s, "layer.select", p)?;
    let (ts, _, _) = s.doc.project.locate(id).ok_or_else(|| bad("layer.select", "layer not in a stack"))?;

    // Paths paint into the active set
    if ts != s.ts() {
        s.finish_path()?;

        // The clone source is a point of the set it was picked in
        s.tool.clone_source = None;
    }

    // Shift+click and Ctrl/Cmd+click select several layers, each on its content
    let range = boolp(p, "range") == Some(true);
    if range || boolp(p, "toggle") == Some(true) {
        s.active_set = ts;
        let active = crate::layer_selection::extend(s, ts, id, range);
        s.target = StackTarget::Content;
        s.revision = s.revision.wrapping_add(1);
        return Ok(json!({ "layer": active.map(|active| active.0), "textureSet": ts, "layers": ids_json(&s.selected_layers()) }));
    }

    s.active_set = ts;
    s.select_one(ts, id);
    s.target = match p.get("target") {
        Some(Value::String(t)) if t == "mask" => {
            if s.doc.project.layer(s.doc.project.mask_owner(id))?.mask.is_none() {
                return Err(bad("layer.select", "the layer has no mask"));
            }

            StackTarget::Mask
        }
        Some(Value::String(t)) if t == "content" => StackTarget::Content,
        Some(v) => match v.as_u64() {
            Some(e) if s.doc.project.effect(LayerId(e)).is_some_and(|(owner, _)| owner == id) => StackTarget::Effect(LayerId(e)),
            _ => return Err(bad("layer.select", "target must be content, mask or an effect id of this layer")),
        },
        None => StackTarget::Content,
    };

    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "layer": id.0, "textureSet": ts, "layers": [id.0] }))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.rename", p)?;
    let name = strp(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("layer.rename", "`name` is required"))?.to_string();
    if name.chars().count() > 256 {
        return Err(bad("layer.rename", "name is longer than 256 characters"));
    }
    s.edit("Rename layer", |s| {
        s.doc.project.layer_mut(id)?.name = name;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn visible(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = batch_param(s, "layer.visible", p, false)?;

    // Use the clicked layer as lead, then the active or top layer if absent
    let lead = u64p(p, "layer")
        .map(LayerId)
        .filter(|lead| ids.contains(lead))
        .or(s.selected_layer().filter(|active| ids.contains(active)))
        .or(ids.first().copied())
        .ok_or_else(|| bad("layer.visible", "no layer given"))?;
    let v = s.edit("Layer visibility", |s| {
        let v = boolp(p, "visible").unwrap_or(!s.doc.project.layer(lead)?.visible);
        for id in &ids {
            s.doc.project.layer_mut(*id)?.visible = v;
        }
        Ok(v)
    })?;
    Ok(json!({ "visible": v }))
}

/// Move `ids` (top row first) above `target` (inside it when `inside`), keeping their order:
/// the lowest goes first and each next one above it.
fn move_stacked(project: &mut Project, ids: &[LayerId], target: Option<LayerId>, inside: bool) -> Result<()> {
    let (mut target, mut inside) = (target, inside);
    for id in ids.iter().rev() {
        project.move_above(*id, target, inside)?;
        (target, inside) = (Some(*id), false);
    }
    Ok(())
}

/// The positions of the moved layers determine whether a reorder needs an undo step.
fn apply_move(s: &mut Session, ids: &[LayerId], change: impl FnOnce(&mut Project) -> Result<()>) -> Result<Value> {
    s.edit_if_changed("Move layer", s.merge_edits, |s| {
        let project = &mut s.doc.project;
        let previous: Vec<_> = ids.iter().map(|id| project.locate(*id)).collect();
        change(project)?;
        let changed = ids.iter().map(|id| project.locate(*id)).ne(previous);
        Ok((Value::Null, changed))
    })
}

/// `layer.move {layers, index | above}`: the listed layers move together, keeping their order.
fn move_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = batch_param(s, "layer.move", p, true)?;
    if let Some(i) = u64p(p, "index") {
        // The lowest one goes to `index` of the texture set's stack, the others right above it in their order
        let to = usize::try_from(i).unwrap_or(usize::MAX);
        apply_move(s, &ids, |project| {
            let mut below = None;
            for id in ids.iter().rev() {
                match below {
                    None => {
                        project.move_above(*id, None, false)?;
                        project.move_layer(*id, to)?;
                    }
                    Some(below) => project.move_above(*id, Some(below), false)?,
                }
                below = Some(*id);
            }
            Ok(())
        })
    } else if p.get("above").is_some() {
        let above = u64p(p, "above").map(LayerId);
        if above.is_some_and(|above| ids.contains(&above)) {
            return Err(bad("layer.move", "`above` is one of the moved layers"));
        }

        let inside = boolp(p, "inside").unwrap_or(false);
        apply_move(s, &ids, |project| move_stacked(project, &ids, above, inside))
    } else {
        Err(bad("layer.move", "give `index` or `above` with `layers`"))
    }
}

fn move_layer(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("layers").is_some() {
        return move_layers(s, p);
    }

    let id = layer_param(s, "layer.move", p)?;
    apply_move(s, &[id], |project| {
        if let Some(d) = p.get("delta").and_then(Value::as_i64) {
            let (_, _, i) = project.locate(id).ok_or(EngineError::Other("layer not in a stack".into()))?;

            // Saturate hostile deltas before the sibling-list clamp
            let distance = usize::try_from(d.unsigned_abs()).unwrap_or(usize::MAX);
            let to = if d < 0 { i.saturating_sub(distance) } else { i.saturating_add(distance) };
            project.move_layer(id, to)?;
        } else if let Some(i) = u64p(p, "index") {
            project.move_layer(id, usize::try_from(i).unwrap_or(usize::MAX))?;
        } else if p.get("above").is_some() {
            let above = u64p(p, "above").map(LayerId);
            project.move_above(id, above, boolp(p, "inside").unwrap_or(false))?;
        } else {
            return Err(bad("layer.move", "give `delta`, `index` or `above`"));
        }

        Ok(())
    })
}

fn blend(s: &mut Session, p: &Value) -> Result<Value> {
    let id = content_param(s, "layer.blend", p)?;
    let all = strp(p, "channel") == Some("all");
    let ch = if all { None } else { channel_param("layer.blend", p, "channel")? };
    let mode = blend_param("layer.blend", p, "mode")?;
    let opacity = f32p(p, "opacity").map(|o| o.clamp(0.0, 1.0));
    let enabled = boolp(p, "enabled");
    let ts_channels = channels_of(s);
    s.edit("Layer blending", |s| {
        let l = s.doc.project.layer_mut(id)?;
        let targets: Vec<Channel> = match ch {
            Some(c) => vec![c],
            None => ts_channels.clone(),
        };
        for c in targets {
            match enabled {
                Some(false) => {
                    l.channels.remove(&c);
                    continue;
                }
                Some(true) => {
                    let mode = if l.is_group() { BlendMode::Passthrough } else { BlendMode::Normal };
                    l.channels.entry(c).or_insert(ChannelBlend { mode, opacity: 1.0 });
                }
                None => {}
            }
            if let Some(b) = l.channels.get_mut(&c) {
                if let Some(m) = mode {
                    b.mode = m;
                }
                if let Some(o) = opacity {
                    b.opacity = o;
                }
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// `layer.blending.set`: each listed channel gets its own mode and opacity in one undo step. Channels the layer
/// has disabled stay disabled, and channels not listed stay as they are.
fn blending_set(s: &mut Session, p: &Value) -> Result<Value> {
    let id = content_param(s, "layer.blending.set", p)?;
    let listed = p.get("channels").and_then(Value::as_object).ok_or_else(|| bad("layer.blending.set", "`channels` is required"))?;
    let mut edits: Vec<(Channel, Option<BlendMode>, Option<f32>)> = Vec::new();
    for (key, value) in listed {
        let channel = Channel::parse(key).ok_or_else(|| bad("layer.blending.set", format!("unknown channel `{key}`")))?;
        let mode = blend_param("layer.blending.set", value, "mode")?;
        let opacity = f32p(value, "opacity").map(|o| o.clamp(0.0, 1.0));
        edits.push((channel, mode, opacity));
    }
    s.edit("Layer blending", |s| {
        let l = s.doc.project.layer_mut(id)?;
        for (channel, mode, opacity) in edits {
            if let Some(b) = l.channels.get_mut(&channel) {
                if let Some(m) = mode {
                    b.mode = m;
                }
                if let Some(o) = opacity {
                    b.opacity = o;
                }
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Which fill `layer.fill.*` edits: a fill layer's (`layer`) or a content fill effect's
/// (`effect`).
#[derive(Clone, Copy)]
enum FillTarget {
    Layer(LayerId),
    Effect(LayerId),
}

fn fill_target(s: &Session, p: &Value, cmd: &str) -> Result<FillTarget> {
    match u64p(p, "effect").map(LayerId) {
        Some(e) => match s.doc.project.effect(e) {
            Some((_, x)) if matches!(x.kind, EffectKind::Material(_)) => Ok(FillTarget::Effect(e)),
            _ => Err(bad(cmd, format!("{} is not a content fill effect", e.0))),
        },
        None => Ok(FillTarget::Layer(content_param(s, cmd, p)?)),
    }
}

fn fill_mut(s: &mut Session, target: FillTarget) -> Result<&mut Fill> {
    match target {
        FillTarget::Layer(id) => match &mut s.doc.project.layer_mut(id)?.kind {
            LayerKind::Fill(f) => Ok(f),
            _ => Err(EngineError::Other("not a fill layer".into())),
        },
        FillTarget::Effect(e) => match s.doc.project.effect_mut(e).map(|x| &mut x.kind) {
            Some(EffectKind::Material(f)) => Ok(f),
            _ => Err(EngineError::Other("not a content fill effect".into())),
        },
    }
}

fn fill_set(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "layer.fill.set";
    let target = fill_target(session, params, CMD)?;
    let channel = channel_param(CMD, params, "channel")?.ok_or_else(|| bad(CMD, "`channel` is required"))?;

    // `solo`: this channel on, every other one off
    let solo = boolp(params, "solo") == Some(true);
    let enabled = if solo { Some(true) } else { boolp(params, "enabled") };
    let normal_format = normal_format_param(CMD, params)?;
    let colors = ramp_param(params, channel)?;
    let init_ramp = boolp(params, "gradientMap") == Some(true);
    if init_ramp && colors.is_some() {
        return Err(bad(CMD, "give `gradientMap` or `colors`, not both"));
    }

    let source = fill_source_from(session, params, (target, channel), normal_format)?;
    if source.is_none() && enabled.is_none() && normal_format.is_none() && colors.is_none() && !init_ramp {
        return Err(bad(
            CMD,
            "give `color`, `value`, `procedural`, `image`, `anchor`, pattern settings (`tile`, `resolution`…), `colors`, `gradientMap`, `normalFormat`, `enabled` or `solo`",
        ));
    }

    if normal_format.is_some() && source.as_ref().is_some_and(|source| !matches!(source, Source::Image { .. })) {
        return Err(bad(CMD, "`normalFormat` goes with `image` or alone"));
    }

    // Changing only the normal format or gradient stops leaves the layer channel on or off
    let touches_channel = source.is_some() || enabled.is_some();
    session.edit("Set fill", |session| {
        let fill = fill_mut(session, target)?;

        // `normalFormat` alone: the convention of the channel's image
        if let (None, Some(format)) = (&source, normal_format) {
            match fill.sources.get_mut(&channel) {
                Some(Source::Image { normal_format, .. }) => *normal_format = format,
                _ => return Err(bad(CMD, "`normalFormat` needs an image on the channel")),
            }
        }

        let current = fill.sources.get(&channel);
        let colors = if init_ramp { initial_ramp(channel, source.as_ref(), current)? } else { colors };
        let source = with_ramp(channel, current, source, colors)?;

        match target {
            // A fill effect's channel is on while it has a source
            FillTarget::Effect(_) if enabled == Some(false) => {
                fill.sources.remove(&channel);
            }

            // Removing stops (`colors: []`) from a channel the fill does not have adds nothing
            FillTarget::Effect(_) if source.is_none() && enabled != Some(true) && current.is_none() => {}
            FillTarget::Effect(_) => {
                let source = source.unwrap_or_else(|| fill.source(channel));
                fill.sources.insert(channel, source);
                if solo {
                    fill.sources.retain(|kept, _| *kept == channel);
                }
            }
            FillTarget::Layer(id) => {
                if let Some(source) = source {
                    fill.sources.insert(channel, source);
                }
                let layer = session.doc.project.layer_mut(id)?;
                if enabled == Some(false) {
                    layer.channels.remove(&channel);
                } else if touches_channel {
                    layer.channels.entry(channel).or_default();
                }
                if solo {
                    layer.channels.retain(|kept, _| *kept == channel);
                }
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// The source `params` give fill channel `channel` of `target`, or None when they only switch it or set its image's normal format
fn fill_source_from(
    session: &mut Session,
    params: &Value,
    (target, channel): (FillTarget, Channel),
    normal_format: Option<NormalFormat>,
) -> Result<Option<Source>> {
    const CMD: &str = "layer.fill.set";
    let low = color_param(CMD, params, "low")?.unwrap_or([0.0; 3]);
    let high = color_param(CMD, params, "high")?.unwrap_or([1.0; 3]);
    let uniform = color_param(CMD, params, "color")?.or(color_param(CMD, params, "value")?);

    if let Some(name) = strp(params, "procedural") {
        let kind = ProceduralKind::parse(name).ok_or_else(|| bad(CMD, format!("unknown procedural `{name}`")))?;
        let mut pattern = Procedural::new(kind);
        crate::procedural_params::pattern_basics_from(params, &mut pattern);

        // A pattern of the same kind keeps its resolution and Tile Generator settings
        let current = fill_mut(session, target)?.sources.get(&channel);
        if let Some(Source::Procedural { pattern: current, .. }) = current
            && current.kind == kind
        {
            pattern.resolution = current.resolution;
            pattern.tile = current.tile.clone();
        }

        crate::procedural_params::pattern_settings_from(CMD, params, &mut pattern)?;
        return Ok(Some(Source::Procedural { pattern, low, high, ramp: Vec::new() }));
    }

    if let Some(color) = uniform {
        return Ok(Some(Source::Uniform(color)));
    }

    if let Some(name) = strp(params, "image") {
        return Ok(Some(Source::Image { name: name.to_string(), normal_format: normal_format.unwrap_or_default() }));
    }

    if let Some(anchor) = u64p(params, "anchor") {
        let reader = match target {
            FillTarget::Layer(id) => Some(id),
            FillTarget::Effect(effect) => session.doc.project.effect(effect).map(|(owner, _)| owner),
        };
        let reader = reader.ok_or_else(|| bad(CMD, "the fill is not in a layer"))?;
        let anchor_channel = channel_param(CMD, params, "anchorChannel")?.unwrap_or(channel);
        return Ok(Some(crate::mask_cmds::anchor_source(session, CMD, reader, LayerId(anchor), anchor_channel)?));
    }

    if !crate::procedural_params::has_pattern_settings(params) {
        return Ok(None);
    }

    // Pattern settings alone edit the channel's current pattern
    let Some(Source::Procedural { pattern, low, high, .. }) = fill_mut(session, target)?.sources.get(&channel) else {
        return Err(bad(CMD, "the channel has no pattern: give `procedural` with its settings"));
    };

    let mut pattern = pattern.clone();
    crate::procedural_params::pattern_basics_from(params, &mut pattern);
    crate::procedural_params::pattern_settings_from(CMD, params, &mut pattern)?;

    Ok(Some(Source::Procedural { pattern, low: *low, high: *high, ramp: Vec::new() }))
}

/// The gradient map stops from `colors` (None when absent, empty to remove).
fn ramp_param(params: &Value, channel: Channel) -> Result<Option<Vec<[f32; 3]>>> {
    const MAX_GRADIENT_STOPS: usize = 16;
    let Some(list) = params.get("colors").filter(|value| !value.is_null()) else { return Ok(None) };

    let invalid = || bad("layer.fill.set", "`colors` must be up to 16 colors ([r, g, b] linear or \"#rrggbb\")");
    let list = list.as_array().filter(|list| list.len() <= MAX_GRADIENT_STOPS).ok_or_else(invalid)?;
    let colors: Vec<_> = list
        .iter()
        .map(|color| crate::commands::color_of(color).map(|rgb| rgb.map(|component| component.clamp(0.0, 1.0))).ok_or_else(invalid))
        .collect::<Result<_>>()?;

    if !colors.is_empty() && channel.kind() != ChannelKind::Color {
        return Err(bad("layer.fill.set", "`colors` (a gradient map) goes on a color channel"));
    }

    Ok(Some(colors))
}

/// The stops `gradientMap: true` turns on for channel `ch`: those the source already has, else a
/// procedural's low, midpoint and high (so its look holds) or an anchor's black, gray and white.
/// `source` is the one the call sets, else the channel's `current` one.
fn initial_ramp(channel: Channel, source: Option<&Source>, current: Option<&Source>) -> Result<Option<Vec<[f32; 3]>>> {
    if channel.kind() != ChannelKind::Color {
        return Err(bad("layer.fill.set", "`gradientMap` goes on a color channel"));
    }

    if let Some(kept) = current.map(Source::ramp).filter(|r| !r.is_empty()) {
        return Ok(Some(kept.to_vec()));
    }

    match source.or(current) {
        Some(Source::Procedural { low, high, .. }) => Ok(Some(vec![*low, std::array::from_fn(|k| (low[k] + high[k]) * 0.5), *high])),
        Some(Source::Anchor { .. }) => Ok(Some(vec![[0.0; 3], [0.5; 3], [1.0; 3]])),
        _ => Err(bad("layer.fill.set", "`gradientMap` needs a procedural or an anchor on the channel")),
    }
}

/// `source` (or, for `colors` alone, the channel's `current` source) with its gradient map stops:
/// `colors` when given, else those of `current`, so picking another pattern or anchor keeps them.
/// Only color channels keep stops, so a source copied onto another channel loses them; removing
/// stops (`colors: []`) where there are none changes nothing.
fn with_ramp(channel: Channel, current: Option<&Source>, source: Option<Source>, colors: Option<Vec<[f32; 3]>>) -> Result<Option<Source>> {
    let adds = colors.as_ref().is_some_and(|colors| !colors.is_empty());
    let needs = || bad("layer.fill.set", "`colors` needs a procedural or an anchor on the channel");
    let mut source = match (source, current) {
        (Some(source), _) => source,
        (None, Some(current)) if colors.is_some() => current.clone(),
        _ if adds => return Err(needs()),
        _ => return Ok(None),
    };

    match &mut source {
        Source::Procedural { ramp, .. } | Source::Anchor { ramp, .. } => {
            *ramp = colors.unwrap_or_else(|| match current {
                Some(current) if channel.kind() == ChannelKind::Color => current.ramp().to_vec(),
                _ => Vec::new(),
            });
        }
        _ if adds => return Err(needs()),
        _ => {}
    }

    Ok(Some(source))
}

/// The projection `p` asks for. Fields it does not give keep their values from `current` when
/// that is of the same kind (or `p` names no kind), and the kind's defaults otherwise.
pub(crate) fn projection_from(p: &Value, cmd: &str, current: Option<&Projection>) -> Result<Projection> {
    let named = match strp(p, "projection") {
        None => None,
        Some("uv") => Some(Projection::default()),
        Some("triplanar" | "tri-planar") => Some(Projection::Triplanar { scale: 1.0, sharpness: 8.0, offset: [0.0; 3] }),
        Some("planar") => Some(Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3] }),
        Some("spherical") => Some(Projection::Spherical { scale: 1.0 }),
        Some("cylindrical") => Some(Projection::Cylindrical { scale: 1.0, axis: 1 }),
        Some(other) => return Err(bad(cmd, format!("unknown projection `{other}`"))),
    };
    let mut proj = match (named, current) {
        (Some(n), Some(c)) if std::mem::discriminant(&n) == std::mem::discriminant(c) => c.clone(),
        (Some(n), _) => n,
        (None, c) => c.cloned().unwrap_or_default(),
    };
    let scale = floats::<2>(p, "scale", 0.001, 1000.0);
    let axis = u64p(p, "axis").map(|a| a.min(2) as u8);
    match &mut proj {
        Projection::Uv { scale: s, rotation, offset } => {
            *s = scale.unwrap_or(*s);
            *rotation = f32p(p, "rotation").map_or(*rotation, |r| r.rem_euclid(360.0));
            *offset = floats(p, "offset", -1000.0, 1000.0).unwrap_or(*offset);
        }
        Projection::Triplanar { scale: s, sharpness, offset } => {
            *s = scale.map_or(*s, |v| v[0]);
            *sharpness = f32p(p, "sharpness").map_or(*sharpness, |v| v.clamp(1.0, 64.0));
            *offset = floats(p, "offset", -1000.0, 1000.0).unwrap_or(*offset);
        }
        Projection::Planar { scale: s, axis: a, offset } => {
            *s = scale.map_or(*s, |v| v[0]);
            *a = axis.unwrap_or(*a);
            *offset = floats(p, "offset", -1000.0, 1000.0).unwrap_or(*offset);
        }
        Projection::Spherical { scale: s } => *s = scale.map_or(*s, |v| v[0]),
        Projection::Cylindrical { scale: s, axis: a } => {
            *s = scale.map_or(*s, |v| v[0]);
            *a = axis.unwrap_or(*a);
        }
        Projection::Decal(_) => {}
    }
    Ok(proj)
}

/// `key` as `N` finite numbers clamped to `lo..=hi`: an array of `N`, or one number for all.
fn floats<const N: usize>(p: &Value, key: &str, lo: f32, hi: f32) -> Option<[f32; N]> {
    let one = |x: &Value| x.as_f64().map(|f| f as f32).filter(|f| f.is_finite()).map(|f| f.clamp(lo, hi));
    match p.get(key)? {
        Value::Array(a) if a.len() == N => {
            let mut out = [0.0; N];
            for (o, x) in out.iter_mut().zip(a) {
                *o = one(x)?;
            }
            Some(out)
        }
        v => one(v).map(|f| [f; N]),
    }
}

fn fill_projection(s: &mut Session, p: &Value) -> Result<Value> {
    let target = fill_target(s, p, "layer.fill.projection")?;
    s.edit("Fill projection", |s| {
        let fill = fill_mut(s, target)?;
        fill.projection = projection_from(p, "layer.fill.projection", Some(&fill.projection))?;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn color_tag(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.colorTag", p)?;
    let tag = u64p(p, "tag").map(|t| t.min(7) as u8);
    s.edit("Layer color tag", |s| {
        s.doc.project.layer_mut(id)?.color_tag = tag;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = u64p(p, "ts").map(|t| t as usize).unwrap_or(s.ts());
    Ok(crate::inspect::stack(s, ts))
}
