//! Texture set commands (Texture Set List / Texture Set Settings).

use serde_json::{Value, json};
use texturecraft_core::{AoMixing, Channel, ChannelBlend, ChannelFormat, HeightToNormalMethod, NormalMixing, UvPaddingMode};

use std::sync::Arc;

use texturecraft_geom::Mesh;

use crate::commands::{CommandSpec, always, bad, boolp, channel_param, f32p, normal_format_param, strp, u64p};
use crate::session::{Document, SetCache, StackTarget, check_resolution};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("textureSet.select", "Select texture set", [], None, "{ts}", always, select),
        cmd!("textureSet.list", "List texture sets", [], None, "{}", always, |s, _| Ok(crate::inspect::texture_sets(s))),
        cmd!(
            "textureSet.inspect",
            "Inspect texture set",
            [],
            None,
            "{ts?} → {index, name, originalName, description, resolution, height, size: [w, h], blockResolution, uvTiles: [UDIM numbers] (empty: no UV tiles), tiles: [{udim, block: [x, y], resolution, bakeEnabled}], channels: [{channel, format, shaded}], layers, parts, baked, visible, normalMixing, aoMixing, heightToNormal, uvPadding, active}",
            always,
            |s, p| Ok(crate::inspect::texture_set(s, ts_param(s, p, "textureSet.inspect")?))
        ),
        cmd!(
            "textureSet.resolution",
            "Texture set size",
            [],
            None,
            "{ts?, resolution: 128..8192 (power of two, texels across), height?: 128..8192 (texels down; default = resolution, a square set)}",
            always,
            resolution
        ),
        cmd!("textureSet.channel.add", "Add channel", [], None, "{ts?, channel}", always, |s, p| channel(s, p, true)),
        cmd!("textureSet.channel.remove", "Remove channel", [], None, "{ts?, channel}", always, |s, p| channel(s, p, false)),
        cmd!("textureSet.channels", "Add or remove channels", [], None, "{ts?, add?: [channel], remove?: [channel], allSets?}", always, channels),
        cmd!("textureSet.channel.format", "Set channel format", [], None, "{ts?, channel, format}", always, channel_format),
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
        cmd!(
            "textureSet.rename",
            "Rename texture set",
            [],
            None,
            "{ts?, name} or {ts?, reset: true} (back to the mesh material's name, `originalName` in inspect)",
            always,
            rename
        ),
        cmd!(
            "textureSet.settings",
            "Texture set settings",
            [],
            None,
            "{ts?, description?, heightStrength?, normalMixing?: combine|replace, aoMixing?: multiply|none, heightToNormal?: sharp|smooth, uvPadding?: 3d|2d, normalFormat?: openGl|directX}",
            always,
            settings
        ),
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
    let ts = ts_param(s, p, "textureSet.resolution")?;
    let width = u64p(p, "resolution").ok_or_else(|| bad("textureSet.resolution", "`resolution` is required"))?;
    let height = match p.get("height") {
        None | Some(Value::Null) => width,
        Some(_) => u64p(p, "height").ok_or_else(|| bad("textureSet.resolution", "`height` must be a number of texels"))?,
    };
    let task = s.resize_task_to(ts, u32::try_from(width).unwrap_or(0), u32::try_from(height).unwrap_or(0))?;
    let out = task.run(&texturecraft_bake::Progress::new())?;
    s.apply_resize(out)
}

/// A texture set size change prepared from a snapshot of the document, so the UI can run the
/// resampling and the new texel caches (seconds at 8K) on a worker thread:
/// [`Session::resize_task`], [`ResizeTask::run`], then [`Session::apply_resize`] (one undo step).
pub struct ResizeTask {
    ts: usize,
    resolution: u32,
    height: u32,
    doc: Document,
    mesh: Arc<Mesh>,
    /// The session's revision and mesh when the task was made: the result applies only to them.
    from: (u64, u64),
}

/// What [`ResizeTask::run`] computed.
pub struct ResizeOutput {
    ts: usize,
    resolution: u32,
    height: u32,
    doc: Document,
    /// The set's new caches (None when its UV tile layout moved: the session rebuilds them).
    cache: Option<SetCache>,
    log: crate::log::SessionLog,
    from: (u64, u64),
}

impl ResizeTask {
    /// Resample the set's painted pixels (bilinear; strokes are not replayed) and build its
    /// caches. Progress goes from 0 to 1.
    pub fn run(self, progress: &texturecraft_bake::Progress) -> Result<ResizeOutput> {
        let ResizeTask { ts, resolution, height, mut doc, mesh, from } = self;
        let set = doc.project.texture_set_mut(ts)?;
        let old = set.clone();
        set.set_size(resolution, height);
        if set.resolution_height.is_some() && !set.tile_resolutions.is_empty() {
            return Err(bad("textureSet.resolution", "a texture set whose UV tiles have their own sizes stays square"));
        }

        crate::session::check_size(set)?;
        let size = set.size();
        let mut log = crate::log::SessionLog::new();

        // Tiles of their own resolution keep it, so the others move within their atlas blocks
        if old.tile_resolutions.is_empty() {
            crate::layer_ops::resample_set(&mut doc, ts, size);
        } else {
            crate::uv_tile_cmds::relayout(&mut doc, ts, &old, &mut log)?;
        }
        progress.done.store(3_000, std::sync::atomic::Ordering::Relaxed);
        let set = doc.project.texture_set(ts)?;
        let cache = (crate::session::uv_layout(&old) == crate::session::uv_layout(set)).then(|| SetCache::new(&mesh, set));
        progress.done.store(10_000, std::sync::atomic::Ordering::Relaxed);
        Ok(ResizeOutput { ts, resolution, height, doc, cache, log, from })
    }
}

impl ResizeOutput {
    /// The texture set and resolution this resize is for.
    pub fn target(&self) -> (usize, u32) {
        (self.ts, self.resolution)
    }
}

impl Session {
    /// Prepare resizing texture set `ts` to a square `resolution` (a power of two, 128..8192)
    pub fn resize_task(&mut self, ts: usize, resolution: u32) -> Result<ResizeTask> {
        self.resize_task_to(ts, resolution, resolution)
    }

    /// Prepare resizing texture set `ts` to `width × height` texels (powers of two, 128..8192)
    pub fn resize_task_to(&mut self, ts: usize, width: u32, height: u32) -> Result<ResizeTask> {
        let (resolution, height) = (check_resolution(width)?, check_resolution(height)?);
        self.doc.project.texture_set(ts)?;
        self.cancel_stroke();
        Ok(ResizeTask { ts, resolution, height, doc: self.doc.clone(), mesh: self.mesh.clone(), from: (self.revision, self.mesh_id) })
    }

    /// Apply a finished resize as one undo step. When the document changed while it ran, the
    /// resize runs again here on the current document.
    pub fn apply_resize(&mut self, out: ResizeOutput) -> Result<Value> {
        if out.from != (self.revision, self.mesh_id) || self.stroke_active() {
            let task = self.resize_task_to(out.ts, out.resolution, out.height)?;
            return self.apply_resize(task.run(&texturecraft_bake::Progress::new())?);
        }

        let ResizeOutput { ts, resolution, height, doc, cache, log, .. } = out;
        let before = std::mem::replace(&mut self.doc, doc);

        // Install the new caches before adding history to avoid rebuilding them
        if let (Some(c), Some(slot)) = (cache, self.caches.get_mut(ts)) {
            let old = std::mem::replace(slot, c);

            // Release large caches without blocking the UI
            #[cfg(not(target_arch = "wasm32"))]
            let _ = std::thread::Builder::new().name("texturecraft-free".into()).spawn(move || drop(old));
            #[cfg(target_arch = "wasm32")]
            drop(old);
        }
        self.log.append(log);
        self.push_history("Texture set size", before);
        Ok(json!({ "ts": ts, "resolution": resolution, "height": height }))
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

/// Add and remove several channels in one undo step, on one texture set or on all of them
fn channels(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "textureSet.channels";
    let list = |key: &str| -> Result<Vec<Channel>> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    let name = item.as_str().ok_or_else(|| bad(cmd, format!("`{key}` lists channel names")))?;
                    Channel::parse(name).ok_or_else(|| bad(cmd, format!("unknown channel `{name}`")))
                })
                .collect(),
            Some(_) => Err(bad(cmd, format!("`{key}` must be a list of channels"))),
        }
    };

    let add = list("add")?;
    let remove = list("remove")?;

    if remove.contains(&Channel::BaseColor) {
        return Err(bad(cmd, "base color can't be removed"));
    }

    if let Some(both) = add.iter().find(|c| remove.contains(c)) {
        return Err(bad(cmd, format!("{} is both added and removed", both.label())));
    }

    let sets: Vec<usize> = if boolp(p, "allSets").unwrap_or(false) { (0..s.set_count()).collect() } else { vec![ts_param(s, p, cmd)?] };

    s.edit("Add or remove channels", |s| {
        for &ts in &sets {
            // A channel the set already has keeps its layers' channel switches
            for &c in &add {
                if !s.doc.project.texture_set(ts)?.has_channel(c) {
                    add_channel(s, ts, c)?;
                }
            }
            s.doc.project.texture_set_mut(ts)?.channels.retain(|(c, _)| !remove.contains(c));
        }
        Ok(())
    })?;

    Ok(json!({ "ts": sets }))
}

/// Store a channel in another format; layer data keeps its precision, so a wider format brings back what was painted
fn channel_format(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "textureSet.channel.format";
    let ts = ts_param(s, p, cmd)?;
    let c = channel_param(cmd, p, "channel")?.ok_or_else(|| bad(cmd, "`channel` is required"))?;
    let name = strp(p, "format").ok_or_else(|| bad(cmd, "`format` is required"))?;
    let all = || ChannelFormat::ALL.map(ChannelFormat::label).join(", ");
    let format = ChannelFormat::parse(name).ok_or_else(|| bad(cmd, format!("unknown format {name:?} ({})", all())))?;

    s.edit("Channel format", |s| {
        if s.doc.project.texture_set_mut(ts)?.set_channel_format(c, format) {
            Ok(())
        } else {
            Err(bad(cmd, format!("the texture set has no {} channel", c.label())))
        }
    })?;

    let float = matches!(format, ChannelFormat::L16F | ChannelFormat::Rgb16F | ChannelFormat::L32F | ChannelFormat::Rgb32F);
    Ok(if float { json!({"note": "float formats are stored at 16 bits for now, and values stay in the channel's range"}) } else { Value::Null })
}

/// Add channel `c` to texture set `ts` (no-op when present) and enable it on the set's layers.
/// Call inside [`Session::edit`].
pub(crate) fn add_channel(s: &mut Session, ts: usize, c: Channel) -> Result<()> {
    s.doc.project.texture_set_mut(ts)?.add_channel(c);

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

/// Longest texture set name and description, in characters
const MAX_NAME_CHARS: usize = 128;
const MAX_DESCRIPTION_CHARS: usize = 4096;
/// Highest height strength a set takes
const MAX_HEIGHT_STRENGTH: f32 = 100.0;

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = ts_param(s, p, "textureSet.rename")?;
    let name = if boolp(p, "reset") == Some(true) {
        s.original_set_name(ts).ok_or_else(|| bad("textureSet.rename", "the mesh has no material name for this set"))?.to_string()
    } else {
        let name = strp(p, "name").map(str::trim).filter(|name| !name.is_empty());
        let name = name.ok_or_else(|| bad("textureSet.rename", "`name` or `reset: true` is required"))?;
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(bad("textureSet.rename", format!("`name` is at most {MAX_NAME_CHARS} characters")));
        }

        name.to_string()
    };
    s.edit("Rename texture set", |s| {
        s.doc.project.texture_set_mut(ts)?.name = name;
        Ok(())
    })?;
    Ok(Value::Null)
}

impl Session {
    /// The name set `ts` had when the mesh loaded: its material's name in the mesh
    pub fn original_set_name(&self, ts: usize) -> Option<&str> {
        let set = self.doc.project.texture_sets.get(ts)?;
        self.mesh.materials.get(set.material).map(String::as_str).filter(|name| !name.trim().is_empty())
    }
}

fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    let ts = ts_param(s, p, "textureSet.settings")?;
    let normal_format = normal_format_param("textureSet.settings", p)?;
    let normal_mixing = choice_param(p, "normalMixing", "normal mixing", NormalMixing::parse, &NormalMixing::ALL.map(NormalMixing::id))?;
    let ao_mixing = choice_param(p, "aoMixing", "ambient occlusion mixing", AoMixing::parse, &AoMixing::ALL.map(AoMixing::id))?;
    let height_to_normal =
        choice_param(p, "heightToNormal", "height to normal method", HeightToNormalMethod::parse, &HeightToNormalMethod::ALL.map(HeightToNormalMethod::id))?;
    let uv_padding = choice_param(p, "uvPadding", "UV padding", UvPaddingMode::parse, &UvPaddingMode::ALL.map(UvPaddingMode::id))?;
    s.edit("Texture set settings", |s| {
        if let Some(format) = normal_format {
            s.doc.project.normal_format = format;
        }
        let set = s.doc.project.texture_set_mut(ts)?;
        if let Some(description) = strp(p, "description") {
            set.description = description.chars().take(MAX_DESCRIPTION_CHARS).collect();
        }
        if let Some(strength) = f32p(p, "heightStrength") {
            set.height_strength = strength.clamp(0.0, MAX_HEIGHT_STRENGTH);
        }
        if let Some(mixing) = normal_mixing {
            set.normal_mixing = mixing;
        }
        if let Some(mixing) = ao_mixing {
            set.ao_mixing = mixing;
        }
        if let Some(method) = height_to_normal {
            set.height_to_normal = method;
        }
        if let Some(mode) = uv_padding {
            set.uv_padding = mode;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// The optional setting `key` of `textureSet.settings`, one of `ids`
fn choice_param<T>(p: &Value, key: &str, what: &str, parse: fn(&str) -> Option<T>, ids: &[&str]) -> Result<Option<T>> {
    let unknown = |value: &Value| bad("textureSet.settings", format!("unknown {what} {value} ({})", ids.join(" or ")));
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value @ Value::String(id)) => parse(id).map(Some).ok_or_else(|| unknown(value)),
        Some(other) => Err(unknown(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_add_and_remove_together_in_one_undo_step_on_every_set() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let has = |s: &Session, ts: usize, c: Channel| s.doc.project.texture_sets[ts].has_channel(c);
        assert!(has(&s, 1, Channel::Height) && !has(&s, 1, Channel::User3));
        let edit = json!({"add": ["emissive", "User3"], "remove": ["height"], "allSets": true});
        crate::execute(&mut s, "textureSet.channels", &edit).unwrap();
        for ts in 0..2 {
            assert!(has(&s, ts, Channel::Emissive) && has(&s, ts, Channel::User3) && !has(&s, ts, Channel::Height), "set {ts}");
        }
        crate::execute(&mut s, "edit.undo", &json!({})).unwrap();
        for ts in 0..2 {
            assert!(!has(&s, ts, Channel::Emissive) && !has(&s, ts, Channel::User3) && has(&s, ts, Channel::Height), "set {ts}");
        }

        // Without allSets only the chosen set changes
        crate::execute(&mut s, "textureSet.channels", &json!({"ts": 1, "add": ["ior"]})).unwrap();
        assert!(has(&s, 1, Channel::Ior) && !has(&s, 0, Channel::Ior));
        assert!(crate::execute(&mut s, "textureSet.channels", &json!({"remove": ["baseColor"]})).is_err());
        assert!(crate::execute(&mut s, "textureSet.channels", &json!({"add": ["ior"], "remove": ["ior"]})).is_err());
        assert!(crate::execute(&mut s, "textureSet.channels", &json!({"add": ["nope"]})).is_err());
    }

    #[test]
    fn channels_applied_to_every_set_keep_the_channels_a_layer_switched_off() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        crate::execute(&mut s, "layer.add.fill", &json!({})).unwrap();
        let layer = s.selected_layer().unwrap();

        // Switched off, as layer.blend {enabled: false} leaves it
        s.doc.project.layer_mut(layer).unwrap().channels.remove(&Channel::Roughness);
        let layer_has_roughness = |s: &Session| s.doc.project.layer(layer).is_ok_and(|l| l.channels.contains_key(&Channel::Roughness));
        assert!(!layer_has_roughness(&s));

        // Apply to all Texture Sets sends every ticked channel, the ones the sets already have too
        let ticked: Vec<&str> = s.doc.project.texture_sets[0].channel_list().iter().map(|c| c.id()).chain(["ior"]).collect();
        crate::execute(&mut s, "textureSet.channels", &json!({"add": ticked, "allSets": true})).unwrap();

        assert!(s.doc.project.texture_sets.iter().all(|set| set.has_channel(Channel::Ior)));
        assert!(!layer_has_roughness(&s), "the layer's roughness stays off");
    }

    #[test]
    fn malformed_size_and_setting_values_are_refused_and_change_nothing() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        crate::execute(&mut s, "textureSet.resolution", &json!({"ts": 0, "resolution": 256, "height": 128})).unwrap();
        let before = s.doc.project.texture_sets[0].clone();

        assert!(crate::execute(&mut s, "textureSet.resolution", &json!({"ts": 0, "resolution": 512, "height": "256"})).is_err());
        for (key, value) in [("normalMixing", json!(1)), ("aoMixing", json!(true)), ("heightToNormal", json!(["sharp"])), ("uvPadding", json!(2))] {
            let mut params = json!({"ts": 0});
            params[key] = value.clone();
            assert!(crate::execute(&mut s, "textureSet.settings", &params).is_err(), "{key}: {value}");
        }

        assert_eq!(s.doc.project.texture_sets[0], before);
    }

    #[test]
    fn a_sets_size_formats_and_settings_survive_save_and_reopen() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        crate::execute(&mut s, "textureSet.resolution", &json!({"ts": 1, "resolution": 256, "height": 128})).unwrap();
        crate::execute(&mut s, "textureSet.channel.format", &json!({"ts": 1, "channel": "roughness", "format": "L8"})).unwrap();
        crate::execute(&mut s, "textureSet.channels", &json!({"ts": 1, "add": ["user7", "coatTangent"]})).unwrap();
        let settings =
            json!({"ts": 1, "normalMixing": "replace", "aoMixing": "none", "heightToNormal": "sharp", "uvPadding": "2d", "description": "Ünïcode 説明"});
        crate::execute(&mut s, "textureSet.settings", &settings).unwrap();
        let inspect = |s: &mut Session, ts: usize| crate::execute(s, "textureSet.inspect", &json!({"ts": ts})).unwrap();
        let (first, second) = (inspect(&mut s, 0), inspect(&mut s, 1));

        let mut back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();

        for (ts, before) in [(0, first), (1, second)] {
            let after = inspect(&mut back, ts);
            for key in ["resolution", "height", "size", "channels", "normalMixing", "aoMixing", "heightToNormal", "uvPadding", "description"] {
                assert_eq!(after[key], before[key], "set {ts} {key}");
            }
        }
        assert_ne!(inspect(&mut back, 0)["uvPadding"], inspect(&mut back, 1)["uvPadding"], "each set keeps its own settings");
    }

    #[test]
    fn rename_reset_restores_the_mesh_material_name() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let original = crate::execute(&mut s, "textureSet.inspect", &json!({"ts": 1})).unwrap()["name"].clone();
        crate::execute(&mut s, "textureSet.rename", &json!({"ts": 1, "name": "Renamed"})).unwrap();
        let inspect = crate::execute(&mut s, "textureSet.inspect", &json!({"ts": 1})).unwrap();
        assert_eq!((&inspect["name"], &inspect["originalName"]), (&json!("Renamed"), &original));
        crate::execute(&mut s, "textureSet.rename", &json!({"ts": 1, "reset": true})).unwrap();
        assert_eq!(crate::execute(&mut s, "textureSet.inspect", &json!({"ts": 1})).unwrap()["name"], original);
        assert!(crate::execute(&mut s, "textureSet.rename", &json!({"ts": 1})).is_err());
    }

    #[test]
    fn a_too_long_name_is_refused_for_its_length_not_as_missing() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let long = "n".repeat(MAX_NAME_CHARS + 1);
        let error = crate::execute(&mut s, "textureSet.rename", &json!({"ts": 1, "name": long})).unwrap_err().to_string();
        assert!(error.contains("at most") && !error.contains("required"), "{error}");
        assert!(crate::execute(&mut s, "textureSet.rename", &json!({"ts": 1, "name": "n".repeat(MAX_NAME_CHARS)})).is_ok());
    }

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

    #[test]
    fn normal_mixing_replace_keeps_height_out_of_the_normal_and_undoes() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let mixing = |s: &Session| crate::inspect::texture_set(s, 0)["normalMixing"].clone();
        assert_eq!(mixing(&s), json!("combine"));

        crate::execute(&mut s, "textureSet.settings", &json!({"ts": 0, "normalMixing": "replace"})).unwrap();
        assert_eq!(mixing(&s), json!("replace"));
        assert!(!s.doc.project.texture_sets[0].height_in_normal() && s.doc.project.texture_sets[1].height_in_normal());
        assert!(crate::execute(&mut s, "textureSet.settings", &json!({"normalMixing": "blend"})).is_err());

        crate::execute(&mut s, "edit.undo", &Value::Null).unwrap();
        assert_eq!(mixing(&s), json!("combine"));
    }

    #[test]
    fn height_to_normal_method_is_set_by_command_and_undoes() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let method = |s: &Session, ts: usize| crate::inspect::texture_set(s, ts)["heightToNormal"].clone();
        assert_eq!(method(&s, 0), json!("smooth"));

        crate::execute(&mut s, "textureSet.settings", &json!({"ts": 0, "heightToNormal": "sharp"})).unwrap();
        assert_eq!((method(&s, 0), method(&s, 1)), (json!("sharp"), json!("smooth")));
        assert!(crate::execute(&mut s, "textureSet.settings", &json!({"heightToNormal": "blurry"})).is_err());

        crate::execute(&mut s, "edit.undo", &Value::Null).unwrap();
        assert_eq!(method(&s, 0), json!("smooth"));
    }

    #[test]
    fn uv_padding_rebuilds_the_viewport_padding_and_undoes() {
        // The ball's second set has texels just past its seams at this size
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 512).unwrap();
        let mode = |s: &Session| crate::inspect::texture_set(s, 1)["uvPadding"].clone();
        let crosses_seams = |s: &Session| !s.caches[1].padding.crossed.is_empty();
        assert_eq!((mode(&s), crosses_seams(&s)), (json!("3d"), true));
        s.update();

        crate::execute(&mut s, "textureSet.settings", &json!({"ts": 1, "uvPadding": "2d"})).unwrap();
        assert_eq!((mode(&s), crosses_seams(&s)), (json!("2d"), false));
        assert!(!s.caches[1].dirty.is_empty(), "the set recomposites with its new padding");
        assert!(crate::execute(&mut s, "textureSet.settings", &json!({"uvPadding": "4d"})).is_err());

        crate::execute(&mut s, "edit.undo", &Value::Null).unwrap();
        assert_eq!((mode(&s), crosses_seams(&s)), (json!("3d"), true));
    }

    #[test]
    fn channel_format_changes_what_the_viewport_holds_and_undoes() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let format = |s: &Session| {
            let channels = crate::inspect::texture_set(s, 0)["channels"].clone();
            channels.as_array().and_then(|list| list.iter().find(|entry| entry["channel"] == "roughness")).map(|entry| entry["format"].clone())
        };
        let roughness = |s: &mut Session| {
            s.update();
            let img = &s.caches[0].composite.channels[&Channel::Roughness];
            let tile = s.caches[0].covered_tiles[0];
            img.tile(tile.0, tile.1).map(<[u16]>::to_vec).unwrap_or_default()
        };
        crate::execute(&mut s, "textureSet.channel.format", &json!({"channel": "roughness", "format": "L16"})).unwrap();

        // A roughness off the 8-bit grid, so 8 bits visibly round it
        crate::execute(&mut s, "layer.add.fill", &json!({"roughness": 0.3013})).unwrap();
        let sixteen = roughness(&mut s);
        assert_eq!(format(&s), Some(json!("L16")));

        crate::execute(&mut s, "textureSet.channel.format", &json!({"channel": "Roughness", "format": "l8"})).unwrap();
        let eight = roughness(&mut s);
        assert_eq!(format(&s), Some(json!("L8")));
        assert_ne!(eight, sixteen, "the composite holds 8 bits");
        let eight_bit_step = u16::MAX / u16::from(u8::MAX);
        assert!(eight.iter().all(|v| v % eight_bit_step == 0), "every texel is on an 8-bit step");

        let float = crate::execute(&mut s, "textureSet.channel.format", &json!({"channel": "height", "format": "L32F"})).unwrap();
        assert!(float["note"].is_string(), "a float format says it holds 16 bits");
        crate::execute(&mut s, "edit.undo", &Value::Null).unwrap();
        assert!(crate::execute(&mut s, "textureSet.channel.format", &json!({"channel": "roughness", "format": "L4"})).is_err());
        assert!(crate::execute(&mut s, "textureSet.channel.format", &json!({"channel": "emissive", "format": "L8"})).is_err());
        crate::execute(&mut s, "edit.undo", &Value::Null).unwrap();
        assert_eq!((format(&s), roughness(&mut s)), (Some(json!("L16")), sixteen));
    }

    #[test]
    fn channels_added_after_a_format_is_set_keep_16_bits() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let format = |s: &Session, id: &str| {
            let channels = crate::inspect::texture_set(s, 0)["channels"].clone();
            channels.as_array().and_then(|list| list.iter().find(|entry| entry["channel"] == id)).map(|entry| entry["format"].clone())
        };
        crate::execute(&mut s, "textureSet.channel.format", &json!({"ts": 0, "channel": "roughness", "format": "L8"})).unwrap();

        crate::execute(&mut s, "textureSet.channel.add", &json!({"ts": 0, "channel": "user3"})).unwrap();
        crate::execute(&mut s, "textureSet.channels", &json!({"ts": 0, "add": ["user4"]})).unwrap();

        assert_eq!((format(&s, "user3"), format(&s, "user4")), (Some(json!("L16")), Some(json!("L16"))));
        assert_eq!(format(&s, "roughness"), Some(json!("L8")), "the set format stays");
    }

    #[test]
    fn ao_mixing_resends_the_set_to_the_viewport_and_undoes() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        s.update();
        let _ = (s.take_uploads(0), s.take_uploads(1));

        crate::execute(&mut s, "textureSet.settings", &json!({"ts": 0, "aoMixing": "none"})).unwrap();
        assert_eq!(crate::inspect::texture_set(&s, 0)["aoMixing"], json!("none"));
        assert!(!s.take_uploads(0).is_empty() && s.take_uploads(1).is_empty(), "only the changed set is resent");
        assert!(crate::execute(&mut s, "textureSet.settings", &json!({"aoMixing": "screen"})).is_err());

        crate::execute(&mut s, "edit.undo", &Value::Null).unwrap();
        s.update();
        assert_eq!(crate::inspect::texture_set(&s, 0)["aoMixing"], json!("multiply"));
        assert!(!s.take_uploads(0).is_empty(), "undo resends the set too");
    }

    #[test]
    fn channels_our_shader_ignores_are_added_marked_and_exported() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        for channel in ["user3", "Displacement", "coat tangent"] {
            crate::execute(&mut s, "textureSet.channel.add", &json!({"ts": 0, "channel": channel})).unwrap();
        }
        let channels = crate::inspect::texture_set(&s, 0)["channels"].clone();
        let shaded = |id: &str| channels.as_array().and_then(|list| list.iter().find(|c| c["channel"] == id)).map(|c| c["shaded"].clone());
        assert_eq!((shaded("user3"), shaded("displacement"), shaded("coatTangent")), (Some(json!(false)), Some(json!(false)), Some(json!(false))));
        assert_eq!(shaded("baseColor"), Some(json!(true)));

        let plan = crate::execute(&mut s, "export.plan", &json!({"preset": "Separate maps"})).unwrap();
        let files: Vec<(Value, String)> =
            plan.as_array().into_iter().flatten().map(|f| (f["ts"].clone(), f["file"].as_str().unwrap_or_default().to_ascii_lowercase())).collect();
        for map in ["user3", "displacement", "coattangent"] {
            let from = |ts: usize| files.iter().any(|(t, file)| *t == json!(ts) && file.contains(map));
            assert!(from(0) && !from(1), "{map} exported only from the set that has it: {files:?}");
        }

        s.update();
        let composite = &s.caches[0].composite;
        assert!(composite.channels.contains_key(&Channel::User3) && composite.channels.contains_key(&Channel::CoatTangent));
    }
}
