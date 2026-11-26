//! A folder's actions on everything inside it: the Scale and Rotation laid on its fills, and new seeds for its patterns

use std::collections::HashMap;

use serde_json::{Value, json};
use texturecraft_core::project::MAX_DEPTH;
use texturecraft_core::{Channel, ContentsOffset, EffectKind, Fill, Filter, Layer, LayerId, LayerKind, Project, RandomizedSeed, Source};

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

    s.edit("Reset folder contents", |s| {
        let contents = contents_mut(&mut s.doc.project, id).ok_or_else(|| bad(CMD, format!("layer {} is not a folder", id.0)))?;
        *contents = ContentsOffset::NONE;

        let records = take_seed_records(&mut s.doc.project, id);
        for id in inside {
            let Some(layer) = s.doc.project.layers.get_mut(&id) else { continue };
            let Some(records) = records.get(&id) else { continue };
            let seeds = layer_seeds(layer);
            let originals = original_seeds(&seeds, records);

            for (target, seed) in seeds {
                if let Some(record) = originals.get(&target).filter(|record| record.after == *seed) {
                    *seed = record.before;
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
        let records = take_seed_records(&mut s.doc.project, folder);
        let mut randomized = Vec::with_capacity(seeds);
        for id in &inside {
            let Some(layer) = s.doc.project.layers.get_mut(id) else { continue };
            let seeds = layer_seeds(layer);
            let originals = records.get(id).map(|records| original_seeds(&seeds, records)).unwrap_or_default();

            for (index, (target, seed)) in (0u32..).zip(seeds) {
                let before = originals.get(&target).filter(|record| record.after == *seed).map_or(*seed, |record| record.before);
                let after = next_seed(*seed ^ key);
                randomized.push(RandomizedSeed { layer: *id, target: Some((target.0, target.1)), input: target.2, index, before, after });
                *seed = after;
            }
        }

        *randomized_mut(&mut s.doc.project, folder).ok_or_else(|| bad(CMD, format!("layer {} is not a folder", folder.0)))? = randomized;
        Ok(())
    })?;
    Ok(json!({ "seeds": seeds }))
}

type SeedTarget = (LayerId, Option<Channel>, Option<String>);

fn take_seed_records(project: &mut Project, folder: LayerId) -> HashMap<LayerId, Vec<RandomizedSeed>> {
    let mut by_layer = HashMap::<_, Vec<_>>::new();
    for record in randomized_mut(project, folder).into_iter().flat_map(std::mem::take) {
        by_layer.entry(record.layer).or_default().push(record);
    }

    by_layer
}

fn original_seeds<'a>(seeds: &[(SeedTarget, &mut u32)], records: &'a [RandomizedSeed]) -> HashMap<SeedTarget, &'a RandomizedSeed> {
    records
        .iter()
        .filter_map(|record| {
            // Old records identified a seed by its position in the layer
            let target = record
                .target
                .map(|(id, channel)| (id, channel, record.input.clone()))
                .or_else(|| seeds.get(record.index as usize).map(|(target, _)| target.clone()))?;
            Some((target, record))
        })
        .collect()
}

/// Whether the folder has a fill to transform, seeds to randomize or changes to reset.
pub fn has_contents(project: &Project, id: LayerId) -> bool {
    fn applicable(project: &Project, id: LayerId, depth: usize, owned: bool) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }

        let Some(layer) = project.layers.get(&id) else { return false };
        let source = project.content_layer(id);
        let mask = project.layers.get(&project.mask_owner(id)).and_then(|layer| layer.mask.as_ref());
        let effects = source.into_iter().flat_map(|layer| &layer.content_effects).chain(mask.into_iter().flat_map(|mask| &mask.effects));
        let fill = source.is_some_and(|layer| matches!(layer.kind, LayerKind::Fill(_)))
            || effects.into_iter().any(|effect| matches!(effect.kind, EffectKind::Fill { .. } | EffectKind::Material(_)));
        fill || (owned && seed_count(layer) > 0)
            || source.is_some_and(|source| source.children().iter().any(|child| applicable(project, *child, depth + 1, owned && source.id == id)))
    }

    contents_changed(project, id) || project.layers.get(&id).is_some_and(|layer| layer.children().iter().any(|child| applicable(project, *child, 0, true)))
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
fn layer_seeds(layer: &mut Layer) -> Vec<(SeedTarget, &mut u32)> {
    let Layer { id, kind, content_effects, mask, .. } = layer;
    let mut seeds = Vec::new();
    if let LayerKind::Fill(fill) = kind {
        seeds.extend(fill.sources.iter_mut().filter_map(|(&channel, source)| Some(((*id, Some(channel), None), source_seed(source)?))));
    }

    let mut inputs = Vec::new();
    let effects = content_effects.iter_mut().chain(mask.iter_mut().flat_map(|mask| mask.effects.iter_mut()));
    for effect in effects {
        if matches!(effect.kind, EffectKind::Filter(_)) {
            inputs.extend(effect.inputs.iter_mut().filter_map(|(name, source)| {
                let seed = source_seed(source)?;
                Some(((effect.id, None, Some(name.clone())), seed))
            }));
        }

        match &mut effect.kind {
            EffectKind::Fill { source, .. } => seeds.extend(source_seed(source).map(|seed| ((effect.id, None, None), seed))),
            EffectKind::Material(fill) => {
                seeds.extend(fill.sources.iter_mut().filter_map(|(&channel, source)| Some(((effect.id, Some(channel), None), source_seed(source)?))))
            }
            EffectKind::Generator(generator) => seeds.push(((effect.id, None, None), &mut generator.seed)),
            EffectKind::Filter(Filter::Warp { seed, .. } | Filter::SlopeBlur { seed, .. } | Filter::DirectionalWarp { seed, .. }) => {
                seeds.push(((effect.id, None, None), seed))
            }
            EffectKind::Filter(Filter::SourceWarp(warp)) => seeds.push(((effect.id, None, None), &mut warp.seed)),
            EffectKind::Filter(Filter::SourceSlopeBlur(slope)) => seeds.push(((effect.id, None, None), &mut slope.seed)),
            _ => {}
        }
    }

    // Legacy positional records count only the layer and effect seeds
    seeds.extend(inputs);
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
        .map(|effect| {
            let inputs = if matches!(effect.kind, EffectKind::Filter(_)) {
                effect.inputs.values().filter(|source| matches!(source, Source::Procedural { .. })).count()
            } else {
                0
            };

            inputs
                + match &effect.kind {
                    EffectKind::Fill { source, .. } => usize::from(matches!(source, Source::Procedural { .. })),
                    EffectKind::Material(fill) => fill_seeds(fill),
                    EffectKind::Generator(_)
                    | EffectKind::Filter(
                        Filter::Warp { .. } | Filter::SlopeBlur { .. } | Filter::DirectionalWarp { .. } | Filter::SourceWarp(_) | Filter::SourceSlopeBlur(_),
                    ) => 1,
                    _ => 0,
                }
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
