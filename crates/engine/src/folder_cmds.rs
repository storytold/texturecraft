//! A folder's actions on everything inside it: the Scale and Rotation laid on its fills, and new seeds for its patterns

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use texturecraft_core::project::MAX_DEPTH;
use texturecraft_core::{ContentsOffset, EffectKind, Fill, Filter, Layer, LayerId, LayerKind, Project, RandomizedSeed, Source};

use crate::commands::{CommandSpec, bad, f32p, has_layer, layer_param};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "folder.contents.set",
            "Folder contents",
            [],
            None,
            "{layer?: folder (default selected), scale?: 0.01..100 multiplying every fill's tiling inside, rotation?: degrees added to every fill's angle inside (wrapped to -180..180), merge?} — relative to each fill's own and never written into the fills; one undo step",
            has_layer,
            set_contents
        ),
        cmd!(
            "folder.contents.reset",
            "Reset folder contents",
            [],
            None,
            "{layer?: folder (default selected)} — drop the folder's Scale and Rotation: every fill inside back to its own, and put back the seeds its Randomize seeds changed (a seed changed since, or a layer moved out, keeps its own), in one undo step",
            has_layer,
            reset_contents
        ),
        cmd!(
            "folder.randomizeSeeds",
            "Randomize seeds",
            [],
            None,
            "{layer?: folder (default selected)} — a new seed for every pattern, generator and noise filter inside the folder (fills, masks, effects, nested folders; instanced sources stay), in one undo step; seeds that were equal stay equal; the folder keeps each seed's value from before so its Contents reset can put it back. Returns {seeds: how many changed}",
            has_layer,
            randomize_seeds
        ),
    ]
}

/// The folder a command acts on: the `layer` param or the selected layer, refused unless it is a folder
fn folder_param(s: &Session, cmd: &str, p: &Value) -> Result<LayerId> {
    let id = layer_param(s, cmd, p)?;
    match s.doc.project.layers.get(&id) {
        Some(layer) if layer.is_group() => Ok(id),
        _ => Err(bad(cmd, format!("layer {} is not a folder", id.0))),
    }
}

/// The Scale and Rotation folder `id` lays on its fills
fn contents_mut(project: &mut Project, id: LayerId) -> Option<&mut ContentsOffset> {
    match &mut project.layers.get_mut(&id)?.kind {
        LayerKind::Group { contents, .. } => Some(contents),
        _ => None,
    }
}

/// The seeds inside folder `id` that its Randomize seeds changed
fn randomized_mut(project: &mut Project, id: LayerId) -> Option<&mut Vec<RandomizedSeed>> {
    match &mut project.layers.get_mut(&id)?.kind {
        LayerKind::Group { randomized, .. } => Some(randomized),
        _ => None,
    }
}

/// Whether folder `id`'s Contents differ from a new folder's: a Scale or Rotation laid on its fills, or seeds its Randomize seeds changed
pub fn contents_changed(project: &Project, id: LayerId) -> bool {
    match project.layers.get(&id).map(|layer| &layer.kind) {
        Some(LayerKind::Group { contents, randomized, .. }) => !contents.is_none() || !randomized.is_empty(),
        _ => false,
    }
}

fn set_contents(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "folder.contents.set";
    let id = folder_param(s, CMD, p)?;
    let (scale, rotation) = (f32p(p, "scale"), f32p(p, "rotation"));
    if scale.is_none() && rotation.is_none() {
        return Err(bad(CMD, "give `scale` and/or `rotation`"));
    }

    let current = s.doc.project.contents_offset(id);
    let offset = ContentsOffset::new(scale.unwrap_or(current.scale), rotation.unwrap_or(current.rotation));
    let reply = json!({ "scale": offset.scale, "rotation": offset.rotation });
    if offset == current {
        return Ok(reply);
    }

    s.edit("Folder contents", |s| {
        let contents = contents_mut(&mut s.doc.project, id).ok_or_else(|| bad(CMD, format!("layer {} is not a folder", id.0)))?;
        *contents = offset;
        Ok(())
    })?;
    Ok(reply)
}

fn reset_contents(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "folder.contents.reset";
    let id = folder_param(s, CMD, p)?;
    if !contents_changed(&s.doc.project, id) {
        return Ok(Value::Null);
    }

    let mut inside = Vec::new();
    descendants(&s.doc.project, id, 0, &mut inside);
    let inside: HashSet<LayerId> = inside.into_iter().collect();

    s.edit("Reset folder contents", |s| {
        let contents = contents_mut(&mut s.doc.project, id).ok_or_else(|| bad(CMD, format!("layer {} is not a folder", id.0)))?;
        *contents = ContentsOffset::NONE;

        let randomized = randomized_mut(&mut s.doc.project, id).map(std::mem::take).unwrap_or_default();
        let mut records_by_layer = HashMap::<LayerId, Vec<&RandomizedSeed>>::new();
        for record in randomized.iter().filter(|record| inside.contains(&record.layer)) {
            records_by_layer.entry(record.layer).or_default().push(record);
        }

        for (layer_id, records) in records_by_layer {
            let Some(layer) = s.doc.project.layers.get_mut(&layer_id) else { continue };
            let mut seeds = layer_seeds(layer);
            for record in records {
                // Preserve seeds edited since randomization
                if let Some(seed) = seeds.get_mut(record.index as usize).filter(|seed| ***seed == record.after) {
                    **seed = record.before;
                }
            }
        }

        Ok(())
    })?;
    Ok(Value::Null)
}

fn randomize_seeds(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "folder.randomizeSeeds";
    let folder = folder_param(s, CMD, p)?;
    let mut inside = Vec::new();
    descendants(&s.doc.project, folder, 0, &mut inside);
    let seeds: usize = inside.iter().filter_map(|id| s.doc.project.layers.get(id)).map(seed_count).sum();
    if seeds == 0 {
        return Ok(json!({ "seeds": 0 }));
    }

    // One mixing key per folder: a seed maps to the same new seed wherever it is, so patterns that lined up still do
    let key = next_seed(folder.0 as u32 ^ (folder.0 >> 32) as u32);
    s.edit("Randomize seeds", |s| {
        let mut randomized = randomized_mut(&mut s.doc.project, folder).map(std::mem::take).unwrap_or_default();
        let mut positions = randomized.iter().enumerate().map(|(position, record)| ((record.layer, record.index), position)).collect();
        for id in &inside {
            let Some(layer) = s.doc.project.layers.get_mut(id) else { continue };
            for (index, seed) in (0u32..).zip(layer_seeds(layer)) {
                let after = next_seed(*seed ^ key);
                remember(&mut randomized, &mut positions, RandomizedSeed { layer: *id, index, before: *seed, after });
                *seed = after;
            }
        }
        *randomized_mut(&mut s.doc.project, folder).ok_or_else(|| bad(CMD, format!("layer {} is not a folder", folder.0)))? = randomized;
        Ok(())
    })?;
    Ok(json!({ "seeds": seeds }))
}

/// Adds `change` to the folder's randomized seeds, found by (layer, seed index) in `positions`; a seed randomized again keeps its first `before` (what the reset puts back), unless it was changed by hand in between
fn remember(randomized: &mut Vec<RandomizedSeed>, positions: &mut HashMap<(LayerId, u32), usize>, change: RandomizedSeed) {
    match positions.get(&(change.layer, change.index)).and_then(|position| randomized.get_mut(*position)) {
        Some(record) if record.after == change.before => record.after = change.after,
        Some(record) => *record = change,
        None => {
            positions.insert((change.layer, change.index), randomized.len());
            randomized.push(change);
        }
    }
}

/// Whether folder `id` holds anything its Contents act on: a fill (layer, content or mask fill) or a seed
pub fn has_contents(project: &Project, id: LayerId) -> bool {
    let mut inside = Vec::new();
    descendants(project, id, 0, &mut inside);
    inside.iter().filter_map(|id| project.layers.get(id)).any(|layer| {
        let fills = matches!(layer.kind, LayerKind::Fill(_))
            || layer
                .content_effects
                .iter()
                .chain(layer.mask.iter().flat_map(|mask| mask.effects.iter()))
                .any(|effect| matches!(effect.kind, EffectKind::Fill { .. } | EffectKind::Material(_)));
        fills || seed_count(layer) > 0
    })
}

/// Every layer inside folder `id`, nested folders' children too (instances' sources are not followed: they live elsewhere)
fn descendants(project: &Project, id: LayerId, depth: usize, out: &mut Vec<LayerId>) {
    if depth > MAX_DEPTH {
        return;
    }

    let Some(layer) = project.layers.get(&id) else { return };
    for &child in layer.children() {
        out.push(child);
        descendants(project, child, depth + 1, out);
    }
}

/// The seeds of a layer's fill, content effects and mask effects
fn layer_seeds(layer: &mut Layer) -> Vec<&mut u32> {
    let Layer { kind, content_effects, mask, .. } = layer;
    let mut seeds = Vec::new();
    if let LayerKind::Fill(fill) = kind {
        seeds.extend(fill.sources.values_mut().filter_map(source_seed));
    }
    let effects = content_effects.iter_mut().chain(mask.iter_mut().flat_map(|mask| mask.effects.iter_mut()));
    for effect in effects {
        match &mut effect.kind {
            EffectKind::Fill { source, .. } => seeds.extend(source_seed(source)),
            EffectKind::Material(fill) => seeds.extend(fill.sources.values_mut().filter_map(source_seed)),
            EffectKind::Generator(generator) => seeds.push(&mut generator.seed),
            EffectKind::Filter(Filter::Warp { seed, .. } | Filter::SlopeBlur { seed, .. } | Filter::DirectionalWarp { seed, .. }) => seeds.push(seed),
            EffectKind::Filter(Filter::SourceWarp(warp)) => seeds.push(&mut warp.seed),
            EffectKind::Filter(Filter::SourceSlopeBlur(slope)) => seeds.push(&mut slope.seed),
            _ => {}
        }
    }
    seeds
}

/// How many seeds [`layer_seeds`] gives for `layer`, read without borrowing it mutably (the panel asks every frame)
fn seed_count(layer: &Layer) -> usize {
    let fill_seeds = |fill: &Fill| fill.sources.values().filter(|source| matches!(source, Source::Procedural { .. })).count();
    let own = match &layer.kind {
        LayerKind::Fill(fill) => fill_seeds(fill),
        _ => 0,
    };
    let effects = layer.content_effects.iter().chain(layer.mask.iter().flat_map(|mask| mask.effects.iter()));
    let effect_seeds: usize = effects
        .map(|effect| match &effect.kind {
            EffectKind::Fill { source, .. } => usize::from(matches!(source, Source::Procedural { .. })),
            EffectKind::Material(fill) => fill_seeds(fill),
            EffectKind::Generator(_)
            | EffectKind::Filter(
                Filter::Warp { .. } | Filter::SlopeBlur { .. } | Filter::DirectionalWarp { .. } | Filter::SourceWarp(_) | Filter::SourceSlopeBlur(_),
            ) => 1,
            _ => 0,
        })
        .sum();
    own + effect_seeds
}

fn source_seed(source: &mut Source) -> Option<&mut u32> {
    match source {
        Source::Procedural { pattern, .. } => Some(&mut pattern.seed),
        _ => None,
    }
}

/// A new seed that looks random but follows from `seed` (SplitMix32's mixing step), so tests and replays get the same seeds
pub fn next_seed(seed: u32) -> u32 {
    let mut z = seed.wrapping_add(0x9e37_79b9);
    z = (z ^ (z >> 16)).wrapping_mul(0x85eb_ca6b);
    z = (z ^ (z >> 13)).wrapping_mul(0xc2b2_ae35);
    z ^ (z >> 16)
}
