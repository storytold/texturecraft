//! Texture set commands (Texture Set List / Texture Set Settings).

use serde_json::{Value, json};
use texturecraft_core::{Channel, ChannelBlend, ChannelFormat};

use std::sync::Arc;

use texturecraft_geom::Mesh;

use crate::commands::{CommandSpec, always, bad, boolp, channel_param, f32p, normal_format_param, strp, u64p};
use crate::session::{Document, SetCache, StackTarget, check_resolution};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("textureSet.select", "Select texture set", [], None, "{ts}", always, select),
        cmd!("textureSet.next", "Next texture set", [], None, "{}", always, |s, _| step(s, true)),
        cmd!("textureSet.previous", "Previous texture set", [], None, "{}", always, |s, _| step(s, false)),
        cmd!("textureSet.list", "List texture sets", [], None, "{}", always, |s, _| Ok(crate::inspect::texture_sets(s))),
        cmd!(
            "textureSet.inspect",
            "Inspect texture set",
            [],
            None,
            "{ts?} → {index, name, resolution, size: [w, h], uvTiles: [UDIM numbers] (empty: no UV tiles), tiles: [{udim, block: [x, y]}], channels, layers, baked, visible, active}",
            always,
            |s, p| Ok(crate::inspect::texture_set(s, ts_param(s, p, "textureSet.inspect")?))
        ),
        cmd!(
            "textureSet.resolution",
            "Texture set size",
            [],
            None,
            "{ts?, resolution: 128..8192 (power of two), all?: true (every texture set, one undo step each)}",
            always,
            resolution
        ),
        cmd!("textureSet.channel.add", "Add channel", [], None, "{ts?, channel}", always, |s, p| channel(s, p, true)),
        cmd!("textureSet.channel.remove", "Remove channel", [], None, "{ts?, channel}", always, |s, p| channel(s, p, false)),
        cmd!(
            "textureSet.channel.format",
            "Change channel format",
            [],
            None,
            "{ts?, channel, format: sRGB8|RGB8|RGB16|RGB16F|RGB32F for colour and normal channels, L8|L16|L16F|L32F for grey ones} — converts the channel's layer data (8 bit rounds it to 256 steps); one undo step",
            always,
            channel_format
        ),
        cmd!("textureSet.visible", "Toggle texture set visibility", [], None, "{ts?, visible?}", always, visible),
        cmd!(
            "textureSet.visibility",
            "Show or hide texture sets",
            [],
            None,
            "{mode: showAll|hideAll|invert|solo, ts?} → {visible: [bool per set]}; solo shows only `ts` (the active set), or every set again when it already was the only one",
            always,
            visibility
        ),
        cmd!("textureSet.rename", "Rename texture set", [], None, "{ts?, name}", always, rename),
        cmd!("textureSet.settings", "Texture set settings", [], None, "{ts?, description?, heightStrength?, normalFormat?: openGl|directX}", always, settings),
        cmd!(
            "textureSet.parts",
            "List mesh parts",
            [],
            None,
            "{ts?} — the mesh parts (submesh names) of a texture set, for geometry masks",
            always,
            |s, p| {
                let ts = ts_param(s, p, "textureSet.parts")?;
                Ok(json!(parts(s, ts)))
            }
        ),
    ]
}

/// The mesh parts (submesh names, in mesh order, each once) of texture set `ts`.
pub fn parts(s: &Session, ts: usize) -> Vec<String> {
    let Some(material) = s.doc.project.texture_sets.get(ts).map(|t| t.material) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for sm in s.mesh.submeshes.iter().filter(|sm| sm.material == material) {
        if !out.contains(&sm.name) {
            out.push(sm.name.clone());
        }
    }
    out
}

pub(crate) fn ts_param(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    match u64p(p, "ts") {
        Some(t) if (t as usize) < s.set_count() => Ok(t as usize),
        Some(t) => Err(bad(cmd, format!("no texture set {t}"))),
        None => Ok(s.ts()),
    }
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = u64p(p, "ts")
        .map(|index| index as usize)
        .filter(|index| *index < s.set_count())
        .ok_or_else(|| bad("textureSet.select", "valid `ts` index required"))?;
    select_set(s, ts)?;
    Ok(json!({ "ts": ts }))
}

/// Make the next (or previous) texture set the active one; the last wraps to the first.
fn step(s: &mut Session, forward: bool) -> Result<Value> {
    let count = s.set_count();
    if count == 0 {
        return Err(bad("textureSet.next", "no texture sets"));
    }
    let ts = if forward { (s.ts() + 1) % count } else { (s.ts() + count - 1) % count };
    select_set(s, ts)?;
    Ok(json!({ "ts": ts }))
}

/// Make texture set `ts` the active one, on its content stack
pub(crate) fn select_set(s: &mut Session, ts: usize) -> Result<()> {
    s.cancel_stroke();

    // Paths paint into the active set
    if ts != s.ts() {
        s.finish_path()?;

        // The clone source is a point of the set it was picked in
        s.tool.clone_source = None;
    }

    s.active_set = ts;
    s.target = StackTarget::Content;
    s.revision = s.revision.wrapping_add(1);
    Ok(())
}

fn resolution(s: &mut Session, p: &Value) -> Result<Value> {
    if boolp(p, "all") == Some(true) {
        let r = u64p(p, "resolution").ok_or_else(|| bad("textureSet.resolution", "`resolution` is required"))?;
        let r = u32::try_from(r).unwrap_or(0);
        check_resolution(r)?;
        for ts in 0..s.set_count() {
            if s.doc.project.texture_set(ts)?.resolution != r {
                let task = s.resize_task(ts, r)?;
                let out = task.run(&texturecraft_bake::Progress::new())?;
                s.apply_resize(out)?;
            }
        }
        return Ok(json!({ "resolution": r }));
    }
    let ts = ts_param(s, p, "textureSet.resolution")?;
    let r = u64p(p, "resolution").ok_or_else(|| bad("textureSet.resolution", "`resolution` is required"))?;
    let task = s.resize_task(ts, u32::try_from(r).unwrap_or(0))?;
    let out = task.run(&texturecraft_bake::Progress::new())?;
    s.apply_resize(out)
}

/// A texture set size change prepared from a snapshot of the document, so the UI can run the
/// resampling and the new texel caches (seconds at 8K) on a worker thread:
/// [`Session::resize_task`], [`ResizeTask::run`], then [`Session::apply_resize`] (one undo step).
pub struct ResizeTask {
    ts: usize,
    resolution: u32,
    doc: Document,
    mesh: Arc<Mesh>,
    /// The session's revision and mesh when the task was made: the result applies only to them.
    from: (u64, u64),
}

/// What [`ResizeTask::run`] computed.
pub struct ResizeOutput {
    ts: usize,
    resolution: u32,
    doc: Document,
    /// The set's new caches (None when its UV tile layout moved: the session rebuilds them).
    cache: Option<SetCache>,
    log: Vec<String>,
    from: (u64, u64),
}

impl ResizeTask {
    /// Resample the set's painted pixels (bilinear; strokes are not replayed) and build its
    /// caches. Progress goes from 0 to 1.
    pub fn run(self, progress: &texturecraft_bake::Progress) -> Result<ResizeOutput> {
        let ResizeTask { ts, resolution, mut doc, mesh, from } = self;
        let set = doc.project.texture_set_mut(ts)?;
        let old = set.clone();
        set.resolution = resolution;
        crate::session::check_size(set)?;
        let size = set.size();
        let mut log = Vec::new();
        // Tiles of their own resolution keep it, so the others move within their atlas blocks.
        if old.tile_resolutions.is_empty() {
            crate::layer_ops::resample_set(&mut doc, ts, size);
        } else {
            crate::uv_tile_cmds::relayout(&mut doc, ts, &old, &mut log)?;
        }
        progress.done.store(3_000, std::sync::atomic::Ordering::Relaxed);
        let set = doc.project.texture_set(ts)?;
        let cache = (crate::session::uv_layout(&old) == crate::session::uv_layout(set)).then(|| SetCache::new(&mesh, set));
        progress.done.store(10_000, std::sync::atomic::Ordering::Relaxed);
        Ok(ResizeOutput { ts, resolution, doc, cache, log, from })
    }
}

impl ResizeOutput {
    /// The texture set and resolution this resize is for.
    pub fn target(&self) -> (usize, u32) {
        (self.ts, self.resolution)
    }
}

impl Session {
    /// Prepare resizing texture set `ts` to `resolution` (a power of two, 128..8192).
    pub fn resize_task(&mut self, ts: usize, resolution: u32) -> Result<ResizeTask> {
        let resolution = check_resolution(resolution)?;
        self.doc.project.texture_set(ts)?;
        self.cancel_stroke();
        Ok(ResizeTask { ts, resolution, doc: self.doc.clone(), mesh: self.mesh.clone(), from: (self.revision, self.mesh_id) })
    }

    /// Apply a finished resize as one undo step. When the document changed while it ran, the
    /// resize runs again here on the current document.
    pub fn apply_resize(&mut self, out: ResizeOutput) -> Result<Value> {
        if out.from != (self.revision, self.mesh_id) || self.stroke_active() {
            let task = self.resize_task(out.ts, out.resolution)?;
            return self.apply_resize(task.run(&texturecraft_bake::Progress::new())?);
        }
        let ResizeOutput { ts, resolution, doc, cache, log, .. } = out;
        let before = std::mem::replace(&mut self.doc, doc);
        // The new caches go in first, so the history step keeps them instead of rebuilding.
        if let (Some(c), Some(slot)) = (cache, self.caches.get_mut(ts)) {
            let old = std::mem::replace(slot, c);
            // Freeing an 8K set's caches takes tens of milliseconds: a thread does it.
            #[cfg(not(target_arch = "wasm32"))]
            let _ = std::thread::Builder::new().name("texturecraft-free".into()).spawn(move || drop(old));
            #[cfg(target_arch = "wasm32")]
            drop(old);
        }
        self.log.extend(log);
        self.push_history("Texture set size", before);
        Ok(json!({ "ts": ts, "resolution": resolution }))
    }
}

fn channel(s: &mut Session, p: &Value, add: bool) -> Result<Value> {
    let ts = ts_param(s, p, "textureSet.channel")?;
    let c = channel_param("textureSet.channel", p, "channel")?.ok_or_else(|| bad("textureSet.channel", "`channel` is required"))?;
    s.edit(if add { "Add channel" } else { "Remove channel" }, |s| {
        if add {
            return add_channel(s, ts, c);
        }
        if c == Channel::BaseColor {
            return Err(bad("textureSet.channel.remove", "base color can't be removed"));
        }
        s.doc.project.texture_set_mut(ts)?.channels.retain(|(x, _)| *x != c);
        Ok(())
    })?;
    Ok(Value::Null)
}

fn channel_format(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "textureSet.channel.format";
    let ts = ts_param(s, p, cmd)?;
    let c = channel_param(cmd, p, "channel")?.ok_or_else(|| bad(cmd, "`channel` is required"))?;
    let name = strp(p, "format").ok_or_else(|| bad(cmd, "`format` is required"))?;
    let format = ChannelFormat::parse(name).ok_or_else(|| bad(cmd, format!("unknown format `{name}`")))?;
    if !ChannelFormat::options_for(c).contains(&format) {
        return Err(bad(cmd, format!("{} can't be stored as {}", c.label(), format.label())));
    }
    let old = s
        .doc
        .project
        .texture_set(ts)?
        .channels
        .iter()
        .find(|(x, _)| *x == c)
        .map(|(_, f)| *f)
        .ok_or_else(|| bad(cmd, format!("the set has no {} channel", c.label())))?;
    if old == format {
        return Ok(json!({ "format": format.label() }));
    }
    s.edit("Change channel format", |s| {
        if format.bits() < old.bits() {
            crate::layer_ops::quantize_set_channel(&mut s.doc, ts, c, format.bits());
        }
        if let Some(slot) = s.doc.project.texture_set_mut(ts)?.channels.iter_mut().find(|(x, _)| *x == c) {
            slot.1 = format;
        }
        Ok(())
    })?;
    Ok(json!({ "format": format.label() }))
}

/// Add channel `c` to texture set `ts` (no-op when present) and enable it on the set's layers.
/// Call inside [`Session::edit`].
pub(crate) fn add_channel(s: &mut Session, ts: usize, c: Channel) -> Result<()> {
    let set = s.doc.project.texture_set_mut(ts)?;
    if !set.has_channel(c) {
        set.channels.push((c, ChannelFormat::default_for(c)));
        set.channels.sort_by_key(|(c, _)| c.index());
    }
    // Existing layers get the new channel enabled.
    let ids: Vec<_> = s.doc.project.walk(ts).into_iter().map(|(id, _)| id).collect();
    for id in ids {
        if let Ok(l) = s.doc.project.layer_mut(id) {
            let mode = if l.is_group() { texturecraft_core::BlendMode::Passthrough } else { texturecraft_core::BlendMode::Normal };
            l.channels.entry(c).or_insert(ChannelBlend { mode, opacity: 1.0 });
        }
    }
    Ok(())
}

fn visible(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = ts_param(s, p, "textureSet.visible")?;
    let set = s.doc.project.texture_set_mut(ts)?;
    set.visible = boolp(p, "visible").unwrap_or(!set.visible);
    let v = set.visible;
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "visible": v }))
}

fn visibility(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "textureSet.visibility";
    let ts = ts_param(s, p, cmd)?;
    let mode = strp(p, "mode").filter(|m| ["showAll", "hideAll", "invert", "solo"].contains(m));
    let mode = mode.ok_or_else(|| bad(cmd, "`mode` must be showAll, hideAll, invert or solo"))?;
    let sets = &mut s.doc.project.texture_sets;
    let solo = sets.iter().enumerate().all(|(i, t)| t.visible == (i == ts));
    for (i, t) in sets.iter_mut().enumerate() {
        t.visible = match mode {
            "showAll" => true,
            "hideAll" => false,
            "invert" => !t.visible,
            _ => solo || i == ts,
        };
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "visible": s.doc.project.texture_sets.iter().map(|t| t.visible).collect::<Vec<_>>() }))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = ts_param(s, p, "textureSet.rename")?;
    let name = strp(p, "name")
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.chars().count() <= 128)
        .ok_or_else(|| bad("textureSet.rename", "`name` is required"))?
        .to_string();
    s.edit("Rename texture set", |s| {
        s.doc.project.texture_set_mut(ts)?.name = name;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = ts_param(s, p, "textureSet.settings")?;
    let nf = normal_format_param("textureSet.settings", p)?;
    s.edit("Texture set settings", |s| {
        if let Some(n) = nf {
            s.doc.project.normal_format = n;
        }
        let set = s.doc.project.texture_set_mut(ts)?;
        if let Some(d) = strp(p, "description") {
            set.description = d.chars().take(4096).collect();
        }
        if let Some(h) = f32p(p, "heightStrength") {
            set.height_strength = h.clamp(0.0, 100.0);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_shows_hides_inverts_and_solos() {
        // The material ball has two texture sets.
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let mut run = |p: Value| crate::execute(&mut s, "textureSet.visibility", &p).map(|r| r["visible"].clone());
        assert_eq!(run(json!({"mode": "hideAll"})).unwrap(), json!([false, false]));
        assert_eq!(run(json!({"mode": "solo", "ts": 1})).unwrap(), json!([false, true]));
        assert_eq!(run(json!({"mode": "solo", "ts": 1})).unwrap(), json!([true, true]), "solo again shows every set");
        assert_eq!(run(json!({"mode": "solo", "ts": 0})).unwrap(), json!([true, false]));
        assert_eq!(run(json!({"mode": "invert"})).unwrap(), json!([false, true]));
        assert_eq!(run(json!({"mode": "showAll"})).unwrap(), json!([true, true]));
        assert!(run(json!({"mode": "sideways"})).is_err() && run(json!({"mode": "solo", "ts": 9})).is_err());
    }

    fn ball() -> Session {
        Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap()
    }

    #[test]
    fn next_and_previous_step_through_the_sets_and_wrap() {
        let mut s = ball();
        assert_eq!(s.ts(), 0);
        assert_eq!(crate::execute(&mut s, "textureSet.next", &Value::Null).unwrap()["ts"], 1);
        assert_eq!(crate::execute(&mut s, "textureSet.next", &Value::Null).unwrap()["ts"], 0, "the last set wraps to the first");
        assert_eq!(crate::execute(&mut s, "textureSet.previous", &Value::Null).unwrap()["ts"], 1, "the first set wraps to the last");
        assert_eq!(s.ts(), 1);
        assert_eq!(crate::execute(&mut s, "textureSet.previous", &Value::Null).unwrap()["ts"], 0);
    }

    #[test]
    fn resolution_all_sizes_every_set() {
        let mut s = ball();
        crate::execute(&mut s, "textureSet.resolution", &json!({"ts": 1, "resolution": 256})).unwrap();
        assert_ne!(s.doc.project.texture_sets[0].resolution, s.doc.project.texture_sets[1].resolution);
        crate::execute(&mut s, "textureSet.resolution", &json!({"resolution": 512, "all": true})).unwrap();
        assert!(s.doc.project.texture_sets.iter().all(|t| t.resolution == 512));
        assert!(crate::execute(&mut s, "textureSet.resolution", &json!({"resolution": 300, "all": true})).is_err());
        assert!(s.doc.project.texture_sets.iter().all(|t| t.resolution == 512), "a bad size changes nothing");
    }
}
