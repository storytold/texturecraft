//! Copy and paste effects between layers, masks and texture sets. The clipboard is the layer
//! clipboard: it holds a layer or an effect.

use serde_json::{Value, json};
use texturecraft_core::{EffectKind, LayerId, Mask, MaskBackground};

use crate::commands::{CommandSpec, bad, has_layer, layer_param, strp};
use crate::layer_ops::{empty_texel, fit};
use crate::mask_cmds::{anchor_source, content_target, effect_id};
use crate::session::StackTarget;
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("effect.copy", "Copy effect", [], None, "{effect?: the selected effect}", has_layer, copy),
        cmd!("effect.cut", "Cut effect", [], None, "{effect?: the selected effect}", has_layer, cut),
        cmd!(
            "effect.paste",
            "Paste effect",
            [],
            None,
            "{layer?, target?: mask|content (default: the selection's stack)} → {effect} — above the selected effect, or on top of the stack",
            can_paste,
            paste
        ),
    ]
}

/// The copied effect, if the clipboard holds one.
pub fn clipboard_effect(s: &Session) -> Option<LayerId> {
    s.clipboard.as_ref().and_then(|(doc, id)| doc.project.effect(*id).map(|_| *id))
}

fn can_paste(s: &Session) -> std::result::Result<(), String> {
    match clipboard_effect(s) {
        Some(_) if s.selected_layer().is_some() => Ok(()),
        Some(_) => Err("select a layer to paste the effect on".into()),
        None => Err("the clipboard holds no effect".into()),
    }
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.copy";
    let eid = effect_id(s, p, CMD)?;
    if s.doc.project.effect(eid).is_none() {
        return Err(bad(CMD, format!("no effect {}", eid.0)));
    }
    s.clipboard = Some((s.doc.clone(), eid));
    Ok(json!({ "copied": eid.0 }))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    let eid = copy(s, p)?["copied"].clone();
    crate::execute(s, "effect.remove", &json!({ "effect": eid }))?;
    Ok(json!({ "cut": eid }))
}

fn paste(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "effect.paste";
    let (copied, copied_id) = session.clipboard.clone().ok_or_else(|| bad(CMD, "the clipboard holds no effect"))?;
    let effect = copied.project.effect(copied_id).map(|(_, effect)| effect.clone()).ok_or_else(|| bad(CMD, "the clipboard holds no effect"))?;
    let layer = layer_param(session, CMD, params)?;

    // The selected effect's stack, or the content or mask the selection targets
    let selected = match session.target {
        StackTarget::Effect(selected) if session.selected_layer() == Some(layer) => Some(selected),
        _ => None,
    };
    let in_content = |session: &Session, effect_id: LayerId| {
        let owner = session.doc.project.effect(effect_id).and_then(|(owner, _)| session.doc.project.layers.get(&owner));
        owner.is_some_and(|owner| owner.content_effects.iter().any(|effect| effect.id == effect_id))
    };
    let asked = strp(params, "target").is_some();
    let content = match selected {
        _ if asked => content_target(params, CMD)?,
        Some(selected) => in_content(session, selected),
        None => session.target == StackTarget::Content && session.selected_layer() == Some(layer),
    };

    // Effects that fit one stack only go there, unless another was asked for
    let content = match effect.kind.content_only() {
        Some(only) if only != content && asked => {
            return Err(bad(CMD, format!("{} effects go on the {}", effect.kind.name(), if only { "layer content" } else { "mask" })));
        }
        Some(only) => only,
        None => content,
    };

    // An instance's content and (unless it has its own) mask belong to its source
    let owner = if content { session.doc.project.content_layer(layer).map_or(layer, |source| source.id) } else { session.doc.project.mask_owner(layer) };
    let owner_layer = session.doc.project.layer(owner)?;
    if content && owner_layer.is_group() {
        return Err(EngineError::Other(format!("\"{}\" is a folder: content effects go on paint and fill layers", owner_layer.name)));
    }

    // Each anchor point the effect reads must be below its new layer
    for (anchor, channel) in effect.kind.anchors_read() {
        anchor_source(session, CMD, owner, anchor, channel)?;
    }
    let (set, _, _) = session.doc.project.locate(owner).ok_or_else(|| bad(CMD, "the layer is not in a stack"))?;
    let (size, channels) = session.doc.project.texture_set(set).map(|texture_set| (texture_set.size(), texture_set.channel_list()))?;

    let pasted = session.edit("Paste effect", |session| {
        let pasted = session.doc.project.alloc_id();
        let owner_layer = session.doc.project.layer_mut(owner)?;
        let stack = if content {
            &mut owner_layer.content_effects
        } else {
            let background =
                if matches!(effect.kind, EffectKind::Generator(_) | EffectKind::Fill { .. }) { MaskBackground::Black } else { MaskBackground::White };
            &mut owner_layer.mask.get_or_insert_with(|| Mask::new(background)).effects
        };
        let index = selected.and_then(|selected| stack.iter().position(|effect| effect.id == selected)).map_or(stack.len(), |position| position + 1);
        stack.insert(index, texturecraft_core::Effect { id: pasted, ..effect.clone() });

        // Painted pixels come along, resampled to the set's size (a mask's paint stays a mask's)
        if let Some(image) = copied.pixels.masks.get(&copied_id).filter(|_| !content) {
            session.doc.pixels.masks.insert(pasted, fit(image, size, &[0, 0]));
        }
        for channel in channels.iter().copied().filter(|_| content) {
            if let Some(image) = copied.pixels.layer(copied_id, channel) {
                session.doc.pixels.layers.insert((pasted, channel), fit(image, size, &empty_texel(channel)));
            }
        }

        // The target belongs to the active set's selection: a paste into another set leaves it
        if let Some((layer_set, _, _)) = session.doc.project.locate(layer) {
            session.selected.insert(layer_set, layer);
            if layer_set == session.ts() {
                session.target = StackTarget::Effect(pasted);
            }
        }
        Ok(pasted)
    })?;
    Ok(json!({ "effect": pasted.0 }))
}
