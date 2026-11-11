//! JSON views of the session for agents: everything an agent changed must be visible here
//! without a screenshot.

use serde_json::{Value, json};
use texturecraft_core::{
    Effect, EffectKind, Fill, Filter, Generator, GeometryMask, GraphInput, GraphRef, ImportReport, Layer, LayerKind, LayerReport, NormalFormat, Projection,
    Source,
};

use crate::Session;
use crate::session::StackTarget;

fn source_json(s: &Source) -> Value {
    match s {
        Source::Uniform(v) => json!({ "uniform": v }),
        Source::Procedural { pattern, low, high } => json!({
            "procedural": pattern.kind.label(), "scale": pattern.scale, "seed": pattern.seed, "contrast": pattern.contrast, "low": low, "high": high
        }),
        Source::Image { name, normal_format: NormalFormat::OpenGl } => json!({ "image": name }),
        Source::Image { name, normal_format: NormalFormat::DirectX } => json!({ "image": name, "normalFormat": "directX" }),
        Source::Anchor { anchor, channel } => json!({ "anchor": anchor.0, "channel": channel.id() }),
        Source::Graph(g) => json!({ "graph": graph_json(g) }),
    }
}

/// A graph reference: the graph, its values, what feeds its inputs and the outputs read.
fn graph_json(g: &GraphRef) -> Value {
    let input = |i: &GraphInput| match i {
        GraphInput::Baked(m) => json!({ "baked": m.id() }),
        GraphInput::Constant(c) => json!({ "constant": c }),
        GraphInput::Image(name) => json!({ "image": name }),
        GraphInput::Nested(inner) => json!({ "graph": graph_json(inner) }),
        GraphInput::Stack { channel } => json!({ "stack": channel.map(|c| c.id()) }),
        GraphInput::StackAlpha { channel } => json!({ "stackAlpha": channel.map(|c| c.id()) }),
        GraphInput::Anchor { anchor, channel, levels } => json!({ "anchor": anchor.0, "channel": channel.map(|c| c.id()), "levels": levels.is_some() }),
    };
    let mut v = json!({
        "name": g.name,
        "key": g.key,
        "params": g.params.iter().map(|(k, v)| (k.clone(), json!(v.lanes()))).collect::<serde_json::Map<_, _>>(),
        "inputs": g.inputs.iter().map(|(k, i)| (k.to_string(), input(i))).collect::<serde_json::Map<_, _>>(),
        "seed": g.seed,
        "readsStack": g.reads_stack(),
    });
    if let Some(o) = &g.output {
        v["output"] = json!(o);
    }
    if !g.outputs.is_empty() {
        v["outputs"] = g.outputs.iter().map(|(c, o)| (c.id().to_string(), json!(o))).collect::<serde_json::Map<_, _>>().into();
    }
    if let Some(a) = &g.alpha_output {
        v["alphaOutput"] = json!(a);
    }
    v
}

/// The project's graphs: texts held, cooks, renders made in this session and what could not be shown.
fn graphs_json(s: &Session) -> Value {
    let g = &s.doc.graphs;
    let problems: Vec<Value> = g
        .problems()
        .into_iter()
        .map(
            |(u, p)| json!({ "textureSet": u.set, "id": u.id.0, "channel": u.channel.map(|c| c.id()), "graph": p.graph, "stale": p.stale, "reason": p.reason }),
        )
        .collect();
    json!({ "texts": g.texts.len(), "used": s.doc.project.graph_keys().len(), "cooks": g.used_cooks().len(), "renders": g.renders(), "problems": problems })
}

fn fill_json(f: &Fill) -> Value {
    let mut v = json!({
        "projection": f.projection.label(),
        "sources": f.sources.iter().map(|(c, s)| (c.id().to_string(), source_json(s))).collect::<serde_json::Map<_, _>>(),
    });
    match &f.projection {
        Projection::Decal(d) => v["decal"] = json!(d),
        Projection::Placed(p) => v["placed"] = json!(p),
        _ => {}
    }
    if let Some(link) = &f.link {
        v["link"] = json!(link);
    }
    v
}

/// A mask or content (`content`) effect.
fn effect_json(session: &Session, effect: &Effect, content: bool) -> Value {
    let mut out = json!({ "id": effect.id.0, "name": effect.name, "kind": effect.kind.name(), "visible": effect.visible, "blend": effect.blend.label(), "opacity": effect.opacity });
    let mask_tiles = || session.doc.pixels.masks.get(&effect.id).map_or(0, |image| image.allocated_count());
    let content_tiles = || session.doc.pixels.layers.iter().filter(|((id, _), _)| *id == effect.id).map(|(_, image)| image.allocated_count()).sum::<usize>();
    if content && !effect.channels.is_empty() {
        out["channels"] = json!(effect.channels.iter().map(|c| c.id()).collect::<Vec<_>>());
    }

    match &effect.kind {
        EffectKind::Generator(generator) => out["generator"] = generator_json(generator),
        EffectKind::Levels(levels) => out["levels"] = json!([levels.in_low, levels.in_high, levels.gamma, levels.out_low, levels.out_high]),
        EffectKind::ColorSelection { colors, tolerance } => {
            out["colors"] = json!(colors);
            out["tolerance"] = json!(tolerance);
        }
        EffectKind::Path(paths) => {
            out["paths"] = json!(
                paths.iter().map(|path| json!({ "points": path.points.len(), "uv": path.uv, "closed": path.closed, "fill": path.fill })).collect::<Vec<_>>()
            );
            out["tiles"] = json!(if content { content_tiles() } else { mask_tiles() });
        }
        EffectKind::Paint if content => out["tiles"] = json!(content_tiles()),
        EffectKind::Paint => out["tiles"] = json!(mask_tiles()),
        EffectKind::Fill { source, .. } => out["source"] = source_json(source),
        EffectKind::Material(fill) => out["fill"] = fill_json(fill),
        EffectKind::Placeholder { label, reason } => out["placeholder"] = json!({ "label": label, "reason": reason }),
        EffectKind::Graph { graph, projection } => {
            out["graph"] = graph_json(graph);
            out["projection"] = json!(projection.label());
        }
        EffectKind::Filter(filter) => {
            out["filter"] = json!(filter);
            match filter {
                Filter::Hsl { hue, saturation, lightness } => out["hsl"] = json!([hue, saturation, lightness]),
                Filter::Blur { radius } => out["radius"] = json!(radius),
                _ => {}
            }
        }
        _ => {}
    }
    out
}

/// A generator's settings, with its curvature mode and detail size weights when it has them.
fn generator_json(generator: &Generator) -> Value {
    let mut out = json!({ "type": format!("{:?}", generator.kind), "level": generator.level, "softness": generator.softness, "noise": generator.noise_amount, "offset": generator.offset, "invert": generator.invert });
    if let Some(detail) = &generator.detail {
        out["mode"] = json!(format!("{:?}", detail.mode));
        out["sizes"] = json!(detail.weights);
    }
    out
}

fn layer_json(s: &Session, l: &Layer, depth: usize) -> Value {
    let channels: serde_json::Map<String, Value> =
        l.channels.iter().map(|(c, b)| (c.id().to_string(), json!({ "mode": b.mode.label(), "opacity": b.opacity }))).collect();
    let mut v = json!({
        "id": l.id.0,
        "name": l.name,
        "kind": l.kind.name(),
        "visible": l.visible,
        "channels": channels,
        "colorTag": l.color_tag,
    });
    if let LayerKind::Fill(f) = &l.kind {
        v["fill"] = fill_json(f);
    }
    if let Some(src) = l.instance_source() {
        let source = s.doc.project.layers.get(&src);
        let set = s.doc.project.locate(src).map(|(ts, _, _)| ts);
        v["instanceOf"] = json!({ "layer": src.0, "name": source.map(|x| x.name.as_str()), "textureSet": set });
    }
    if l.height_mask.enabled {
        v["heightMask"] = json!({ "contrast": l.height_mask.contrast, "offset": l.height_mask.offset });
    }
    match &l.geometry_mask {
        Some(GeometryMask::Parts(parts)) => v["geometryMask"] = json!(parts),
        Some(GeometryMask::UvTiles(tiles)) => v["geometryMask"] = json!({ "uvTiles": tiles }),
        None => {}
    }
    if let Some(lock) = &l.lock {
        v["locked"] = crate::lock_cmds::lock_json(lock);
    }
    if let Some(r) = s.doc.project.import.as_ref().and_then(|r| r.layers.get(&l.id)) {
        v["import"] = layer_report_json(r);
    }
    if matches!(l.kind, LayerKind::Paint) {
        // Sorted: the store is a hash map, and inspect output must be stable.
        let mut painted: Vec<(&str, usize)> =
            s.doc.pixels.layers.iter().filter(|((id, _), _)| *id == l.id).map(|((_, c), img)| (c.id(), img.allocated_count())).collect();
        painted.sort_unstable();
        v["painted"] = json!(painted.iter().map(|(c, n)| json!({ "channel": c, "tiles": n })).collect::<Vec<_>>());
    }
    if let Some(m) = &l.mask {
        v["mask"] = json!({
            "enabled": m.enabled,
            "background": format!("{:?}", m.background).to_lowercase(),
            "effects": m.effects.iter().map(|e| effect_json(s, e, false)).collect::<Vec<_>>(),
        });
    }
    if !l.content_effects.is_empty() {
        v["contentEffects"] = json!(l.content_effects.iter().map(|e| effect_json(s, e, true)).collect::<Vec<_>>());
    }
    if depth < texturecraft_core::project::MAX_DEPTH {
        let kids: Vec<Value> = l.children().iter().rev().filter_map(|c| s.doc.project.layers.get(c)).map(|c| layer_json(s, c, depth + 1)).collect();
        if l.is_group() {
            v["children"] = json!(kids);
        }
    }
    v
}

fn layer_report_json(r: &LayerReport) -> Value {
    json!({
        "role": format!("{:?}", r.role),
        "fidelity": r.fidelity.id(),
        "path": r.path,
        "flags": r.flags.iter().map(|f| json!({ "kind": format!("{:?}", f.kind), "key": f.key, "text": f.text })).collect::<Vec<_>>(),
    })
}

/// The import report of a converted project: the item lines, counts per fidelity, the layers (with what the stack
/// still holds of them), and the locked layers and placeholder effects of the whole project.
pub fn import_report(s: &Session, r: &ImportReport) -> Value {
    let p = &s.doc.project;
    let counts: serde_json::Map<String, Value> = texturecraft_core::Fidelity::ALL.iter().map(|f| (f.id().to_string(), json!(r.count(*f)))).collect();
    json!({
        "source": r.source,
        "importer": r.importer,
        "exact": r.is_exact(),
        "counts": counts,
        "entries": r.entries.iter().map(|e| json!({ "item": e.item, "fidelity": e.fidelity.id(), "note": e.note })).collect::<Vec<_>>(),
        "layers": r.layers.iter().map(|(id, l)| {
            let mut v = layer_report_json(l);
            v["layer"] = json!(id.0);
            v["present"] = json!(p.locate(*id).is_some());
            v
        }).collect::<Vec<_>>(),
        "originalResults": r.original_results().map(|id| id.0).collect::<Vec<_>>(),
    })
}

/// Every locked layer and placeholder effect of the project (any texture set), for agents checking an import.
pub fn locks(s: &Session) -> Value {
    let p = &s.doc.project;
    let in_stack = |l: &&Layer| p.locate(l.id).is_some();
    let locked: Vec<Value> = p
        .layers
        .values()
        .filter(in_stack)
        .filter_map(|l| {
            l.lock.as_ref().map(
                |lock| json!({ "layer": l.id.0, "name": l.name, "textureSet": p.locate(l.id).map(|(ts, _, _)| ts), "lock": crate::lock_cmds::lock_json(lock) }),
            )
        })
        .collect();
    let placeholders: Vec<Value> = p
        .layers
        .values()
        .filter(in_stack)
        .flat_map(|l| {
            l.placeholders().map(move |e| match &e.kind {
                EffectKind::Placeholder { label, reason } => json!({ "effect": e.id.0, "layer": l.id.0, "label": label, "reason": reason }),
                _ => Value::Null,
            })
        })
        .collect();
    json!({ "lockedLayers": locked, "placeholders": placeholders })
}

/// The layer stack of texture set `ts`, top → bottom (as displayed).
pub fn stack(s: &Session, ts: usize) -> Value {
    let Some(set) = s.doc.project.texture_sets.get(ts) else { return Value::Null };
    json!(set.stack.iter().rev().filter_map(|id| s.doc.project.layers.get(id)).map(|l| layer_json(s, l, 0)).collect::<Vec<_>>())
}

pub fn texture_sets(s: &Session) -> Value {
    json!((0..s.set_count()).map(|i| texture_set(s, i)).collect::<Vec<_>>())
}

/// One texture set, including UV tile layout and bake selection
pub fn texture_set(session: &Session, index: usize) -> Value {
    let Some(texture_set) = session.doc.project.texture_sets.get(index) else { return Value::Null };

    let grid = texture_set.tile_grid();
    let tiles: Vec<Value> = texture_set
        .uv_tiles
        .iter()
        .filter_map(|tile| {
            grid?.block(*tile).map(|(x, y)| {
                json!({"udim": tile, "block": [x, y], "resolution": texture_set.tile_resolution(*tile), "bakeEnabled": !texture_set.bake_excluded_tiles.contains(tile)})
            })
        })
        .collect();
    json!({
        "index": index,
        "name": texture_set.name,
        "resolution": texture_set.resolution,
        "size": texture_set.size(),
        "blockResolution": texture_set.block_resolution(),
        "uvTiles": texture_set.uv_tiles,
        "tiles": tiles,
        "channels": texture_set
            .channels
            .iter()
            .map(|(channel, format)| match texture_set.channel_names.get(channel) {
                Some(name) => json!({ "channel": channel.id(), "format": format.label(), "name": name }),
                None => json!({ "channel": channel.id(), "format": format.label() }),
            })
            .collect::<Vec<_>>(),
        "layers": session.doc.project.walk(index).len(),
        "parts": crate::set_cmds::parts(session, index),
        "baked": crate::project_cmds::baked_json(session.doc.baked.get(&index)),
        "visible": texture_set.visible,
        "active": index == session.ts(),
    })
}

/// The whole session.
pub fn session(s: &Session) -> Value {
    let ts = s.ts();
    json!({
        "project": s.doc.project.name,
        "mesh": { "source": s.doc.project.mesh, "triangles": s.mesh.triangle_count(), "vertices": s.mesh.vertex_count(), "materials": s.mesh.materials },
        "path": s.path,
        "modified": s.modified,
        "textureSets": texture_sets(s),
        "bakeMeshes": crate::bake::meshes_json(s),
        "activeTextureSet": ts,
        "selectedLayer": s.selected_layer().map(|l| l.0),
        "target": match s.target { StackTarget::Content => json!("content"), StackTarget::Mask => json!("mask"), StackTarget::Effect(e) => json!(e.0) },
        "layers": stack(s, ts),
        "tool": { "tool": s.tool.tool.id(), "brush": crate::tool_cmds::brush_json(s) },
        "view": s.view.label(),
        "bakePreview": crate::bake_preview::preview_json(s),
        "camera": crate::view_cmds::camera_json(s),
        "display": s.doc.project.display,
        "history": { "undo": s.undo_label(), "redo": s.redo_label(), "depth": s.history_labels().len() },
        "memoryBytes": s.memory().total(),
        "pendingTiles": s.pending_tiles(),
        "locks": locks(s),
        "graphs": graphs_json(s),
        "import": s.doc.project.import.as_ref().map_or(Value::Null, |r| import_report(s, r)),
    })
}
