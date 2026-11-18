//! Smart masks: procedural mask presets (generators and patterns driven by the baked mesh maps)
//! that replace a layer's mask in one step. Smart materials build their masks from the same
//! presets. A user smart mask is a layer's mask saved to a `.tcmask` file (the `.tcraft`
//! container holding one layer that carries the mask) and applied back like a preset.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use texturecraft_core::{
    BlendMode, Effect, EffectKind, Filter, Generator, GeneratorKind, Layer, LayerId, LayerKind, Mask, MaskBackground, Procedural, ProceduralKind, Project,
    Projection, Source,
};

use crate::commands::{CommandSpec, always, bad, has_layer, layer_param, strp};
use crate::session::StackTarget;
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mask.smart.apply", "Apply smart mask", [], None, "{name: smart mask id or label, layer?} or {path: .tcmask file, layer?}", has_layer, apply_cmd),
        cmd!(
            "mask.smart.create",
            "Create smart mask",
            [],
            None,
            "{layer?, path: .tcmask file, name?} — saves the layer's mask (all effects, parameters, painted pixels and images) as a user smart mask",
            has_layer,
            create_cmd
        ),
        cmd!("mask.smart.list", "List smart masks", [], None, "{}", always, |_, _| Ok(json!(
            SMART_MASKS.iter().map(|m| json!({"id": m.id, "label": m.label})).collect::<Vec<_>>()
        ))),
    ]
}

/// Smart mask file extension.
pub const SMART_MASK_EXT: &str = "tcmask";

pub struct SmartMask {
    pub id: &'static str,
    pub label: &'static str,
}

pub const SMART_MASKS: &[SmartMask] = &[
    SmartMask { id: "edge_wear", label: "Edges Worn" },
    SmartMask { id: "edge_chipped", label: "Edges Chipped" },
    SmartMask { id: "dirt_cavities", label: "Dirt in Cavities" },
    SmartMask { id: "dust_top", label: "Dust from Top" },
    SmartMask { id: "ground_dirt", label: "Ground Dirt" },
    SmartMask { id: "rust_leaks", label: "Rust Leaks" },
    SmartMask { id: "scratches", label: "Scratches" },
    SmartMask { id: "grunge", label: "Grunge Blotches" },
    SmartMask { id: "paint_chips", label: "Paint Chips" },
    SmartMask { id: "fingerprints", label: "Fingerprints" },
    SmartMask { id: "edges_soft", label: "Edges Soft" },
    SmartMask { id: "edges_sharp", label: "Edges Sharp" },
    SmartMask { id: "exposed", label: "Exposed Areas" },
    SmartMask { id: "thin_areas", label: "Thin Areas" },
    SmartMask { id: "dust_cavities", label: "Dust in Cavities" },
    SmartMask { id: "moss_top", label: "Moss from Top" },
    SmartMask { id: "water_stains", label: "Water Stains" },
    SmartMask { id: "sun_faded", label: "Sun Faded" },
    SmartMask { id: "snow_top", label: "Snow from Top" },
    SmartMask { id: "moss_ground", label: "Moss from Ground" },
    SmartMask { id: "rust_cavities", label: "Rust in Cavities" },
    SmartMask { id: "edge_scratches", label: "Edge Scratches" },
    SmartMask { id: "dirt_streaks", label: "Dirt Streaks" },
    SmartMask { id: "occluded", label: "Occluded Areas" },
];

fn norm(x: &str) -> String {
    x.to_ascii_lowercase().replace([' ', '_', '-'], "")
}

pub fn find(name: &str) -> Option<&'static SmartMask> {
    SMART_MASKS.iter().find(|m| norm(m.id) == norm(name) || norm(m.label) == norm(name))
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// A mesh-map generator with explicit threshold, softness and noise breakup (curvature as baked, without detail sizes).
pub(crate) fn generator(kind: GeneratorKind, level: f32, softness: f32, noise: f32) -> EffectKind {
    EffectKind::Generator(Generator { level, softness, noise_amount: noise, detail: None, ..Generator::new(kind) })
}

/// A direction or position generator (`invert` flips the result).
pub(crate) fn directional(kind: GeneratorKind, direction: [f32; 3], level: f32, softness: f32, noise: f32, invert: bool) -> EffectKind {
    EffectKind::Generator(Generator { level, softness, noise_amount: noise, direction, invert, detail: None, ..Generator::new(kind) })
}

/// A triplanar grayscale pattern fill
pub(crate) fn pattern(kind: ProceduralKind, scale: f32, seed: u32, contrast: f32) -> EffectKind {
    EffectKind::Fill {
        source: Source::Procedural { pattern: Procedural { scale, seed, contrast, ..Procedural::new(kind) }, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() },
        projection: Projection::Triplanar { scale: 1.0, sharpness: 8.0, offset: [0.0; 3] },
    }
}

/// Background and effects (bottom → top) of preset `id`.
pub(crate) fn preset(id: &str) -> Option<(MaskBackground, Vec<(EffectKind, BlendMode)>)> {
    use BlendMode::*;
    use GeneratorKind as G;
    use ProceduralKind as P;
    let fx = match find(id)?.id {
        "edge_wear" => vec![(generator(G::EdgeWear, 0.55, 0.12, 0.5), Normal)],
        // Wide edge band cut by chips, plus a thin solid line right on the edge.
        "edge_chipped" => vec![
            (generator(G::EdgeWear, 0.8, 0.2, 0.3), Normal),
            (pattern(P::Chips, 3.0, 5, 1.0), Multiply),
            (generator(G::EdgeWear, 0.45, 0.06, 0.6), Lighten),
        ],
        "dirt_cavities" => vec![(generator(G::Dirt, 0.55, 0.25, 0.4), Normal)],
        "dust_top" => vec![(directional(G::Direction, UP, 0.3, 0.15, 0.3, false), Normal), (pattern(P::Clouds, 3.0, 9, 1.4), Multiply)],
        "ground_dirt" => vec![(directional(G::Position, [0.0, -1.0, 0.0], 0.35, 0.25, 0.5, false), Normal), (pattern(P::Grunge, 2.0, 12, 0.8), Multiply)],
        // Streaks on surfaces that don't face up, fed by the crevices they run out of.
        "rust_leaks" => vec![
            (pattern(P::Streaks, 3.0, 4, 1.2), Normal),
            (directional(G::Direction, UP, 0.2, 0.1, 0.0, true), Multiply),
            (generator(G::Dirt, 0.45, 0.2, 0.5), Lighten),
        ],
        "scratches" => vec![(pattern(P::Scratches, 3.0, 6, 1.0), Normal)],
        "grunge" => vec![(pattern(P::Grunge, 2.0, 8, 1.0), Normal)],
        "paint_chips" => vec![(pattern(P::Chips, 5.0, 2, 1.0), Normal), (generator(G::EdgeWear, 0.6, 0.12, 0.5), Lighten)],
        "fingerprints" => vec![(pattern(P::Smudges, 4.0, 3, 1.0), Normal)],
        // A wide, soft highlight on convex edges, and a thin clean line right on them.
        "edges_soft" => vec![(generator(G::EdgeWear, 0.35, 0.35, 0.15), Normal)],
        "edges_sharp" => vec![(generator(G::EdgeWear, 0.65, 0.05, 0.0), Normal)],
        // Open surfaces that light reaches (the opposite of cavities).
        "exposed" => vec![(generator(G::AmbientOcclusion, 0.5, 0.3, 0.0), Normal)],
        "thin_areas" => vec![(generator(G::ThinAreas, 0.3, 0.2, 0.2), Normal)],
        // Cavities filled with soft, cloudy dust.
        "dust_cavities" => vec![(generator(G::Dirt, 0.5, 0.3, 0.2), Normal), (pattern(P::Clouds, 4.0, 21, 1.2), Multiply)],
        // Patches growing on what faces up, broken up by cells.
        "moss_top" => vec![(directional(G::Direction, UP, 0.45, 0.2, 0.6, false), Normal), (pattern(P::Cells, 6.0, 17, 1.4), Multiply)],
        // Tide marks: streaks running down the sides, faded by clouds.
        "water_stains" => vec![
            (pattern(P::Streaks, 2.0, 13, 0.9), Normal),
            (directional(G::Direction, UP, 0.15, 0.2, 0.0, true), Multiply),
            (pattern(P::Clouds, 2.5, 14, 1.0), Multiply),
        ],
        // Surfaces facing the sky, softly graded.
        "sun_faded" => vec![(directional(G::Direction, UP, 0.5, 0.45, 0.1, false), Normal)],
        // A crisp cap on what faces up, with a ragged border.
        "snow_top" => vec![(directional(G::Direction, UP, 0.6, 0.08, 0.6, false), Normal)],
        // Growth creeping up from the lowest parts, in clumps.
        "moss_ground" => vec![(directional(G::Position, [0.0, -1.0, 0.0], 0.4, 0.3, 0.5, false), Normal), (pattern(P::Cells, 6.0, 25, 1.4), Multiply)],
        // Cavities eaten by blotchy corrosion.
        "rust_cavities" => vec![(generator(G::Dirt, 0.6, 0.25, 0.5), Normal), (pattern(P::Grunge, 3.0, 27, 1.3), Multiply)],
        // Scratches gathered where edges get handled.
        "edge_scratches" => vec![(generator(G::EdgeWear, 0.75, 0.3, 0.2), Normal), (pattern(P::Scratches, 4.0, 29, 1.2), Multiply)],
        // Grime running down from the crevices it collects in.
        "dirt_streaks" => vec![(pattern(P::Streaks, 2.5, 31, 1.1), Normal), (generator(G::Dirt, 0.5, 0.25, 0.4), Multiply)],
        // Hidden surfaces light hardly reaches (the opposite of exposed areas).
        "occluded" => vec![(directional(G::AmbientOcclusion, UP, 0.5, 0.3, 0.0, true), Normal)],
        _ => return None,
    };
    Some((MaskBackground::Black, fx))
}

/// Build a mask from a background and effects, allocating effect ids in `p`.
pub(crate) fn build_mask(p: &mut Project, bg: MaskBackground, effects: Vec<(EffectKind, BlendMode)>) -> Mask {
    let mut m = Mask::new(bg);
    for (k, blend) in effects {
        m.effects.push(Effect { blend, ..Effect::new(p.alloc_id(), k) });
    }
    m
}

/// Preset `id` as a mask, optionally inverted (e.g. paint that survives where wear is not).
pub(crate) fn mask(p: &mut Project, id: &str, invert: bool) -> Result<Mask> {
    let (bg, mut fx) = preset(id).ok_or_else(|| EngineError::Other(format!("unknown smart mask `{id}`")))?;
    if invert {
        fx.push((EffectKind::Filter(Filter::Invert), BlendMode::Normal));
    }
    Ok(build_mask(p, bg, fx))
}

/// Replace layer `layer`'s mask with smart mask `name` (one undo step).
pub fn apply(s: &mut Session, layer: LayerId, name: &str) -> Result<()> {
    let sm = find(name).ok_or_else(|| EngineError::Other(format!("unknown smart mask `{name}`")))?;
    s.edit(&crate::session::history_label(crate::session::APPLY_SMART_MASK_TEMPLATE, sm.label), |s| {
        let m = mask(&mut s.doc.project, sm.id, false)?;
        let old = s.doc.project.layer_mut(layer)?.mask.replace(m);
        for e in old.map(|m| m.effects).unwrap_or_default() {
            s.doc.pixels.masks.remove(&e.id);
        }
        s.target = StackTarget::Mask;
        Ok(())
    })
}

impl Session {
    /// Serialize layer `layer`'s mask (an instance without its own mask has its source's) as a user smart mask file named `name`.
    pub fn smart_mask_bytes(&self, layer: LayerId, name: &str) -> Result<Vec<u8>> {
        let owner = self.doc.project.mask_owner(layer);
        let mask = self.doc.project.layer(owner)?.mask.clone().ok_or_else(|| EngineError::Other("the layer has no mask".into()))?;
        let (ts, _, _) = self.doc.project.locate(owner).ok_or_else(|| EngineError::Other("layer not in a stack".into()))?;
        let set = self.doc.project.texture_set(ts)?;
        let mut mini = crate::material_lib::one_set_document(set, name);
        let id = mini.project.alloc_id();
        let mut holder = Layer::new(id, name, LayerKind::Paint, &set.channel_list());

        // The mask comes back switched on whatever state it was saved in
        holder.mask = Some(Mask { enabled: true, ..copy_mask(&self.doc, &mask, &mut mini, set.size())? });
        mini.project.insert_layer(0, holder, None, false)?;
        self.file_bytes(mini, id)
    }

    /// Replace layer `layer`'s mask with the one in a user smart mask file (one undo step).
    pub fn apply_smart_mask_bytes(&mut self, layer: LayerId, bytes: &[u8]) -> Result<()> {
        let pf = texturecraft_formats::load_project_bytes(bytes).map_err(|e| EngineError::Other(e.to_string()))?;
        self.apply_smart_mask_file(layer, pf)
    }

    fn apply_smart_mask_file(&mut self, layer: LayerId, pf: texturecraft_formats::ProjectFile) -> Result<()> {
        let mask = pf
            .project
            .texture_sets
            .first()
            .and_then(|t| t.stack.last().copied())
            .and_then(|root| pf.project.layers.get(&root))
            .and_then(|l| l.mask.clone())
            .ok_or_else(|| EngineError::Other("the smart mask file has no mask".into()))?;
        let resources: BTreeMap<String, Arc<Vec<u8>>> = pf.resources.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        let src = crate::session::Document {
            bake_paint: BTreeMap::new(),
            project: pf.project,
            pixels: pf.pixels,
            baked: BTreeMap::new(),
            resources: BTreeMap::new(),
        };
        let label = src.project.texture_sets.first().map(|t| t.name.clone()).unwrap_or_default();
        self.edit(&format!("Apply smart mask {label}"), |s| {
            let (ts, _, _) = s.doc.project.locate(layer).ok_or_else(|| EngineError::Other("layer not in a stack".into()))?;
            let size = s.doc.project.texture_set(ts)?.size();

            // An image whose name the project already uses for other bytes comes in under a new name
            let mut renames = BTreeMap::new();
            for (name, bytes) in &resources {
                let mut taken = name.clone();
                let mut number = 2;
                while s.doc.resources.get(&taken).is_some_and(|have| have != bytes) {
                    let (stem, ext) = name.rsplit_once('.').map_or((name.as_str(), String::new()), |(stem, ext)| (stem, format!(".{ext}")));
                    taken = format!("{stem}_{number}{ext}");
                    number += 1;
                }
                s.doc.resources.entry(taken.clone()).or_insert_with(|| bytes.clone());
                if taken != *name {
                    renames.insert(name.clone(), taken);
                }
            }
            let mut new = copy_mask(&src, &mask, &mut s.doc, size)?;
            for source in new.effects.iter_mut().flat_map(|effect| sources_mut(&mut effect.kind)) {
                if let Source::Image { name, .. } = source
                    && let Some(renamed) = renames.get(name.as_str())
                {
                    *name = renamed.clone();
                }
            }
            let old = s.doc.project.layer_mut(layer)?.mask.replace(new);
            for e in old.map(|m| m.effects).unwrap_or_default() {
                s.doc.pixels.masks.remove(&e.id);
            }
            s.target = StackTarget::Mask;
            Ok(())
        })?;
        self.sync_images();
        Ok(())
    }
}

/// The sources an effect reads.
fn sources_mut(kind: &mut EffectKind) -> Vec<&mut Source> {
    match kind {
        EffectKind::Fill { source, .. } => vec![source],
        EffectKind::Material(fill) => fill.sources.values_mut().collect(),
        EffectKind::Generator(generator) => generator.inputs.values_mut().collect(),
        _ => Vec::new(),
    }
}

/// `mask` of document `src` with fresh effect ids from `dst` (anchor sources follow), its painted pixels moved to `dst` at `size`.
fn copy_mask(src: &crate::session::Document, mask: &Mask, dst: &mut crate::session::Document, size: (u32, u32)) -> Result<Mask> {
    let mut copy = mask.clone();
    let mut moved = BTreeMap::new();
    for e in &mut copy.effects {
        let old = e.id;
        e.id = dst.project.alloc_id();
        if matches!(e.kind, EffectKind::Anchor) {
            moved.insert(old, e.id);
        }
        if let Some(img) = src.pixels.masks.get(&old) {
            dst.pixels.masks.insert(e.id, crate::layer_ops::fit(img, size, &[0, 0]));
        }
    }

    // Sources that read an anchor effect of this mask follow it to its new id
    for source in copy.effects.iter_mut().flat_map(|effect| sources_mut(&mut effect.kind)) {
        if let Source::Anchor { anchor, .. } = source {
            *anchor = *moved.get(anchor).ok_or_else(|| EngineError::Other(format!("smart mask refers to anchor {} outside this mask", anchor.0)))?;
        }
    }
    Ok(copy)
}

fn apply_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = layer_param(s, "mask.smart.apply", p)?;
    if let Some(path) = strp(p, "path") {
        apply_file(s, layer, path)?;
    } else {
        let name = strp(p, "name").ok_or_else(|| bad("mask.smart.apply", "`name` or `path` is required"))?;
        apply(s, layer, name)?;
    }
    Ok(json!({ "layer": layer.0 }))
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_file(s: &mut Session, layer: LayerId, path: &str) -> Result<()> {
    let file = texturecraft_formats::load_project(std::path::Path::new(path)).map_err(|e| bad("mask.smart.apply", format!("cannot read {path}: {e}")))?;
    s.apply_smart_mask_file(layer, file)
}

#[cfg(target_arch = "wasm32")]
fn apply_file(_: &mut Session, _: LayerId, _: &str) -> Result<()> {
    Err(EngineError::Other("file paths are not available in the browser".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn create_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = layer_param(s, "mask.smart.create", p)?;
    let path = strp(p, "path").ok_or_else(|| bad("mask.smart.create", "`path` is required"))?;
    let name = strp(p, "name").map(str::to_string).unwrap_or_else(|| s.doc.project.layer(layer).map(|l| l.name.clone()).unwrap_or_default());
    let bytes = s.smart_mask_bytes(layer, &name)?;
    let path = if path.ends_with(&format!(".{SMART_MASK_EXT}")) { path.to_string() } else { format!("{path}.{SMART_MASK_EXT}") };
    if let Some(dir) = std::path::Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| bad("mask.smart.create", format!("cannot create {}: {e}", dir.display())))?;
    }

    texturecraft_formats::write_atomic(std::path::Path::new(&path), &bytes).map_err(|e| bad("mask.smart.create", format!("cannot write {path}: {e}")))?;
    Ok(json!({ "path": path }))
}

#[cfg(target_arch = "wasm32")]
fn create_cmd(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_resolves_and_ids_are_unique() {
        let (mut ids, mut labels) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
        for m in SMART_MASKS {
            assert!(preset(m.id).is_some(), "{}", m.id);
            assert!(ids.insert(norm(m.id)) && labels.insert(norm(m.label)), "duplicate {}", m.id);
            assert_eq!(find(m.label).map(|f| f.id), Some(m.id));
        }
    }
}
