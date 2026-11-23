//! An effect's blending per channel: the Blending options of an effect row (one channel's blending on every channel, copy and paste)

use std::collections::BTreeMap;

use serde_json::{Value, json};
use texturecraft_core::{BlendMode, Channel, ChannelBlend, Effect, LayerId, Project};

use crate::commands::{CommandSpec, bad, channel_param, has_layer};
use crate::mask_cmds::effect_id;
use crate::session::StackTarget;
use crate::{Result, Session, cmd};

/// The undo step of every Blending options command
const UNDO_LABEL: &str = "Blending options";

/// What a copy of channel blending holds: the blending shared by the channels, and the channels that blend apart
pub type CopiedBlending = (ChannelBlend, BTreeMap<Channel, ChannelBlend>);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "effect.blending.modeToEveryChannel",
            "Use this blend mode on every channel",
            [],
            None,
            "{effect?: the selected content effect, channel?: the channel whose blend mode every channel takes (default baseColor)}",
            content_effect_selected,
            mode_to_every_channel
        ),
        cmd!(
            "effect.blending.opacityToEveryChannel",
            "Use this opacity on every channel",
            [],
            None,
            "{effect?: the selected content effect, channel?: the channel whose opacity every channel takes (default baseColor)}",
            content_effect_selected,
            opacity_to_every_channel
        ),
        cmd!(
            "effect.blending.bothToEveryChannel",
            "Use both on every channel",
            [],
            None,
            "{effect?: the selected content effect, channel?: the channel whose blend mode and opacity every channel takes (default baseColor)}",
            content_effect_selected,
            both_to_every_channel
        ),
        cmd!(
            "effect.blending.copy",
            "Copy channel blending",
            [],
            None,
            "{effect?: the selected effect} — every channel's blend mode and opacity",
            blending_effect_selected,
            copy
        ),
        cmd!(
            "effect.blending.paste",
            "Paste channel blending",
            [],
            None,
            "{effect?: the selected effect} — the copied blend modes and opacities (a mask effect takes the shared ones)",
            can_paste,
            paste
        ),
    ]
}

/// Sets `effect`'s blend `mode` and `opacity` where given: on `channel` alone, or on every channel (`None`, a mask effect's only one); channels left blending like the others stop blending apart
pub fn set_blending(effect: &mut Effect, channel: Option<Channel>, mode: Option<BlendMode>, opacity: Option<f32>) {
    let edit = |blend: &mut ChannelBlend| {
        if let Some(mode) = mode {
            blend.mode = mode;
        }
        if let Some(opacity) = opacity {
            blend.opacity = opacity.clamp(0.0, 1.0);
        }
    };
    match channel {
        Some(channel) => {
            let mut blend = effect.blend_of(Some(channel));
            edit(&mut blend);
            effect.channel_blends.insert(channel, blend);
        }
        None => {
            let mut shared = effect.blend_of(None);
            edit(&mut shared);
            effect.blend = shared.mode;
            effect.opacity = shared.opacity;
            effect.channel_blends.values_mut().for_each(edit);
        }
    }
    let shared = effect.blend_of(None);
    effect.channel_blends.retain(|_, blend| *blend != shared);
}

/// Whether effect `id` is in a layer's mask (it then blends one channel)
pub fn on_mask(project: &Project, id: LayerId) -> bool {
    project.layers.values().any(|layer| layer.mask.as_ref().is_some_and(|mask| mask.effects.iter().any(|effect| effect.id == id)))
}

/// Whether the selected effect has blending (and with `content_only`, is a content effect); with no effect selected, a command names its effect
fn selected_blending_effect(s: &Session, content_only: bool) -> std::result::Result<(), String> {
    let StackTarget::Effect(id) = s.target else { return has_layer(s) };
    let Some((_, effect)) = s.doc.project.effect(id) else { return has_layer(s) };
    blending_refusal(&s.doc.project, id, effect, content_only)
}

/// Why effect `id` (`effect`) can't take a Blending options command: it has no blending or, with `content_only`, it is a mask effect
fn blending_refusal(project: &Project, id: LayerId, effect: &Effect, content_only: bool) -> std::result::Result<(), String> {
    if !effect.kind.blends() {
        return Err(format!("{} has no blending", effect.kind.name().to_ascii_lowercase()));
    }

    if content_only && on_mask(project, id) {
        return Err("a mask effect blends one channel".into());
    }

    Ok(())
}

fn content_effect_selected(s: &Session) -> std::result::Result<(), String> {
    selected_blending_effect(s, true)
}

fn blending_effect_selected(s: &Session) -> std::result::Result<(), String> {
    selected_blending_effect(s, false)
}

fn can_paste(s: &Session) -> std::result::Result<(), String> {
    if s.blending_clipboard.is_none() {
        return Err("no channel blending is copied".into());
    }

    blending_effect_selected(s)
}

/// Effect `effect` (or the selected one) of command `cmd`, checked to have blending and, with `content_only`, to be a content effect
fn blending_effect(s: &Session, p: &Value, cmd: &str, content_only: bool) -> Result<LayerId> {
    let id = effect_id(s, p, cmd)?;
    let (_, effect) = s.doc.project.effect(id).ok_or_else(|| bad(cmd, format!("no effect {}", id.0)))?;
    blending_refusal(&s.doc.project, id, effect, content_only).map_err(|refusal| bad(cmd, refusal))?;
    Ok(id)
}

/// Every channel of the content effect in `p` takes what `copied` picks (a blend mode, an opacity or both) from the `channel` in `p`
fn to_every_channel(s: &mut Session, p: &Value, cmd: &str, copied: fn(ChannelBlend) -> (Option<BlendMode>, Option<f32>)) -> Result<Value> {
    let id = blending_effect(s, p, cmd, true)?;
    let channel = channel_param(cmd, p, "channel")?.unwrap_or(Channel::BaseColor);
    let (_, current) = s.doc.project.effect(id).ok_or_else(|| bad(cmd, format!("no effect {}", id.0)))?;
    let mut changed = current.clone();
    let (mode, opacity) = copied(changed.blend_of(Some(channel)));
    set_blending(&mut changed, None, mode, opacity);
    replace_blending(s, id, cmd, changed)
}

/// Effect `id` replaced by `changed`, in one undo step; no step when nothing changes
fn replace_blending(s: &mut Session, id: LayerId, cmd: &str, changed: Effect) -> Result<Value> {
    let (_, current) = s.doc.project.effect(id).ok_or_else(|| bad(cmd, format!("no effect {}", id.0)))?;
    if *current == changed {
        return Ok(Value::Null);
    }

    s.edit(UNDO_LABEL, |s| {
        let effect = s.doc.project.effect_mut(id).ok_or_else(|| bad(cmd, format!("no effect {}", id.0)))?;
        *effect = changed;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn mode_to_every_channel(s: &mut Session, p: &Value) -> Result<Value> {
    to_every_channel(s, p, "effect.blending.modeToEveryChannel", |source| (Some(source.mode), None))
}

fn opacity_to_every_channel(s: &mut Session, p: &Value) -> Result<Value> {
    to_every_channel(s, p, "effect.blending.opacityToEveryChannel", |source| (None, Some(source.opacity)))
}

fn both_to_every_channel(s: &mut Session, p: &Value) -> Result<Value> {
    to_every_channel(s, p, "effect.blending.bothToEveryChannel", |source| (Some(source.mode), Some(source.opacity)))
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.blending.copy";
    let id = blending_effect(s, p, CMD, false)?;
    let (_, effect) = s.doc.project.effect(id).ok_or_else(|| bad(CMD, format!("no effect {}", id.0)))?;
    s.blending_clipboard = Some((effect.blend_of(None), effect.channel_blends.clone()));
    Ok(json!({ "copied": id.0 }))
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.blending.paste";
    let (shared, channels) = s.blending_clipboard.clone().ok_or_else(|| bad(CMD, "no channel blending is copied"))?;
    let id = blending_effect(s, p, CMD, false)?;
    let mask = on_mask(&s.doc.project, id);
    let (_, current) = s.doc.project.effect(id).ok_or_else(|| bad(CMD, format!("no effect {}", id.0)))?;
    let mut changed = current.clone();
    changed.blend = shared.mode;
    changed.opacity = shared.opacity;
    changed.channel_blends = if mask { BTreeMap::new() } else { channels };
    replace_blending(s, id, CMD, changed)
}
