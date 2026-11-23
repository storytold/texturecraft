//! Layer stack commands: add, remove, duplicate, select, rename, visibility, reorder, group,
//! per-channel blending and fill sources.

use serde_json::{Value, json};
use texturecraft_core::{
    Backface, BlendMode, Channel, ChannelBlend, ChannelKind, Decal, EffectKind, Fill, FillSymmetry, Filtering, ImageInput, Layer, LayerId, LayerKind,
    NormalFormat, Procedural, ProceduralKind, Project, Projection, Projector, Sampling, ScaleMode, ShapeCrop, Source, SymmetryKind, UvTransform, UvWrap,
};
use texturecraft_paint::alphas::{self, LibraryAlpha};

use crate::commands::{
    CommandSpec, always, bad, blend_param, boolp, channel_param, color_param, content_param, f32p, has_layer, layer_param, normal_format_param, strp, u64p,
};
use crate::layer_selection::{batch_param, ids_json};
use crate::{EngineError, Result, Session, cmd, session::StackTarget};

mod fill_target;

pub(crate) use fill_target::FillTarget;

/// One full turn, in degrees (the brush's, so every angle wraps on the same turn)
pub(crate) use texturecraft_paint::FULL_TURN_DEGREES;

/// The smallest scale or tiling a fill's placement takes
pub const MIN_SCALE: f32 = 0.001;
/// The largest scale, tiling or offset a fill's placement takes
pub const MAX_PLACEMENT: f32 = 1000.0;

/// The command [`fill_set`] runs as
const FILL_SET: &str = "layer.fill.set";

/// How far a warp's corner may be pulled from the frame's centre, in frame sizes
const MAX_CORNER_REACH: f32 = 100.0;

/// The largest Projection Depth, in lengths of the frame's longer side
pub(crate) const MAX_PROJECTION_DEPTH: f32 = 100.0;

/// The `layer.fill.set` keys a one-source grayscale fill (a mask fill or a filter's image input) has no use for
const GRAY_REFUSED_KEYS: [&str; 4] = ["channel", "enabled", "solo", "normalFormat"];

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
            "{layer? | effect?: content fill effect id | mask fill effect id (one grayscale source: no channel, enabled, solo or normalFormat), channel, color?: ([r,g,b] linear or \"#rrggbb\" sRGB)|value?|procedural?: kind, scale?, seed?, contrast?, balance?, invert?, low?, high? (the same kind keeps what is not given), resolution?: auto|textureSet|uvTile|custom, customResolution?, resolutionFactor?: -3..3, tile?: {Tile Generator settings in camelCase, as project.inspect shows them}, tileDefaults?: true (alone, they edit the channel's pattern)|image?|alpha?: library alpha id or name (its picture becomes a project image, normally alpha_<id>.png), normalFormat?: openGl|directX (the image's normal map convention; alone, it changes the channel's image)|anchor?: anchor effect id, anchorChannel?, colors?: gradient map of a color channel's procedural or anchor, 1 to 16 colors dark to light ([r, g, b] linear or \"#rrggbb\" sRGB; [] removes it; absent keeps it; alone, it changes the channel's source)|gradientMap?: true (turns the color stops on from the channel's current procedural or anchor: a procedural's low, their midpoint and high; an anchor's black, gray and white; stops already there stay), enabled?, solo?: true (this channel on, every other one off)} | {effect: filter effect id, input: image input id (distanceMap, customNoise), value?|procedural?|image?|alpha?|anchor? as a mask fill's source, or clear: true (back to its default gray)}",
            has_layer,
            fill_set
        ),
        cmd!(
            "layer.fill.projection",
            "Set fill projection",
            [],
            None,
            "{layer? | effect?: content or mask fill effect id, projection?: uvTile (Fill (Match per UV Tile))|uv|triplanar|planar|spherical|cylindrical|warp (default: the current one; a new warp's frame sits around the mesh, seen from the front), tiling?: number | [u, v] (UV transformations except on uvTile; uv: same as scale), rotation?: degrees (UV transformations), uvOffset?: [u, v] (UV transformations of a 3D projection), scale?: number | [u, v] (uv: the tiling; 3D projections but warp: one number), offset?: [u, v] (uv) | [x, y, z] (3D projections but warp; a warp is placed by its frame with layer.decal.set, and refuses either), projectorRotation?: [x, y, z] degrees, projectorScale?: number | [x, y, z] (3D projection settings; a negative axis mirrors it), shapeCrop?: cropped|extendsOutside (3D projections), backfaceCulling?: bool, backfaceAngle?: degrees, backfaceHardness?: 0..1 (planar, cylindrical), depthCulling?: bool, depthCullingHardness?: 0..1 (planar, warp; surfaces hidden from the projector behind others get none of it; new planar and warp projections start with it on at 0.75), sharpness?: 1..64 (triplanar), warpCorners?: [[x, y] × 4] (warp: top left, top right, bottom right, bottom left, across and up in frame sizes from its centre; unpulled = ±0.5), projectionDepth?: lengths of the frame's longer side it reaches in front of and behind it when cropped (warp), angle?: 1..360 degrees one repeat spans (cylindrical), axis?: 0|1|2 (planar, cylindrical), filtering?: bilinearHq|bilinearSharp|nearest (how images are read), wrap?: repeat|repeatHorizontally|repeatVertically|none (past the 0..1 tile; not tri-planar), symmetry?: bool (copies of a 3D projection; UV projections have none), symmetryType?: mirror|radial, symmetryAxis?: 0|1|2 or x|y|z (the mirror plane's normal or the turning axis), flipCopy?: [u, v] (turn the copies' images over), axisPosition?: number | [x, y, z] (where the plane or axis passes, -1..1 mesh radii from the mesh's centre), symmetryCount?: 2..64 (radial copies, the original included), angleSpan?: 1..360 degrees (radial copies spread over it; 360 spaces them evenly)} — what is not given keeps its value",
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
        let channels = channels_of(s);
        insert_selected(s, new_folder(id, name, &channels))
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
    let folder = s.edit("Group layers", |s| {
        // The folder takes the top layer's place and parent; the others follow it in, in stack order
        let id = ids.first().copied().ok_or_else(|| bad("layer.group", "no layer to group"))?;
        let (ts, _, _) = s.doc.project.locate(id).ok_or(EngineError::Other("layer not in a stack".into()))?;
        let folder = s.doc.project.alloc_id();
        let name = s.doc.project.next_layer_name(ts, "Folder");
        let channels = s.doc.project.texture_set(ts)?.channel_list();
        s.doc.project.insert_layer(ts, new_folder(folder, name, &channels), Some(id), false)?;
        move_stacked(&mut s.doc.project, &ids, Some(folder), true)?;
        s.select_one(ts, folder);
        Ok(folder)
    })?;
    Ok(json!({ "layer": folder.0 }))
}

/// An empty folder `id` whose channels pass the blending of its contents through
fn new_folder(id: LayerId, name: String, channels: &[Channel]) -> Layer {
    let mut folder = Layer::new(id, name, LayerKind::folder(), channels);
    for blend in folder.channels.values_mut() {
        blend.mode = BlendMode::Passthrough;
    }
    folder
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

pub(crate) fn fill_target(s: &Session, p: &Value, cmd: &str) -> Result<FillTarget> {
    match u64p(p, "effect").map(LayerId) {
        Some(effect) => match s.doc.project.effect(effect).map(|(_, found)| &found.kind) {
            Some(EffectKind::Material(_)) => Ok(FillTarget::Effect(effect)),
            Some(EffectKind::Fill { .. }) => Ok(FillTarget::Mask(effect)),
            _ => Err(bad(cmd, format!("{} is not a fill effect", effect.0))),
        },
        None => Ok(FillTarget::Layer(content_param(s, cmd, p)?)),
    }
}

/// The material of the fill at `target`; a mask fill has none
pub(crate) fn fill_mut(s: &mut Session, target: FillTarget) -> Result<&mut Fill> {
    match target {
        FillTarget::Layer(id) => match &mut s.doc.project.layer_mut(id)?.kind {
            LayerKind::Fill(f) => Ok(f),
            _ => Err(EngineError::Other("not a fill layer".into())),
        },
        FillTarget::Effect(effect) => match s.doc.project.effect_mut(effect).map(|found| &mut found.kind) {
            Some(EffectKind::Material(f)) => Ok(f),
            _ => Err(EngineError::Other("not a content fill effect".into())),
        },
        FillTarget::Mask(_) => Err(EngineError::Other("a mask fill has one grayscale source and no channels or material".into())),
    }
}

/// How the fill at `target` is projected and sampled: every fill has both
pub(crate) fn placement_mut(s: &mut Session, target: FillTarget) -> Result<(&mut Projection, &mut Sampling)> {
    if let FillTarget::Mask(effect) = target {
        return match s.doc.project.effect_mut(effect).map(|found| &mut found.kind) {
            Some(EffectKind::Fill { projection, sampling, .. }) => Ok((projection, sampling)),
            _ => Err(EngineError::Other("not a mask fill effect".into())),
        };
    }

    let fill = fill_mut(s, target)?;
    Ok((&mut fill.projection, &mut fill.sampling))
}

/// The grayscale source of the mask fill effect `effect`
fn mask_source_mut(s: &mut Session, effect: LayerId) -> Result<&mut Source> {
    match s.doc.project.effect_mut(effect).map(|found| &mut found.kind) {
        Some(EffectKind::Fill { source, .. }) => Ok(source),
        _ => Err(EngineError::Other("not a mask fill effect".into())),
    }
}

fn fill_set(session: &mut Session, params: &Value) -> Result<Value> {
    if let Some(input) = params.get("input") {
        let input = input.as_str().ok_or_else(|| bad(FILL_SET, "`input` must be a filter input name"))?;
        return input_set(session, params, input);
    }

    let target = fill_target(session, params, FILL_SET)?;
    if let FillTarget::Mask(effect) = target {
        return mask_fill_set(session, params, effect);
    }

    let channel = channel_param(FILL_SET, params, "channel")?.ok_or_else(|| bad(FILL_SET, "`channel` is required"))?;

    // `solo`: this channel on, every other one off
    let solo = boolp(params, "solo") == Some(true);
    let enabled = if solo { Some(true) } else { boolp(params, "enabled") };
    let normal_format = normal_format_param(FILL_SET, params)?;
    let colors = ramp_param(params, channel)?;
    let init_ramp = boolp(params, "gradientMap") == Some(true);
    if init_ramp && colors.is_some() {
        return Err(bad(FILL_SET, "give `gradientMap` or `colors`, not both"));
    }

    let source = fill_source_from(session, params, (target, channel), normal_format)?;

    // A library alpha that became the source joins the project's images
    let library_alpha = match &source {
        Some(Source::Image { name, .. }) => crate::fill_alpha::library_alpha_of(name).filter(|_| strp(params, "image").is_none()),
        _ => None,
    };
    if source.is_none() && enabled.is_none() && normal_format.is_none() && colors.is_none() && !init_ramp {
        return Err(bad(
            FILL_SET,
            "give `color`, `value`, `procedural`, `image`, `alpha`, `anchor`, pattern settings (`tile`, `resolution`…), `colors`, `gradientMap`, `normalFormat`, `enabled` or `solo`",
        ));
    }

    if normal_format.is_some() && source.as_ref().is_some_and(|source| !matches!(source, Source::Image { .. })) {
        return Err(bad(FILL_SET, "`normalFormat` goes with `image` or alone"));
    }

    // Changing only the normal format or gradient stops leaves the layer channel on or off
    let touches_channel = source.is_some() || enabled.is_some();
    let format_existing_image = normal_format.filter(|_| source.is_none());
    let replaces_material = source.is_some() || colors.is_some() || init_ramp;
    let current = fill_mut(session, target)?.sources.get(&channel);
    let colors = if init_ramp { initial_ramp(channel, source.as_ref(), current)? } else { colors };
    let mut source = with_ramp(channel, current, source, colors)?;

    session.edit("Set fill", |session| {
        if let (Some(alpha), Some(Source::Image { name, .. })) = (library_alpha, &mut source) {
            *name = session.add_library_alpha(alpha)?;
        }

        let fill = fill_mut(session, target)?;

        // `normalFormat` alone: the convention of the channel's image
        if let Some(format) = format_existing_image {
            match source.as_mut().or_else(|| fill.sources.get_mut(&channel)) {
                Some(Source::Image { normal_format, .. }) => *normal_format = format,
                _ => return Err(bad(FILL_SET, "`normalFormat` needs an image on the channel")),
            }
        }

        // A channel set on its own (its source or its gradient map): the sources are no longer one whole material
        if replaces_material {
            fill.material = None;
        }

        match target {
            FillTarget::Mask(_) => return Err(bad(FILL_SET, "a mask fill has no channels")),
            // A fill effect's channel is on while it has a source
            FillTarget::Effect(_) if enabled == Some(false) => {
                fill.sources.remove(&channel);
            }

            // Removing stops (`colors: []`) from a channel the fill does not have adds nothing
            FillTarget::Effect(_) if source.is_none() && enabled != Some(true) && !fill.sources.contains_key(&channel) => {}
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

/// `layer.fill.set` on mask fill effect `effect`: its one grayscale source (no `channel`, `enabled`, `solo` or `normalFormat`)
fn mask_fill_set(s: &mut Session, p: &Value, effect: LayerId) -> Result<Value> {
    refuse_gray_keys(p, "a mask fill")?;

    let current = mask_source_mut(s, effect)?.clone();
    let (mut source, library_alpha) = gray_source(s, p, effect, &current)?;

    // Setting what the fill already reads adds no undo step
    if gray_source_is_current(s, Some(&current), Some(&source), library_alpha) {
        return Ok(Value::Null);
    }

    s.edit("Set fill", |s| {
        if let (Some(alpha), Source::Image { name, .. }) = (library_alpha, &mut source) {
            *name = s.add_library_alpha(alpha)?;
        }

        *mask_source_mut(s, effect)? = source;
        Ok(())
    })?;
    Ok(Value::Null)
}

/// `layer.fill.set {effect, input}`: the source of a filter effect's image input `id`, or its default gray back (`clear`)
fn input_set(s: &mut Session, p: &Value, id: &str) -> Result<Value> {
    let effect = u64p(p, "effect").map(LayerId).ok_or_else(|| bad(FILL_SET, "`input` goes with a filter `effect`"))?;
    let found = s.doc.project.effect(effect).map(|(_, found)| found).ok_or_else(|| bad(FILL_SET, format!("no effect {}", effect.0)))?;
    let EffectKind::Filter(filter) = &found.kind else { return Err(bad(FILL_SET, format!("{} is not a filter effect", effect.0))) };
    let input = ImageInput::named(filter, id).ok_or_else(|| bad(FILL_SET, format!("the {} filter has no image input `{id}`", filter.label())))?;
    refuse_gray_keys(p, "an image input")?;

    let (source, library_alpha) = if p.get("clear").and_then(Value::as_bool) == Some(true) {
        (None, None)
    } else {
        let (source, alpha) = gray_source(s, p, effect, &found.input(&input))?;
        (Some(source), alpha)
    };

    // Its default gray is not stored, so a project that never set the input saves as before
    let mut stored = source.filter(|source| *source != input.default_source());

    // Setting what the input already reads adds no undo step
    if gray_source_is_current(s, found.inputs.get(input.id), stored.as_ref(), library_alpha) {
        return Ok(Value::Null);
    }

    s.edit("Set image input", |s| {
        if let (Some(alpha), Some(Source::Image { name, .. })) = (library_alpha, &mut stored) {
            *name = s.add_library_alpha(alpha)?;
        }

        let found = s.doc.project.effect_mut(effect).ok_or_else(|| bad(FILL_SET, "the effect is gone"))?;
        match stored {
            Some(source) => found.inputs.insert(input.id.to_string(), source),
            None => found.inputs.remove(input.id),
        };
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Refuses the [`GRAY_REFUSED_KEYS`] for `subject`, a fill with one grayscale source
fn refuse_gray_keys(p: &Value, subject: &str) -> Result<()> {
    if GRAY_REFUSED_KEYS.iter().any(|key| p.get(key).is_some()) {
        let keys = GRAY_REFUSED_KEYS.map(|key| format!("`{key}`")).join(", ");
        return Err(bad(FILL_SET, format!("{subject} has one grayscale source: leave out {keys}")));
    }

    Ok(())
}

/// Whether the current source already reads the requested value or library alpha
fn gray_source_is_current(s: &Session, current: Option<&Source>, requested: Option<&Source>, library_alpha: Option<&LibraryAlpha>) -> bool {
    if let Some(alpha) = library_alpha {
        return matches!(current, Some(Source::Image { name, .. }) if s.library_alpha(name).is_some_and(|current_alpha| current_alpha.id == alpha.id));
    }

    current == requested
}

/// The grayscale source `p` asks for in place of `current` (read by a mask or filter effect `effect`), and the library alpha it needs added
fn gray_source(s: &Session, p: &Value, effect: LayerId, current: &Source) -> Result<(Source, Option<&'static LibraryAlpha>)> {
    let low = color_param(FILL_SET, p, "low")?;
    let high = color_param(FILL_SET, p, "high")?;
    let color = color_param(FILL_SET, p, "color")?;
    let value = color_param(FILL_SET, p, "value")?;
    if let Some(kind) = strp(p, "procedural") {
        let kind = ProceduralKind::parse(kind).ok_or_else(|| bad(FILL_SET, format!("unknown procedural `{kind}`")))?;
        Ok((procedural_from(p, kind, Some(current), [low, high])?, None))
    } else if let Some(gray) = color.or(value) {
        Ok((Source::Uniform(gray.map(|component| component.clamp(0.0, 1.0))), None))
    } else if let Some(name) = strp(p, "image") {
        Ok((crate::mask_cmds::image_source(s, FILL_SET, name)?, None))
    } else if let Some(name) = strp(p, "alpha") {
        let alpha = alphas::find(name).ok_or_else(|| bad(FILL_SET, format!("unknown alpha `{name}`")))?;
        Ok((Source::image(crate::fill_alpha::resource_name(alpha)), Some(alpha)))
    } else if let Some(anchor) = u64p(p, "anchor") {
        let reader = s.doc.project.effect(effect).map(|(owner, _)| owner).ok_or_else(|| bad(FILL_SET, "the effect is not in a layer"))?;

        // A gray reads one channel of the anchor's stack: the one it read, else Height
        let kept = match current {
            Source::Anchor { channel, .. } => *channel,
            _ => Channel::Height,
        };
        let channel = channel_param(FILL_SET, p, "anchorChannel")?.unwrap_or(kept);
        Ok((crate::mask_cmds::anchor_source(s, FILL_SET, reader, LayerId(anchor), channel)?, None))
    } else if edits_fill_pattern(p) {
        // Pattern settings alone edit the current pattern
        let Source::Procedural { pattern, .. } = current else {
            return Err(bad(FILL_SET, "the source has no pattern: give `procedural` with its settings"));
        };

        Ok((procedural_from(p, pattern.kind, Some(current), [low, high])?, None))
    } else {
        Err(bad(FILL_SET, "give `value`, `procedural`, `image`, `alpha` or `anchor`"))
    }
}

fn edits_fill_pattern(params: &Value) -> bool {
    crate::procedural_params::has_pattern_settings(params)
        || ["scale", "seed", "contrast", "balance", "invert", "low", "high"].iter().any(|key| params.get(key).is_some())
}

/// The procedural source `params` ask for with `low`/`high` already parsed: a pattern of the same kind as `current` keeps what `params` do not give; another kind starts from its defaults
fn procedural_from(params: &Value, kind: ProceduralKind, current: Option<&Source>, [low, high]: [Option<[f32; 3]>; 2]) -> Result<Source> {
    let (mut pattern, current_low, current_high) = match current {
        Some(Source::Procedural { pattern, low, high, .. }) if pattern.kind == kind => (pattern.clone(), *low, *high),
        _ => (Procedural::new(kind), [0.0; 3], [1.0; 3]),
    };

    crate::procedural_params::pattern_basics_from(params, &mut pattern);
    crate::procedural_params::pattern_settings_from(FILL_SET, params, &mut pattern)?;
    Ok(Source::Procedural { pattern, low: low.unwrap_or(current_low), high: high.unwrap_or(current_high), ramp: Vec::new() })
}

/// The source `params` give fill channel `channel` of `target`, or None when they only switch it or set its image's normal format
fn fill_source_from(
    session: &mut Session,
    params: &Value,
    (target, channel): (FillTarget, Channel),
    normal_format: Option<NormalFormat>,
) -> Result<Option<Source>> {
    let low = color_param(FILL_SET, params, "low")?;
    let high = color_param(FILL_SET, params, "high")?;
    let uniform = color_param(FILL_SET, params, "color")?.or(color_param(FILL_SET, params, "value")?);

    if let Some(name) = strp(params, "procedural") {
        let kind = ProceduralKind::parse(name).ok_or_else(|| bad(FILL_SET, format!("unknown procedural `{name}`")))?;

        // A pattern of the same kind keeps what `params` do not give; another kind starts from its defaults
        let current = fill_mut(session, target)?.sources.get(&channel);
        return Ok(Some(procedural_from(params, kind, current, [low, high])?));
    }

    if let Some(color) = uniform {
        return Ok(Some(Source::Uniform(color)));
    }

    if let Some(name) = strp(params, "image") {
        return Ok(Some(Source::Image { name: name.to_string(), normal_format: normal_format.unwrap_or_default() }));
    }

    if let Some(name) = strp(params, "alpha") {
        let alpha = alphas::find(name).ok_or_else(|| bad(FILL_SET, format!("unknown alpha `{name}`")))?;
        return Ok(Some(Source::Image { name: crate::fill_alpha::resource_name(alpha), normal_format: normal_format.unwrap_or_default() }));
    }

    if let Some(anchor) = u64p(params, "anchor") {
        let reader = match target {
            FillTarget::Layer(id) => Some(id),
            FillTarget::Effect(effect) | FillTarget::Mask(effect) => session.doc.project.effect(effect).map(|(owner, _)| owner),
        };
        let reader = reader.ok_or_else(|| bad(FILL_SET, "the fill is not in a layer"))?;
        let anchor_channel = channel_param(FILL_SET, params, "anchorChannel")?.unwrap_or(channel);
        return Ok(Some(crate::mask_cmds::anchor_source(session, FILL_SET, reader, LayerId(anchor), anchor_channel)?));
    }

    if !edits_fill_pattern(params) {
        return Ok(None);
    }

    // Pattern settings alone edit the channel's current pattern
    let current = fill_mut(session, target)?.sources.get(&channel);
    let Some(Source::Procedural { pattern, .. }) = current else {
        return Err(bad(FILL_SET, "the channel has no pattern: give `procedural` with its settings"));
    };

    Ok(Some(procedural_from(params, pattern.kind, current, [low, high])?))
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

/// A new fill's projection of kind `id` (`uv`, `uvTile`, `triplanar`, `planar`, `spherical`, `cylindrical`, or a `warp` from frame `fit`)
pub(crate) fn new_projection(id: &str, fit: impl FnOnce() -> Decal) -> Option<Projection> {
    match id {
        "warp" => Some(Projection::warp(fit())),
        _ => new_unframed_projection(id),
    }
}

/// The ids [`new_unframed_projection`] takes, one per kind
pub(crate) const UNFRAMED_PROJECTION_IDS: [&str; 6] = ["uv", "uvTile", "triplanar", "planar", "spherical", "cylindrical"];

/// A new fill's projection of kind `id`, any kind but a warp (which needs a frame)
pub(crate) fn new_unframed_projection(id: &str) -> Option<Projection> {
    let projection = match id {
        "uv" => Projection::default(),
        "uvTile" => Projection::UvTile,
        "triplanar" | "tri-planar" => Projection::triplanar(),
        "planar" => Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: Default::default(), projector: Projector::depth_culled(), placed: None },
        "spherical" => Projection::Spherical { scale: 1.0, uv: Default::default(), offset: [0.0; 3], projector: Default::default() },
        "cylindrical" => {
            Projection::Cylindrical { scale: 1.0, axis: 1, uv: Default::default(), offset: [0.0; 3], projector: Default::default(), angle: FULL_TURN_DEGREES }
        }
        _ => return None,
    };
    Some(projection)
}

/// The projection `p` asks for: fields it does not give keep their values from `current` when that is of the same kind (or `p` names no kind), and the kind's defaults otherwise; a new warp starts from frame `fit`
pub(crate) fn projection_from(p: &Value, cmd: &str, current: Option<&Projection>, fit: impl FnOnce() -> Decal) -> Result<Projection> {
    let named = match strp(p, "projection") {
        None => None,
        Some("warp") if matches!(current, Some(Projection::Warp { .. })) => current.cloned(),
        Some(id) => Some(new_projection(id, fit).ok_or_else(|| bad(cmd, format!("unknown projection `{id}`")))?),
    };
    let mut projection = match (named, current) {
        (Some(named), Some(current)) if std::mem::discriminant(&named) == std::mem::discriminant(current) => current.clone(),
        (Some(named), _) => named,
        (None, current) => current.cloned().unwrap_or_default(),
    };
    let scale = floats::<2>(p, "scale", MIN_SCALE, MAX_PLACEMENT);
    let tiling = floats::<2>(p, "tiling", MIN_SCALE, MAX_PLACEMENT);
    let rotation = f32p(p, "rotation").map(|degrees| degrees.rem_euclid(FULL_TURN_DEGREES));
    let axis = u64p(p, "axis").map(|axis| axis.min(2) as u8);

    // The UV transformations of a 3D projection: tiling, rotation and an offset across the surface
    let place = |uv: &mut UvTransform| {
        uv.tiling = tiling.unwrap_or(uv.tiling);
        uv.rotation = rotation.unwrap_or(uv.rotation);
        uv.offset = floats(p, "uvOffset", -MAX_PLACEMENT, MAX_PLACEMENT).unwrap_or(uv.offset);
    };

    // The 3D projection settings: the projector's offset, turn and per-axis scale
    let projector_offset = floats::<3>(p, "offset", -MAX_PLACEMENT, MAX_PLACEMENT);
    let crop = match strp(p, "shapeCrop") {
        None => None,
        Some(id) => Some(ShapeCrop::from_id(id).ok_or_else(|| bad(cmd, format!("unknown shapeCrop `{id}`: cropped or extendsOutside")))?),
    };
    let turn = |projector: &mut Projector| {
        projector.rotation = floats(p, "projectorRotation", -FULL_TURN_DEGREES, FULL_TURN_DEGREES).unwrap_or(projector.rotation);
        projector.scale = floats(p, "projectorScale", -MAX_PLACEMENT, MAX_PLACEMENT).map_or(projector.scale, |scale| scale.map(signed_scale));
        projector.crop = crop.unwrap_or(projector.crop);
        projector.backface.on = p.get("backfaceCulling").and_then(Value::as_bool).unwrap_or(projector.backface.on);
        projector.backface.angle = f32p(p, "backfaceAngle").map_or(projector.backface.angle, |angle| clamp_to(angle, &Backface::ANGLE_RANGE));
        projector.backface.hardness = f32p(p, "backfaceHardness").map_or(projector.backface.hardness, |hardness| hardness.clamp(0.0, 1.0));
        projector.depth.on = p.get("depthCulling").and_then(Value::as_bool).unwrap_or(projector.depth.on);
        projector.depth.hardness = f32p(p, "depthCullingHardness").map_or(projector.depth.hardness, |hardness| hardness.clamp(0.0, 1.0));
    };
    match &mut projection {
        Projection::Uv { scale: uv_scale, rotation: uv_rotation, offset } => {
            // A UV projection's scale is its tiling
            *uv_scale = tiling.or(scale).unwrap_or(*uv_scale);
            *uv_rotation = rotation.unwrap_or(*uv_rotation);
            *offset = floats(p, "offset", -MAX_PLACEMENT, MAX_PLACEMENT).unwrap_or(*offset);
        }
        Projection::UvTile => {}
        Projection::Triplanar { scale: projection_scale, sharpness, offset, uv, projector } => {
            *projection_scale = scale.map_or(*projection_scale, |across| across[0]);
            *sharpness = f32p(p, "sharpness").map_or(*sharpness, |given| clamp_to(given, &Projection::TRIPLANAR_SHARPNESS_RANGE));
            *offset = projector_offset.unwrap_or(*offset);
            place(uv);
            turn(projector);
        }
        Projection::Planar { scale: projection_scale, axis: projection_axis, offset, uv, projector, .. } => {
            *projection_scale = scale.map_or(*projection_scale, |across| across[0]);
            *projection_axis = axis.unwrap_or(*projection_axis);
            *offset = projector_offset.unwrap_or(*offset);
            place(uv);
            turn(projector);
        }
        Projection::Spherical { scale: projection_scale, uv, offset, projector } => {
            *projection_scale = scale.map_or(*projection_scale, |across| across[0]);
            *offset = projector_offset.unwrap_or(*offset);
            place(uv);
            turn(projector);
        }
        Projection::Cylindrical { scale: projection_scale, axis: projection_axis, uv, offset, projector, angle } => {
            *projection_scale = scale.map_or(*projection_scale, |across| across[0]);
            *angle = f32p(p, "angle").map_or(*angle, |given| clamp_to(given, &Projection::CYLINDER_ANGLE_RANGE));
            *projection_axis = axis.unwrap_or(*projection_axis);
            *offset = projector_offset.unwrap_or(*offset);
            place(uv);
            turn(projector);
        }
        Projection::Warp { corners, depth, uv, projector, .. } => {
            // A warp has no scale or offset of its own: its frame places it, so they would change nothing
            if p.get("scale").is_some() || p.get("offset").is_some() {
                return Err(bad(cmd, "a warp is placed by its frame: set frameOffset or frameScale with layer.decal.set"));
            }

            *corners = warp_corners(p, cmd)?.unwrap_or(*corners);
            *depth = f32p(p, "projectionDepth").map_or(*depth, |given| given.clamp(0.0, MAX_PROJECTION_DEPTH));
            place(uv);
            turn(projector);
        }
    }
    Ok(projection)
}

/// `value` held within `range`
fn clamp_to(value: f32, range: &std::ops::RangeInclusive<f32>) -> f32 {
    value.clamp(*range.start(), *range.end())
}

/// The projection `params` asks for, as [`projection_from`]; without `read_scale` its `scale` is left out (a procedural mask's scale is its pattern's)
pub(crate) fn projection_from_fields(
    params: &Value,
    command: &str,
    current: Option<&Projection>,
    read_scale: bool,
    fit: impl FnOnce() -> Decal,
) -> Result<Projection> {
    let mut placement = params.clone();
    if !read_scale && let Some(fields) = placement.as_object_mut() {
        fields.remove("scale");
    }
    projection_from(&placement, command, current, fit)
}

/// The `warpCorners` param: four [across, up] corners in frame sizes from the frame's centre, finite and within reach of the frame (None when not given)
fn warp_corners(p: &Value, cmd: &str) -> Result<Option<[[f32; 2]; 4]>> {
    let Some(given) = p.get("warpCorners") else { return Ok(None) };

    let malformed = || bad(cmd, "warpCorners is four [across, up] pairs of finite numbers");
    let list = given.as_array().filter(|list| list.len() == 4).ok_or_else(malformed)?;
    let mut corners = Projection::FRAME_CORNERS;
    for (corner, value) in corners.iter_mut().zip(list) {
        let pair = value.as_array().filter(|pair| pair.len() == 2).ok_or_else(malformed)?;
        for (axis, number) in corner.iter_mut().zip(pair) {
            let reach = number.as_f64().map(|number| number as f32).filter(|number| number.is_finite()).ok_or_else(malformed)?;
            *axis = reach.clamp(-MAX_CORNER_REACH, MAX_CORNER_REACH);
        }
    }
    Ok(Some(corners))
}

/// `current` with the `filtering`, `wrap`, `scaleMode` and symmetry params applied
pub(crate) fn sampling_from(p: &Value, cmd: &str, current: Sampling) -> Result<Sampling> {
    let mut sampling = current;
    if let Some(id) = strp(p, "filtering") {
        sampling.filtering = Filtering::from_id(id).ok_or_else(|| bad(cmd, format!("unknown filtering `{id}`: bilinearHq, bilinearSharp or nearest")))?;
    }
    if let Some(id) = strp(p, "wrap") {
        sampling.wrap = UvWrap::from_id(id).ok_or_else(|| bad(cmd, format!("unknown wrap `{id}`: repeat, repeatHorizontally, repeatVertically or none")))?;
    }
    if let Some(id) = strp(p, "scaleMode") {
        sampling.scale = ScaleMode::from_id(id).ok_or_else(|| bad(cmd, format!("unknown scaleMode `{id}`: tiling or physicalSize")))?;
    }
    sampling.symmetry = symmetry_from(p, cmd, sampling.symmetry)?;
    Ok(sampling)
}

/// `current` with the `symmetry`, `symmetryType`, `symmetryAxis`, `flipCopy`, `axisPosition`, `symmetryCount` and `angleSpan` params applied
fn symmetry_from(p: &Value, cmd: &str, current: FillSymmetry) -> Result<FillSymmetry> {
    let mut symmetry = current;
    symmetry.on = boolp(p, "symmetry").unwrap_or(symmetry.on);
    if let Some(id) = strp(p, "symmetryType") {
        symmetry.kind = SymmetryKind::from_id(id).ok_or_else(|| bad(cmd, format!("unknown symmetryType `{id}`: mirror or radial")))?;
    }
    if let Some(given) = p.get("symmetryAxis").filter(|given| !given.is_null()) {
        // An index or the letter tool.brush takes for its own symmetry axis
        let axis = match given {
            Value::String(letter) => ["x", "y", "z"].iter().position(|name| letter.eq_ignore_ascii_case(name)).and_then(|index| u8::try_from(index).ok()),
            index => index.as_u64().and_then(|index| u8::try_from(index).ok()).filter(|index| *index <= FillSymmetry::LAST_AXIS),
        };
        symmetry.axis = axis.ok_or_else(|| bad(cmd, "symmetryAxis is 0 (X), 1 (Y) or 2 (Z), or \"x\", \"y\" or \"z\""))?;
    }
    if let Some(flips) = p.get("flipCopy") {
        let [Some(across), Some(down)] = flips.as_array().map(Vec::as_slice).unwrap_or_default().iter().map(Value::as_bool).collect::<Vec<_>>()[..] else {
            return Err(bad(cmd, "flipCopy is [u, v], two booleans"));
        };

        symmetry.flip = [across, down];
    }
    let reach = FillSymmetry::REACH;
    symmetry.position = floats(p, "axisPosition", -reach, reach).unwrap_or(symmetry.position);
    if let Some(count) = u64p(p, "symmetryCount") {
        let (fewest, most) = (u64::from(*FillSymmetry::COUNTS.start()), u64::from(*FillSymmetry::COUNTS.end()));
        symmetry.count = u32::try_from(count.clamp(fewest, most)).unwrap_or(*FillSymmetry::COUNTS.start());
    }
    symmetry.span = f32p(p, "angleSpan").map_or(symmetry.span, |span| clamp_to(span, &FillSymmetry::SPANS));
    Ok(symmetry)
}

/// `value` as one axis of a projector's scale: its size kept within [`MIN_SCALE`]..[`MAX_PLACEMENT`] on its own side, so a mirrored axis stays mirrored
pub fn signed_scale(value: f32) -> f32 {
    value.signum() * value.abs().clamp(MIN_SCALE, MAX_PLACEMENT)
}

/// `key` as `N` finite numbers clamped to `lo..=hi`: an array of `N`, or one number for all
fn floats<const N: usize>(p: &Value, key: &str, lo: f32, hi: f32) -> Option<[f32; N]> {
    let number = |value: &Value| value.as_f64().map(|given| given as f32).filter(|given| given.is_finite()).map(|given| given.clamp(lo, hi));
    match p.get(key)? {
        Value::Array(list) if list.len() == N => {
            let mut out = [0.0; N];
            for (slot, value) in out.iter_mut().zip(list) {
                *slot = number(value)?;
            }
            Some(out)
        }
        value => number(value).map(|given| [given; N]),
    }
}

fn fill_projection(s: &mut Session, p: &Value) -> Result<Value> {
    let target = fill_target(s, p, "layer.fill.projection")?;
    let mesh = s.mesh.clone();
    s.edit("Fill projection", |s| {
        let (projection, sampling) = placement_mut(s, target)?;
        *projection = projection_from(p, "layer.fill.projection", Some(projection), || crate::warp_cmds::fitted_frame(&mesh))?;
        *sampling = sampling_from(p, "layer.fill.projection", *sampling)?;
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
