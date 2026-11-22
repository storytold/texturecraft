//! Texture set commands (Texture Set List / Texture Set Settings).

use serde_json::{Value, json};
use texturecraft_core::{Channel, ChannelBlend, ChannelFormat};

use std::sync::Arc;

use texturecraft_compose::{BakedMaps, GraphStore, ImageStore, Inputs};
use texturecraft_geom::{Bvh, Mesh};

use crate::commands::{CommandSpec, always, bad, boolp, channel_param, f32p, normal_format_param, strp, u64p};
use crate::session::{Document, SetCache, StackTarget, check_resolution};
use crate::stroke_replay::ReplayScene;
use crate::{ResizeProgress, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("textureSet.select", "Select texture set", [], None, "{ts}", always, select),
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
        cmd!("textureSet.resolution", "Texture set size", [], None, "{ts?, resolution: 128..8192 (power of two)}", always, resolution),
        cmd!(
            "textureSet.channel.add",
            "Add channel",
            [],
            None,
            "{ts?, channel, name?: what the set calls it (user channels; on a present channel it renames it, \"\" clears)}",
            always,
            |s, p| channel(s, p, true)
        ),
        cmd!("textureSet.channel.remove", "Remove channel", [], None, "{ts?, channel}", always, |s, p| channel(s, p, false)),
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
    source: Arc<Mesh>,
    bvh: Arc<Bvh>,
    images: ImageStore,
    solo: Option<crate::Solo>,
    lookup_budget: usize,
    /// The project, its revision and its mesh when the task was made.
    from: (u64, u64, u64),
}

/// What [`ResizeTask::run`] computed.
pub struct ResizeOutput {
    ts: usize,
    resolution: u32,
    doc: Document,
    cache: SetCache,
    /// The replacement atlas mesh if the UV tile layout moved.
    mesh: Option<Arc<Mesh>>,
    graphs: Option<GraphStore>,
    log: crate::log::SessionLog,
    from: (u64, u64, u64),
}

type Resampled = Option<std::result::Result<texturecraft_texel::TileImage, String>>;

impl ResizeTask {
    /// Repaint recorded strokes at the new size, resample the other images, and build the set's
    /// caches. Progress goes from 0 to 1.
    pub fn run(self, progress: &texturecraft_bake::Progress) -> Result<ResizeOutput> {
        self.run_with_resampler(progress, |_, _, _| None)
    }

    /// An optional accelerator resamples ordinary paint images. A rejected image uses the
    /// same CPU resampler as headless editing. The resized tiles remain part of the document.
    pub fn run_with_resampler(
        self,
        progress: &texturecraft_bake::Progress,
        accelerate: impl FnMut(&texturecraft_texel::TileImage, (u32, u32), &[u16]) -> Resampled,
    ) -> Result<ResizeOutput> {
        self.run_inner(progress, accelerate, None::<(&ResizeProgress, fn(&Inputs) -> bool)>)
    }

    /// Prepare the new view and its graph results before replacing the document shown by the front end.
    pub fn run_for_view(
        self,
        progress: &ResizeProgress,
        accelerate: impl FnMut(&texturecraft_texel::TileImage, (u32, u32), &[u16]) -> Resampled,
        gpu_can_compose: impl FnOnce(&Inputs) -> bool,
    ) -> Result<ResizeOutput> {
        self.run_inner(&progress.paint, accelerate, Some((progress, gpu_can_compose)))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run_for_browser(self, progress: &ResizeProgress) -> Result<ResizeOutput> {
        let output = self.run(&progress.paint)?;
        progress.ready();
        Ok(output)
    }

    fn run_inner(
        self,
        progress: &texturecraft_bake::Progress,
        mut accelerate: impl FnMut(&texturecraft_texel::TileImage, (u32, u32), &[u16]) -> Resampled,
        view: Option<(&ResizeProgress, impl FnOnce(&Inputs) -> bool)>,
    ) -> Result<ResizeOutput> {
        let start = chrono::Utc::now();
        let ResizeTask { ts, resolution, mut doc, mesh, source, bvh, images, solo, lookup_budget, from } = self;
        let set = doc.project.texture_set_mut(ts)?;
        let old = set.clone();
        set.resolution = resolution;
        crate::session::check_size(set)?;
        let size = set.size();
        let scene = ReplayScene { mesh: &mesh, bvh: &bvh, material: set.material };
        let mut log = crate::log::SessionLog::new();
        let mut gpu_ms = 0;
        let mut gpu_images = 0;
        let mut fallback = None;
        let cancelled = || progress.cancel.load(std::sync::atomic::Ordering::Relaxed);
        if cancelled() {
            return Err(crate::EngineError::Other("Texture set resize was cancelled".into()));
        }
        let replay = if old.resolution != resolution && old.uv_tiles.is_empty() && old.tile_resolutions.is_empty() {
            doc.stroke_replays.prepare_resize(&doc.project, &doc.pixels, ts, size, &scene, progress)
        } else {
            Default::default()
        };
        if cancelled() {
            return Err(crate::EngineError::Other("Texture set resize was cancelled".into()));
        }
        replay.remove_rasters(&mut doc.pixels);

        // Tiles of their own resolution keep it, so the others move within their atlas blocks
        if old.tile_resolutions.is_empty() {
            crate::layer_ops::map_set_images(&mut doc, ts, |image, empty| {
                if cancelled() {
                    return image.clone();
                }
                if old.uv_tiles.is_empty() && image.allocated_count() > 0 && (image.width(), image.height()) != size {
                    let submitted = chrono::Utc::now();
                    if let Some(result) = accelerate(image, size, empty) {
                        gpu_ms += (chrono::Utc::now() - submitted).num_milliseconds();
                        match result {
                            Ok(resized) => {
                                gpu_images += 1;
                                return resized;
                            }
                            Err(reason) => {
                                fallback.get_or_insert(reason);
                            }
                        }
                    }
                }
                if cancelled() { image.clone() } else { crate::layer_ops::fit(image, size, empty) }
            });
        } else {
            crate::uv_tile_cmds::relayout(&mut doc, ts, &old, &mut log)?;
        }
        if cancelled() {
            return Err(crate::EngineError::Other("Texture set resize was cancelled".into()));
        }
        progress.done.store(3_000, std::sync::atomic::Ordering::Relaxed);
        let resampled = chrono::Utc::now();
        let set = doc.project.texture_set(ts)?;
        let new_mesh = (crate::session::uv_layout(&old) != crate::session::uv_layout(set)).then(|| crate::session::tiled_mesh(&source, &doc.project));
        let display_mesh = new_mesh.as_ref().unwrap_or(&mesh);
        let mut cache = SetCache::new(display_mesh, set, if replay.is_empty() { lookup_budget } else { usize::MAX });
        let cache_built = chrono::Utc::now();
        if !replay.is_empty() {
            let deferred = replay.deferred_count();
            let replayed = replay.run(&mut doc.pixels, &mut doc.stroke_replays, &cache.gbuf, &scene, progress)?;
            if deferred > 0 {
                log.warn("Paint", format!("{deferred} paint targets exceeded the resize replay budget. Their raster pixels were resampled and their paint operations were retained."));
            }
            if replayed > 0 {
                log.performance(format!("Replayed {replayed} recorded paint strokes in {} ms", (chrono::Utc::now() - cache_built).num_milliseconds()));
            }
        }
        if cancelled() {
            return Err(crate::EngineError::Other("Texture set resize was cancelled".into()));
        }
        progress.done.store(10_000, std::sync::atomic::Ordering::Relaxed);
        if let Some(reason) = fallback {
            log.performance(format!("GPU paint resize used CPU fallback: {reason}"));
        }
        log.performance(format!(
            "Resize of texture set {} from {} to {}: paint resampling {} ms (GPU {} images in {} ms including transfers), CPU UV cache {} ms",
            ts + 1,
            old.resolution,
            resolution,
            (resampled - start).num_milliseconds(),
            gpu_images,
            gpu_ms,
            (cache_built - resampled).num_milliseconds(),
        ));

        let mut graphs = None;
        if let Some((progress, gpu_can_compose)) = view {
            let started = chrono::Utc::now();
            let view_project = solo.and_then(|solo| texturecraft_compose::solo_project(&doc.project, solo.layer, solo.ignore_mask));
            let project = view_project.as_ref().unwrap_or(&doc.project);
            let set = project.texture_set(ts)?;
            let count = project.render_tree(&set.stack).into_iter().filter_map(|id| project.layers.get(&id)).map(|layer| layer.graphs().len()).sum();
            let prepared = doc.graphs.for_resize(progress.graphs_cancel.clone(), ts);
            let empty = BakedMaps::default();
            let gbuf = cache.gbuf().clone();
            let inputs = Inputs {
                project,
                ts,
                mesh: display_mesh,
                gbuf: &gbuf,
                pixels: &doc.pixels,
                baked: doc.baked.get(&ts).unwrap_or(&empty),
                block_scales: cache.composite.block_scales.clone(),
                preview: None,
                images: &images,
                graphs: &prepared,
                bounds: display_mesh.bounds(),
            };
            if count > 0 {
                progress.graphs(&prepared, count);
                texturecraft_compose::eval::prepare_texture_set_graphs(&inputs);
            }
            progress.check()?;
            let prepared_display = !gpu_can_compose(&inputs);
            if prepared_display {
                crate::resize_view::prepare_cpu_view(progress, &inputs, &mut cache)?;
            }
            progress.check()?;
            prepared.park_old_resize_results(crate::session::COOK_BUDGET);
            progress.check()?;
            progress.ready();
            log.performance(format!(
                "Resize view for texture set {} at {resolution}: {} in {} ms, {} graph renders, {} cached graph results",
                ts + 1,
                if prepared_display { "CPU view prepared" } else { "GPU inputs prepared" },
                (chrono::Utc::now() - started).num_milliseconds(),
                prepared.renders(),
                prepared.resize_cache_hits(),
            ));
            graphs = Some(prepared);
        }
        Ok(ResizeOutput { ts, resolution, doc, cache, mesh: new_mesh, graphs, log, from })
    }
}

impl ResizeOutput {
    /// The texture set and resolution this resize is for.
    pub fn target(&self) -> (usize, u32) {
        (self.ts, self.resolution)
    }

    pub fn matches_session(&self, session: &Session) -> bool {
        self.from == (session.project_id, session.revision, session.mesh_id) && !session.stroke_active()
    }
}

impl Session {
    /// Prepare resizing texture set `ts` to `resolution` (a power of two, 128..8192).
    pub fn resize_task(&mut self, ts: usize, resolution: u32) -> Result<ResizeTask> {
        let resolution = check_resolution(resolution)?;
        self.doc.project.texture_set(ts)?;
        self.cancel_stroke();
        Ok(ResizeTask {
            ts,
            resolution,
            doc: self.doc.clone(),
            mesh: self.mesh.clone(),
            source: self.source.clone(),
            bvh: self.bvh.clone(),
            images: self.images.clone(),
            solo: self.solo,
            lookup_budget: self.lookup_budget_for(ts),
            from: (self.project_id, self.revision, self.mesh_id),
        })
    }

    /// Apply a finished resize as one undo step. When the document changed while it ran, the
    /// resize runs again here on the current document.
    pub fn apply_resize(&mut self, out: ResizeOutput) -> Result<Value> {
        if out.from.0 != self.project_id {
            return Err(crate::EngineError::Other("Texture set resize is from another project".into()));
        }
        if !out.matches_session(self) {
            let task = self.resize_task(out.ts, out.resolution)?;
            return self.apply_resize(task.run(&texturecraft_bake::Progress::new())?);
        }

        let ResizeOutput { ts, resolution, doc, mut cache, mesh, graphs, log, .. } = out;
        if let Some(graphs) = graphs {
            doc.graphs.accept_resize(&graphs, ts);
        }
        let before = std::mem::replace(&mut self.doc, doc);

        // Install the new caches before adding history to avoid rebuilding them
        if let Some(slot) = self.caches.get_mut(ts) {
            cache.revision = slot.revision.wrapping_add(1);
            cache.material_revision = slot.material_revision.wrapping_add(1);
            let old = std::mem::replace(slot, cache);

            // Release large caches without blocking the UI
            #[cfg(not(target_arch = "wasm32"))]
            let _ = std::thread::Builder::new().name("texturecraft-free".into()).spawn(move || drop(old));
            #[cfg(target_arch = "wasm32")]
            drop(old);
        }
        self.log.append(log);
        self.record_resize(before, ts, mesh);
        self.trim_lookups(self.ts());
        Ok(json!({ "ts": ts, "resolution": resolution }))
    }
}

/// Longest channel name kept, in characters.
const MAX_CHANNEL_NAME: usize = 256;

fn channel(s: &mut Session, p: &Value, add: bool) -> Result<Value> {
    let ts = ts_param(s, p, "textureSet.channel")?;
    let c = channel_param("textureSet.channel", p, "channel")?.ok_or_else(|| bad("textureSet.channel", "`channel` is required"))?;
    let name = strp(p, "name").map(|n| n.trim().chars().take(MAX_CHANNEL_NAME).collect::<String>());
    s.edit(if add { "Add channel" } else { "Remove channel" }, |s| {
        if add {
            add_channel(s, ts, c)?;
            if let Some(name) = name {
                let set = s.doc.project.texture_set_mut(ts)?;
                match name.is_empty() {
                    true => set.channel_names.remove(&c),
                    false => set.channel_names.insert(c, name),
                };
            }
            return Ok(());
        }
        if c == Channel::BaseColor {
            return Err(bad("textureSet.channel.remove", "base color can't be removed"));
        }
        let set = s.doc.project.texture_set_mut(ts)?;
        set.channels.retain(|(x, _)| *x != c);
        set.channel_names.remove(&c);
        Ok(())
    })?;
    Ok(Value::Null)
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

    #[test]
    fn user_channels_are_added_named_renamed_and_removed() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let mut run = |id: &str, p: Value| crate::execute(&mut s, id, &p);
        run("layer.add.fill", json!({})).unwrap();
        run("textureSet.channel.add", json!({"channel": "user2", "name": "  Wetness  "})).unwrap();
        let info = run("textureSet.inspect", json!({})).unwrap();
        let user = info["channels"].as_array().unwrap().iter().find(|c| c["channel"] == "user2").cloned().unwrap();
        assert_eq!((user["name"].as_str(), user["format"].as_str()), (Some("Wetness"), Some("RGB8")));
        // a present channel is renamed; "" clears the name
        run("textureSet.channel.add", json!({"channel": "User 2", "name": "Dirt"})).unwrap();
        assert_eq!(run("textureSet.inspect", json!({})).unwrap()["channels"].to_string().matches("Dirt").count(), 1);
        run("textureSet.channel.add", json!({"channel": "user2", "name": ""})).unwrap();
        assert!(!run("textureSet.inspect", json!({})).unwrap()["channels"].to_string().contains("\"name\""));
        // the layers get the channel; uncovered it shows white
        let layers = run("project.inspect", json!({})).unwrap().to_string();
        assert!(layers.contains("user2"), "{layers}");
        run("textureSet.channel.add", json!({"channel": "user2", "name": "x".repeat(10_000)})).unwrap();
        run("textureSet.channel.remove", json!({"channel": "user2"})).unwrap();
        assert!(s.doc.project.texture_sets[0].channel_names.is_empty() && !s.doc.project.texture_sets[0].has_channel(Channel::User2));
        assert_eq!(Channel::User2.default_value(), [1.0; 3]);
    }
}
