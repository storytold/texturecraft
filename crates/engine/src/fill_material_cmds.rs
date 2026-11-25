//! A fill's whole material: set every channel of a fill layer or content fill effect from one built-in material at once

use std::collections::BTreeMap;

use serde_json::Value;
use texturecraft_core::{Channel, EffectKind, Fill, LayerKind, Project, Source};

use crate::commands::{CommandSpec, bad, has_layer, strp};
use crate::layer_cmds::{FillTarget, fill_mut, fill_target};
use crate::materials::{Builder, MATERIALS, SmartMaterial, build, find};
use crate::{EngineError, Result, Session, cmd};

/// The command [`set_material`] runs as
const CMD: &str = "layer.fill.material";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "layer.fill.material",
        "Set fill material",
        [],
        None,
        "{layer? | effect?: content fill effect id, material: built-in material id (not a smart material) | null (back to the channels' defaults)} — sets every channel the material writes, turns those channels on and the others off; the projection stays; linked fills are rejected",
        has_layer,
        set_material
    )]
}

/// The built-in materials a fill can hold whole: the plain ones (smart materials are folders of layers)
pub fn whole_materials() -> impl Iterator<Item = &'static SmartMaterial> {
    MATERIALS.iter().filter(|material| !material.smart)
}

fn set_material(session: &mut Session, params: &Value) -> Result<Value> {
    let target = fill_target(session, params, CMD)?;
    let material = material_param(params)?;
    let set = target_set(session, target)?;
    if holds(session, target, set, material)? {
        return Ok(Value::Null);
    }

    let Some(found) = material else {
        session.edit("Clear fill material", |session| {
            let edited = fill_mut(session, target)?;
            edited.material = None;
            match target {
                // A fill effect changes a channel while it has a source: its channels stay on, at their defaults
                FillTarget::Effect(_) => edited.sources.iter_mut().for_each(|(channel, source)| *source = Source::uniform_of(*channel)),
                FillTarget::Layer(_) | FillTarget::Mask(_) => edited.sources.clear(),
            }
            Ok(())
        })?;
        return Ok(Value::Null);
    };

    session.edit(&format!("Set fill material {}", found.label), |session| {
        let channels = session.doc.project.texture_set(set)?.channel_list();
        let sources = material_sources(&mut session.doc.project, &channels, found)?;
        let edited = fill_mut(session, target)?;
        edited.sources = sources;
        edited.material = Some(found.id.to_string());

        let written: Vec<Channel> = edited.sources.keys().copied().collect();
        if let FillTarget::Layer(id) = target {
            let layer = session.doc.project.layer_mut(id)?;
            layer.channels.retain(|channel, _| written.contains(channel));
            for channel in written {
                layer.channels.entry(channel).or_default();
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// The `material` param: a plain built-in material, or `None` for null
fn material_param(params: &Value) -> Result<Option<&'static SmartMaterial>> {
    match params.get("material") {
        Some(Value::Null) => Ok(None),
        Some(_) => {
            let id = strp(params, "material").ok_or_else(|| bad(CMD, "`material` is a material id or null"))?;
            let found = find(id).ok_or_else(|| bad(CMD, format!("unknown material `{id}`")))?;
            if found.smart {
                return Err(bad(CMD, format!("`{}` is a smart material (a folder of layers): add it from the shelf with material.apply", found.id)));
            }

            Ok(Some(found))
        }
        None => Err(bad(CMD, "`material` is required: a material id or null")),
    }
}

/// Whether the fill at `target` (in texture set `set`) already holds `material` whole (or, for `None`, no material and its channels' defaults), so setting it changes nothing
fn holds(session: &Session, target: FillTarget, set: usize, material: Option<&SmartMaterial>) -> Result<bool> {
    let (fill, layer_channels) = match target {
        FillTarget::Layer(id) => {
            let layer = session.doc.project.layer(id)?;
            let LayerKind::Fill(fill) = &layer.kind else { return Err(bad(CMD, "not a fill layer")) };
            (fill, Some(&layer.channels))
        }
        FillTarget::Effect(id) => match session.doc.project.effect(id).map(|(_, found)| &found.kind) {
            Some(EffectKind::Material(fill)) => (fill, None),
            _ => return Err(bad(CMD, "not a content fill effect")),
        },
        FillTarget::Mask(_) => return Err(bad(CMD, "a mask fill has one grayscale source and no channels or material")),
    };
    if fill.link.is_some() {
        return Err(bad(CMD, "a linked material cannot be replaced with this command"));
    }

    if fill.material.as_deref() != material.map(|found| found.id) {
        return Ok(false);
    }

    // Turning a channel off keeps the material name, so compare every source on content effects
    Ok(match (material, layer_channels) {
        (Some(_), Some(channels)) => channels.len() == fill.sources.len() && fill.sources.keys().all(|channel| channels.contains_key(channel)),
        (Some(found), None) => {
            let channels = session.doc.project.texture_set(set)?.channel_list();

            // The recipe allocates a layer ID, but comparing its sources must leave the project alone
            let mut scratch = Project::new("", "");
            fill.sources == material_sources(&mut scratch, &channels, found)?
        }
        (None, Some(_)) => fill.sources.is_empty(),
        (None, None) => fill.sources.iter().all(|(channel, source)| *source == Source::uniform_of(*channel)),
    })
}

/// The texture set the fill `target` paints in, whichever set is selected
fn target_set(session: &Session, target: FillTarget) -> Result<usize> {
    let owner = match target {
        FillTarget::Layer(id) => id,
        FillTarget::Effect(id) | FillTarget::Mask(id) => {
            session.doc.project.effect(id).map(|(owner, _)| owner).ok_or_else(|| bad(CMD, format!("no effect {}", id.0)))?
        }
    };
    session.doc.project.locate(owner).map(|(set, ..)| set).ok_or_else(|| bad(CMD, format!("layer {} is in no texture set", owner.0)))
}

/// The material's sources for the enabled channels. The recipe allocates layer IDs in `project`.
fn material_sources(project: &mut Project, channels: &[Channel], material: &SmartMaterial) -> Result<BTreeMap<Channel, Source>> {
    let fill = whole_fill(project, material, channels)?;
    Ok(fill.sources.into_iter().filter(|(channel, _)| channels.contains(channel)).collect())
}

/// The one unmasked fill a plain material is made of
fn whole_fill(project: &mut Project, material: &SmartMaterial, channels: &[Channel]) -> Result<Fill> {
    let layers = build(&mut Builder { p: project, channels: channels.to_vec() }, material.id)?;
    match <[_; 1]>::try_from(layers) {
        Ok([layer]) if layer.mask.is_none() => match layer.kind {
            LayerKind::Fill(fill) => Ok(fill),
            _ => Err(EngineError::Other(format!("material `{}` is not a fill", material.id))),
        },
        _ => Err(EngineError::Other(format!("material `{}` is more than one fill", material.id))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_whole_material_is_one_unmasked_fill() {
        let mut session = Session::new_project("t", texturecraft_geom::primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
        let channels = session.doc.project.texture_set(session.ts()).map(|set| set.channel_list()).unwrap_or_default();
        for material in whole_materials() {
            assert!(whole_fill(&mut session.doc.project, material, &channels).is_ok(), "{}", material.id);
        }
    }
}
