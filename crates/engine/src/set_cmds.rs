//! Texture set commands (Texture Set List / Texture Set Settings).

use serde_json::{Value, json};
use texturecraft_core::{BakedMap, Channel, ChannelBlend, ChannelFormat};

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
            "{ts?} → {index, name, resolution, size: [w, h], uvTiles: [UDIM numbers] (empty: no UV tiles), tiles: [{udim, block: [x, y]}], channels, layers, baked, visible, active}",
            always,
            |s, p| Ok(crate::inspect::texture_set(s, ts_param(s, p, "textureSet.inspect")?))
        ),
        cmd!("textureSet.resolution", "Texture set size", [], None, "{ts?, resolution: 128..8192 (power of two)}", always, resolution),
        cmd!("textureSet.channel.add", "Add channel", [], None, "{ts?, channel}", always, |s, p| channel(s, p, true)),
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

        // Install the new caches before adding history to avoid rebuilding them
        if let (Some(c), Some(slot)) = (cache, self.caches.get_mut(ts)) {
            let old = std::mem::replace(slot, c);

            // Release large caches without blocking the UI
            #[cfg(not(target_arch = "wasm32"))]
            let _ = std::thread::Builder::new().name("texturecraft-free".into()).spawn(move || drop(old));
            #[cfg(target_arch = "wasm32")]
            drop(old);
        }
        self.log.extend_warnings(log);
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
        if let Some(n) = nf
            && s.doc.project.normal_format != n
        {
            s.doc.project.normal_format = n;
            for (ts, texture_set) in s.doc.project.texture_sets.iter().enumerate() {
                if let Some(imported_tiles) = texture_set.imported_tiles.get(&BakedMap::Normal).filter(|tiles| !tiles.is_empty())
                    && let Some(baked) = s.doc.baked.get_mut(&ts)
                {
                    let size = (baked.resolution, baked.height(BakedMap::Normal));
                    if let Some(normal_map) = baked.maps.get_mut(&BakedMap::Normal) {
                        let mut flipped = (**normal_map).clone();
                        if let Some(grid) = texture_set.tile_grid() {
                            let tile_width = size.0 / grid.cols.max(1);
                            let tile_height = size.1 / grid.rows.max(1);
                            for &tile in imported_tiles {
                                if let Some((col, row)) = grid.block(tile) {
                                    for y in row * tile_height..(row + 1) * tile_height {
                                        let first = (y as usize * size.0 as usize + col as usize * tile_width as usize) * 3;
                                        let last = first + tile_width as usize * 3;
                                        if let Some(row_pixels) = flipped.get_mut(first..last) {
                                            for pixel in row_pixels.as_chunks_mut::<3>().0 {
                                                pixel[1] = 1.0 - pixel[1];
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            for pixel in flipped.as_chunks_mut::<3>().0 {
                                pixel[1] = 1.0 - pixel[1];
                            }
                        }
                        *normal_map = Arc::new(flipped);
                    }
                }
            }
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
    fn settings_changing_normal_format_flips_imported_normal_mesh_map_not_baked() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        // Texture set 0 has an imported normal map with green = 0.2
        let entry0 = s.doc.baked.entry(0).or_default();
        entry0.resolution = 1;
        entry0.maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.2, 1.0]));
        let set0 = s.doc.project.texture_set_mut(0).unwrap();
        set0.baked.insert(BakedMap::Normal);
        set0.imported_tiles.insert(BakedMap::Normal, [1001].into_iter().collect());

        // Texture set 1 has an internally baked normal map stored through store_bake_maps
        let mut baked_maps = std::collections::BTreeMap::new();
        baked_maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.3, 1.0]));
        s.store_bake_maps(1, 1, 1, baked_maps, None, None).unwrap();
        assert!(!s.doc.project.texture_set(1).unwrap().imported_tiles.contains_key(&BakedMap::Normal));

        crate::execute(&mut s, "textureSet.settings", &json!({"normalFormat": "directX"})).unwrap();
        assert_eq!(s.doc.project.normal_format, texturecraft_core::NormalFormat::DirectX);
        let green_imported = s.doc.baked[&0].maps[&BakedMap::Normal][1];
        assert!((green_imported - 0.8).abs() < 1e-4, "imported normal green should flip to 0.8: {green_imported}");
        let green_baked = s.doc.baked[&1].maps[&BakedMap::Normal][1];
        assert!((green_baked - 0.3).abs() < 1e-4, "internally baked normal should remain unchanged: {green_baked}");

        // Now simulate importing onto set 1, verifying imported_tiles is set, then rebaking via store_bake_maps clears imported flag
        s.doc.project.texture_set_mut(1).unwrap().imported_tiles.insert(BakedMap::Normal, [1001].into_iter().collect());
        let mut rebake_maps = std::collections::BTreeMap::new();
        rebake_maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.3, 1.0]));
        s.store_bake_maps(1, 1, 1, rebake_maps, None, None).unwrap();
        assert!(!s.doc.project.texture_set(1).unwrap().imported_tiles.contains_key(&BakedMap::Normal), "rebaking must clear imported flag");

        crate::execute(&mut s, "textureSet.settings", &json!({"normalFormat": "openGl"})).unwrap();
        assert_eq!(s.doc.project.normal_format, texturecraft_core::NormalFormat::OpenGl);
        let green_imported_back = s.doc.baked[&0].maps[&BakedMap::Normal][1];
        assert!((green_imported_back - 0.2).abs() < 1e-4, "imported normal green should flip back to 0.2: {green_imported_back}");
        let green_baked_still = s.doc.baked[&1].maps[&BakedMap::Normal][1];
        assert!((green_baked_still - 0.3).abs() < 1e-4, "internally baked normal should still remain unchanged: {green_baked_still}");
    }

    #[test]
    fn baking_other_maps_preserves_imported_normal_provenance() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let entry0 = s.doc.baked.entry(0).or_default();
        entry0.resolution = 1;
        entry0.maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.2, 1.0]));
        let set0 = s.doc.project.texture_set_mut(0).unwrap();
        set0.baked.insert(BakedMap::Normal);
        set0.imported_tiles.insert(BakedMap::Normal, [1001].into_iter().collect());
        set0.bake.maps.select([BakedMap::AmbientOcclusion]);

        // Bake only AmbientOcclusion
        let mut ao_bake = std::collections::BTreeMap::new();
        ao_bake.insert(BakedMap::AmbientOcclusion, Arc::new(vec![1.0]));
        s.store_bake_maps(0, 1, 1, ao_bake, Some(&[BakedMap::AmbientOcclusion]), None).unwrap();
        assert!(s.doc.project.texture_set(0).unwrap().imported_tiles.contains_key(&BakedMap::Normal), "baking only AO must preserve imported Normal provenance");

        crate::execute(&mut s, "textureSet.settings", &json!({"normalFormat": "directX"})).unwrap();
        let green = s.doc.baked[&0].maps[&BakedMap::Normal][1];
        assert!((green - 0.8).abs() < 1e-4, "imported normal green should flip to 0.8 after baking other maps: {green}");
    }

    #[test]
    fn excluded_tiles_without_bake_flips_all_imported_tiles() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let set = s.doc.project.texture_set_mut(0).unwrap();
        set.uv_tiles = vec![1001, 1002];
        set.baked.insert(BakedMap::Normal);
        set.imported_tiles.insert(BakedMap::Normal, [1001, 1002].into_iter().collect());

        // Exclude tile 1002 via real bake.tile.set command
        crate::execute(&mut s, "bake.tile.set", &json!({"tile": 1002, "enabled": false})).unwrap();
        assert!(s.doc.project.texture_sets[0].bake_excluded_tiles.contains(&1002));

        // 2x1 atlas: pixel 0 is tile 1001 (green = 0.2), pixel 1 is tile 1002 (green = 0.2)
        let entry = s.doc.baked.entry(0).or_default();
        entry.resolution = 2;
        entry.maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.2, 1.0, 0.5, 0.2, 1.0]));

        // Switch to DirectX without baking: both imported tiles must flip
        crate::execute(&mut s, "textureSet.settings", &json!({"normalFormat": "directX"})).unwrap();
        let pixels = &s.doc.baked[&0].maps[&BakedMap::Normal];
        assert!((pixels[1] - 0.8).abs() < 1e-4, "imported tile 1001 must flip to 0.8: {}", pixels[1]);
        assert!((pixels[4] - 0.8).abs() < 1e-4, "imported tile 1002 must flip to 0.8: {}", pixels[4]);
    }

    #[test]
    fn exclusions_changed_after_partial_bake_preserves_per_tile_provenance() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let set = s.doc.project.texture_set_mut(0).unwrap();
        set.uv_tiles = vec![1001, 1002];
        set.baked.insert(BakedMap::Normal);
        set.imported_tiles.insert(BakedMap::Normal, [1001, 1002].into_iter().collect());

        // Atlas of 2 tiles: 1001 (baked, col 0) has green = 0.3, 1002 (excluded, imported, col 1) has green = 0.2
        let entry = s.doc.baked.entry(0).or_default();
        entry.resolution = 2;
        entry.maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.3, 1.0, 0.5, 0.2, 1.0]));

        // Partial bake where 1002 is excluded: store_bake_maps prunes only 1001 from imported_tiles
        let mut maps = std::collections::BTreeMap::new();
        maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.3, 1.0, 0.5, 0.2, 1.0]));
        let baked_tiles: std::collections::BTreeSet<u32> = [1001].into_iter().collect();
        s.store_bake_maps(0, 2, 1, maps, Some(&[BakedMap::Normal]), Some(&baked_tiles)).unwrap();

        assert_eq!(s.doc.project.texture_sets[0].imported_tiles.get(&BakedMap::Normal).unwrap(), &[1002].into_iter().collect::<std::collections::BTreeSet<_>>());

        // Now exclusions change after the bake (e.g. 1002 re-enabled or 1001 excluded) without rebaking
        crate::execute(&mut s, "bake.tile.set", &json!({"tile": 1002, "enabled": true})).unwrap();
        crate::execute(&mut s, "bake.tile.set", &json!({"tile": 1001, "enabled": false})).unwrap();

        // Switch to DirectX: only the unbaked tile 1002 flips, freshly baked tile 1001 stays unchanged
        crate::execute(&mut s, "textureSet.settings", &json!({"normalFormat": "directX"})).unwrap();
        let pixels = &s.doc.baked[&0].maps[&BakedMap::Normal];
        assert!((pixels[1] - 0.3).abs() < 1e-4, "baked tile 1001 must not flip: {}", pixels[1]);
        assert!((pixels[4] - 0.8).abs() < 1e-4, "imported tile 1002 must flip to 0.8: {}", pixels[4]);
    }

    #[test]
    fn mid_bake_selection_change_uses_task_snapshot() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let set = s.doc.project.texture_set_mut(0).unwrap();
        set.baked.insert(BakedMap::Normal);
        set.imported_tiles.insert(BakedMap::Normal, [1001].into_iter().collect());
        set.bake.maps.select([BakedMap::Normal]);

        // Create streamed bake with task snapshot where Normal was enabled
        let mut maps = std::collections::BTreeMap::new();
        maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.3, 1.0]));
        let streamed = crate::bake::StreamedBake {
            resolution: 1,
            height: 1,
            maps,
            tracer: "cpu".into(),
            baked_maps: vec![BakedMap::Normal],
            baked_tiles: [1001].into_iter().collect(),
        };

        // Mid-bake: user changes settings to deselect Normal
        s.doc.project.texture_set_mut(0).unwrap().bake.maps.select([BakedMap::AmbientOcclusion]);

        // Applying the streamed bake must use the task snapshot and clear imported provenance for Normal
        s.apply_streamed_bake(0, streamed).unwrap();
        assert!(!s.doc.project.texture_sets[0].imported_tiles.contains_key(&BakedMap::Normal), "task snapshot must clear imported Normal even if deselected mid-bake");
    }

    #[test]
    fn undo_and_save_load_preserves_imported_tiles() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let entry = s.doc.baked.entry(0).or_default();
        entry.resolution = 1;
        entry.maps.insert(BakedMap::Normal, Arc::new(vec![0.5, 0.2, 1.0]));
        let set = s.doc.project.texture_set_mut(0).unwrap();
        set.baked.insert(BakedMap::Normal);
        set.imported_tiles.insert(BakedMap::Normal, [1001].into_iter().collect());

        // Switch to DirectX
        crate::execute(&mut s, "textureSet.settings", &json!({"normalFormat": "directX"})).unwrap();
        assert!((s.doc.baked[&0].maps[&BakedMap::Normal][1] - 0.8).abs() < 1e-4);

        // Save to bytes and reload
        let bytes = s.to_project_bytes().unwrap();
        let reloaded = Session::from_project_bytes(&bytes).unwrap();
        assert_eq!(reloaded.doc.project.texture_sets[0].imported_tiles, s.doc.project.texture_sets[0].imported_tiles);

        // Undo flips back to 0.2 and restores OpenGl
        s.undo().unwrap();
        assert_eq!(s.doc.project.normal_format, texturecraft_core::NormalFormat::OpenGl);
        assert!((s.doc.baked[&0].maps[&BakedMap::Normal][1] - 0.2).abs() < 1e-4);
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
}
