//! A fill's whole material: set every channel of a fill layer or content fill effect from one built-in material at once

use std::collections::BTreeMap;

use serde_json::Value;
use texturecraft_core::{Channel, EffectKind, Fill, LayerId, LayerKind, Project, Source};

use crate::commands::{CommandSpec, bad, has_layer, strp, u64p};
use crate::fill_material_choice::FillMaterialChoice;
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
        "{layer? | effect?: content fill effect id, material?: built-in material id (not a smart material) | null (back to the channels' defaults), anchor?: anchor effect id} — sets every channel the material writes (or links every enabled channel to `anchor`), turns those channels on and the others off; the projection stays; linked fills are rejected",
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
    let choice = choice_param(params)?;
    let set = target_set(session, target)?;
    if holds(session, target, set, choice)? {
        return Ok(Value::Null);
    }

    match choice {
        FillMaterialChoice::Clear => {
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
        }
        FillMaterialChoice::Material(found) => {
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
        }
        FillMaterialChoice::Anchor(anchor) => {
            let reader = match target {
                FillTarget::Layer(id) => id,
                FillTarget::Effect(effect) | FillTarget::Mask(effect) => {
                    session.doc.project.effect(effect).map(|(owner, _)| owner).ok_or_else(|| bad(CMD, format!("no effect {}", effect.0)))?
                }
            };
            let channels = active_channels(session, target, set)?;
            let mut linked = BTreeMap::new();
            for channel in channels {
                linked.insert(channel, crate::mask_cmds::anchor_source(session, CMD, reader, anchor, channel)?);
            }
            let anchor_name = session.doc.project.effect(anchor).map(|(_, effect)| effect.name.clone()).unwrap_or_default();
            session.edit(&format!("Link fill to {anchor_name}"), |session| {
                let edited = fill_mut(session, target)?;
                edited.material = None;
                edited.sources.extend(linked);
                Ok(())
            })?;
        }
    }
    Ok(Value::Null)
}

/// The `material` or `anchor` param: one built-in material, one anchor point, or `Clear` for null
fn choice_param(params: &Value) -> Result<FillMaterialChoice> {
    if params.get("material").is_some() && params.get("anchor").is_some() {
        return Err(bad(CMD, "give `material` or `anchor`, not both"));
    }

    if let Some(material) = params.get("material") {
        if material.is_null() {
            return Ok(FillMaterialChoice::Clear);
        }

        let id = strp(params, "material").ok_or_else(|| bad(CMD, "`material` is a material id or null"))?;
        let found = find(id).ok_or_else(|| bad(CMD, format!("unknown material `{id}`")))?;
        if found.smart {
            return Err(bad(CMD, format!("`{}` is a smart material (a folder of layers): add it from the shelf with material.apply", found.id)));
        }

        return Ok(FillMaterialChoice::Material(found));
    }

    if params.get("anchor").is_some() {
        let id = u64p(params, "anchor").ok_or_else(|| bad(CMD, "`anchor` is an anchor effect id"))?;
        return Ok(FillMaterialChoice::Anchor(LayerId(id)));
    }

    Err(bad(CMD, "`material` or `anchor` is required"))
}

/// Channels active on `target` when linking an anchor in Material mode
fn active_channels(session: &Session, target: FillTarget, set: usize) -> Result<Vec<Channel>> {
    match target {
        FillTarget::Layer(id) => Ok(session.doc.project.layer(id)?.channels.keys().copied().collect()),
        FillTarget::Effect(id) => {
            let Some((_, effect)) = session.doc.project.effect(id) else {
                return Err(bad(CMD, format!("no effect {}", id.0)));
            };
            let EffectKind::Material(fill) = &effect.kind else {
                return Err(bad(CMD, "not a content fill effect"));
            };
            if fill.sources.is_empty() { Ok(session.doc.project.texture_set(set)?.channel_list()) } else { Ok(fill.sources.keys().copied().collect()) }
        }
        FillTarget::Mask(_) => Err(bad(CMD, "a mask fill has one grayscale source and no channels or material")),
    }
}

/// Whether the fill at `target` (in texture set `set`) already holds `choice` whole, so setting it changes nothing
fn holds(session: &Session, target: FillTarget, set: usize, choice: FillMaterialChoice) -> Result<bool> {
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

    Ok(match choice {
        FillMaterialChoice::Clear => {
            fill.material.is_none()
                && match layer_channels {
                    Some(_) => fill.sources.is_empty(),
                    None => fill.sources.iter().all(|(channel, source)| *source == Source::uniform_of(*channel)),
                }
        }
        FillMaterialChoice::Material(found) => {
            fill.material.as_deref() == Some(found.id)
                && match layer_channels {
                    Some(channels) => channels.len() == fill.sources.len() && fill.sources.keys().all(|channel| channels.contains_key(channel)),
                    None => {
                        let channels = session.doc.project.texture_set(set)?.channel_list();

                        // The recipe allocates a layer ID, but comparing its sources must leave the project alone
                        let mut scratch = Project::new("", "");
                        fill.sources == material_sources(&mut scratch, &channels, found)?
                    }
                }
        }
        FillMaterialChoice::Anchor(anchor) => {
            let channels = active_channels(session, target, set)?;
            fill.material.is_none()
                && !channels.is_empty()
                && channels.into_iter().all(|channel| fill.sources.get(&channel) == Some(&Source::Anchor { anchor, channel, ramp: Vec::new() }))
        }
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
