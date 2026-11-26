//! `import.verify`: compares the open project's composite with the results stored in the file it was imported from,
//! per texture set and channel, on the mesh's texels only, and grades each channel (`texturecraft-import-verify`).
//!
//! The file is only read: its stored results are bound to texture sets as the import binds them
//! (`texturecraft_import::stored`), and a set of the open project is matched by name. The project is composited as it
//! stands, with its "Original result" layers hidden (they hold the stored result itself), so after an import this
//! measures exactly what the import's safety net measured, and after edits it measures the edits.

use serde_json::Value;
use texturecraft_compose::{Composite, Inputs, all_tiles, composite_tiles};
use texturecraft_core::import_report::ORIGINAL_RESULT_KEY;
use texturecraft_core::{Channel, LayerId, Project};
use texturecraft_import::stored::{self, StoredResults, StoredSet};
use texturecraft_import_verify::{ChannelReport, IslandMask, SetReport, Tally, VerifyReport};

use crate::commands::{CommandSpec, always, bad, strp, u64p};
use crate::{Result, Session, cmd};

const ID: &str = "import.verify";
/// Largest border (texels) left out next to island borders.
const MAX_BORDER: u64 = 16;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "import.verify",
        "Verify against the original...",
        [],
        None,
        "{path?: the project file the open project was imported from (native only) | bytes?: that file as base64 (any target), name?: file name for the report (with bytes), set?: only the texture set of this name, border?: texels left out next to island borders (0-16; default 0)} → {source, border, sets: [{name, size, storedSize?, texels, grade, channels: [{channel, metrics: {mean, max, within1, within2, psnr, texels, samples, fit}, grade, outlier, safetyNetOnly, note}], notes}], totals: {exact, passes, near, off, safetyNetOnly, notCompared}, passes, text (the same as a table)}. Compares the open project's composite (its \"Original result\" layers hidden) with the results stored in the file, per set and channel, on the mesh's texels; differences in 8-bit steps of the stored encoding. Grades: exact (mean <= 0.5, >= 99 % within 1 step), passes (mean <= 0.5, >= 95 %), near (mean <= 2), off. Normal is a safety net only. Reads the file; changes nothing",
        always,
        verify
    )]
}

fn verify(s: &mut Session, p: &Value) -> Result<Value> {
    let border = match p.get("border").filter(|v| !v.is_null()) {
        None => 0,
        Some(_) => u64p(p, "border").filter(|b| *b <= MAX_BORDER).ok_or_else(|| bad(ID, format!("`border` must be a whole number from 0 to {MAX_BORDER}")))?,
    };
    let only = strp(p, "set");
    if let Some(name) = only
        && !s.doc.project.texture_sets.iter().any(|t| t.name == name)
    {
        return Err(bad(ID, format!("the project has no texture set \"{name}\"")));
    }
    let (results, label) = read_file(p)?;
    let report = s.verify_against(results, &format!("{label} against \"{}\"", s.doc.project.name), border as u32, only);
    let passes = report.passes();
    let mut v = serde_json::to_value(&report).map_err(|e| bad(ID, e.to_string()))?;
    if let Some(o) = v.as_object_mut() {
        o.insert("passes".into(), Value::Bool(passes));
        o.insert("text".into(), Value::String(report.to_string()));
    }
    Ok(v)
}

/// The stored results of the file from `bytes` or `path`, and the file's name for the report.
fn read_file(p: &Value) -> Result<(StoredResults, String)> {
    if let Some(v) = p.get("bytes").filter(|v| !v.is_null()) {
        let bytes = crate::import_cmds::bytes_param(v)?;
        let name = strp(p, "name").unwrap_or("the original file").to_string();
        let results = stored::read_bytes(bytes).map_err(|e| bad(ID, format!("{name}: {e}")))?;
        return Ok((results, name));
    }
    read_path(p)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_path(p: &Value) -> Result<(StoredResults, String)> {
    let path = strp(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(ID, "give `path` (the original project file) or `bytes`"))?;
    let results = stored::read_path(std::path::Path::new(path)).map_err(|e| bad(ID, format!("{path}: {e}")))?;
    let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    Ok((results, name))
}

#[cfg(target_arch = "wasm32")]
fn read_path(_: &Value) -> Result<(StoredResults, String)> {
    Err(bad(ID, "`path` is not available in the browser: give `bytes`"))
}

/// The project with every layer holding the original's stored result hidden.
fn without_original_results(project: &Project) -> Project {
    let mut p = project.clone();
    let mut ids: Vec<LayerId> = p.import.as_ref().map(|r| r.original_results().collect()).unwrap_or_default();
    ids.extend(p.layers.iter().filter(|(_, l)| l.lock.as_ref().is_some_and(|k| k.keys().contains(&ORIGINAL_RESULT_KEY))).map(|(id, _)| *id));
    for id in ids {
        if let Some(l) = p.layers.get_mut(&id) {
            l.visible = false;
        }
    }
    p
}

impl Session {
    /// Compares every texture set (or only `only`) with `results`; `border` texels next to island borders are left out.
    pub fn verify_against(&self, mut results: StoredResults, source: &str, border: u32, only: Option<&str>) -> VerifyReport {
        let mut report = VerifyReport::new(source, border);
        report.notes.extend(results.notes.iter().cloned());
        let project = without_original_results(&self.doc.project);
        for (ts, set) in project.texture_sets.iter().enumerate() {
            if only.is_some_and(|o| o != set.name) {
                continue;
            }
            let Some(stored) = results.set_mut(&set.name) else {
                report.notes.push(format!("texture set \"{}\": no stored result of that name in the file; not compared", set.name));
                continue;
            };
            match self.verify_set(&project, ts, stored, border) {
                Some(r) => report.push(r),
                None => report.notes.push(format!("texture set \"{}\": not ready to composite; not compared", set.name)),
            }
        }
        for s in &results.sets {
            if !project.texture_sets.iter().any(|t| t.name == s.name) && only.is_none() {
                report.notes.push(format!("stored result of \"{}\": no texture set of that name in the project", s.name));
            }
        }
        report
    }

    fn verify_set(&self, project: &Project, ts: usize, stored: &mut StoredSet, border: u32) -> Option<SetReport> {
        let set = project.texture_sets.get(ts)?;
        let cache = self.caches.get(ts)?;
        let (w, h) = set.size();
        let mut mask = IslandMask::from_gbuf(cache.gbuf());
        if (mask.width, mask.height) != (w, h) {
            return None;
        }
        mask.shrink(border);
        let mut out = SetReport::new(set.name.clone(), [w, h], mask.count());
        out.notes.extend(stored.notes.iter().cloned());
        let have = set.channel_list();
        let channels: Vec<Channel> = stored.channels.iter().map(|c| c.channel).filter(|c| have.contains(c)).collect();
        let empty = texturecraft_compose::BakedMaps::default();
        let inputs = Inputs {
            project,
            ts,
            mesh: &self.mesh,
            gbuf: cache.gbuf(),
            pixels: &self.doc.pixels,
            baked: self.doc.baked.get(&ts).unwrap_or(&empty),
            block_scales: cache.composite.block_scales.clone(),
            preview: None,
            images: &self.images,
            graphs: &self.doc.graphs,
            bounds: self.mesh.bounds(),
        };
        let mut ours = Composite::with_size(&channels, w, h);
        composite_tiles(&inputs, &mut ours, &all_tiles(w, h));
        for sc in &mut stored.channels {
            let id = sc.channel.id();
            if !have.contains(&sc.channel) {
                out.push(ChannelReport::not_compared(id, format!("the texture set has no {} channel", sc.channel.label())));
                continue;
            }
            let (sw, sh) = sc.size();
            if (sw, sh) != (w, h) {
                if w != h {
                    out.push(ChannelReport::not_compared(id, format!("stored result {sw}x{sh}, texture set {w}x{h} (not square); not compared")));
                    continue;
                }
                // as the import resamples it for a set of a capped resolution
                out.stored_size = Some([sw, sh]);
                sc.resampled(w);
            }
            let n = sc.channel.components();
            let mut tally = Tally::default();
            let mut compared_texels = 0usize;
            for y in 0..h {
                for x in 0..w {
                    if mask.contains(x, y) {
                        compared_texels += 1;
                        let a = sc.encode(ours.value(sc.channel, x, y));
                        let b = sc.final_look(x, y);
                        tally.add(a.get(..n).unwrap_or(&a), b.get(..n).unwrap_or(&b));
                    }
                }
            }
            if compared_texels == 0 {
                out.push(ChannelReport::not_compared(id, "no texel is left to compare after the border"));
                continue;
            }
            out.push(ChannelReport::compared(id, tally.finish()));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use texturecraft_core::import_report::original_result_layer;
    use texturecraft_core::{Fill, Layer, LayerKind, Source};
    use texturecraft_import::image::Rgba;
    use texturecraft_import::resources::CacheImage;
    use texturecraft_import::stored::StoredChannel;
    use texturecraft_import_verify::Grade;

    fn session() -> Session {
        Session::new_project("t", texturecraft_geom::primitives::builtin("plane").unwrap(), "builtin:plane", None, 128).unwrap()
    }

    fn fill(s: &mut Session, name: &str, v: f32) -> LayerId {
        let p = &mut s.doc.project;
        let mut f = Fill::default();
        f.sources.insert(Channel::Roughness, Source::Uniform([v; 3]));
        let l = Layer::new(p.alloc_id(), name, LayerKind::Fill(f), &[Channel::Roughness]);
        let id = l.id;
        p.insert_layer(0, l, None, false).unwrap();
        id
    }

    fn stored(name: &str, v: f32, size: u32) -> StoredResults {
        let image = CacheImage { bit: 0, image: Rgba::solid(size, size, [v, v, v, 1.0]), srgb: false, missing_bytes: 0 };
        let channel = StoredChannel { channel: Channel::Roughness, label: "Roughness".into(), image, default: [0.3; 3] };
        StoredResults { sets: vec![StoredSet { name: name.into(), channels: vec![channel], notes: Vec::new() }], notes: Vec::new() }
    }

    #[test]
    fn a_matching_composite_is_exact_and_a_different_one_is_off() {
        let mut s = session();
        fill(&mut s, "rough", 0.3);
        let set = s.doc.project.texture_sets[0].name.clone();
        let r = s.verify_against(stored(&set, 0.3, 128), "t", 0, None);
        let c = &r.sets[0].channels[0];
        assert_eq!((c.channel.as_str(), c.grade), ("roughness", Some(Grade::Exact)), "{r}");
        assert!(r.sets[0].texels > 1000 && r.passes());
        let off = s.verify_against(stored(&set, 0.9, 128), "t", 0, None);
        assert_eq!(off.sets[0].grade, Some(Grade::Off));
        assert!(off.sets[0].channels[0].outlier && !off.passes());
        // a stored result of another size is resampled to the set's
        let big = s.verify_against(stored(&set, 0.3, 256), "t", 1, None);
        assert_eq!((big.sets[0].stored_size, big.sets[0].grade), (Some([256, 256]), Some(Grade::Exact)));
        assert!(big.sets[0].texels < r.sets[0].texels, "the border leaves texels out");
    }

    #[test]
    fn layers_holding_the_stored_result_are_left_out_of_the_composite() {
        let mut s = session();
        fill(&mut s, "rough", 0.3);
        // a shown layer locked as the original's result, with other values on top of the stack
        let top = fill(&mut s, "original", 0.9);
        let locked = original_result_layer(top, &[Channel::Roughness], true);
        if let Some(l) = s.doc.project.layers.get_mut(&top) {
            l.lock = locked.lock;
        }
        let set = s.doc.project.texture_sets[0].name.clone();
        let r = s.verify_against(stored(&set, 0.3, 128), "t", 0, None);
        assert_eq!(r.sets[0].grade, Some(Grade::Exact), "{r}");
        assert!(s.doc.project.layers[&top].visible, "the session's project is not changed");
    }

    #[test]
    fn sets_and_channels_without_a_counterpart_are_reported_not_compared() {
        let mut s = session();
        let r = s.verify_against(stored("Elsewhere", 0.3, 128), "t", 0, None);
        assert!(r.sets.is_empty() && r.notes.iter().any(|n| n.contains("Elsewhere")) && !r.passes());
        let set = s.doc.project.texture_sets[0].name.clone();
        let mut results = stored(&set, 0.3, 128);
        results.sets[0].channels[0].channel = Channel::SheenColor;
        let r = s.verify_against(results, "t", 0, None);
        assert!(r.sets[0].channels[0].grade.is_none() && r.totals.not_compared == 1, "{r}");
        // bad params are errors, never panics
        for p in
            [json!({}), json!({"bytes": "AAAA"}), json!({"path": "/nonexistent/file"}), json!({"border": 99, "path": "x"}), json!({"set": "nope", "path": "x"})]
        {
            assert!(verify(&mut s, &p).is_err(), "{p}");
        }
    }
}
