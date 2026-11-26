//! Layer locks (`texturecraft_core::Lock`): the `layer.unlock` command and the guard every edit passes.
//!
//! The guard compares the document before and after an edit ([`locked_change`]), so no command can change a locked
//! layer's content by accident, whichever path it takes. What stays allowed on a locked layer: visibility, rename,
//! color tag, move, delete, channel blending and opacity, the mask switch, and `layer.unlock`. Painting is refused
//! before a stroke starts ([`refuse_locked`]). Placeholder effects are read-only on every layer: an edit may hide,
//! move or remove one, never change what it stands for.

use serde_json::{Value, json};
use texturecraft_core::{Channel, Effect, EffectKind, Layer, LayerId, Project};

use crate::commands::{CommandSpec, always, bad, layer_param};
use crate::session::Document;
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "layer.unlock",
        "Unlock layer",
        [],
        None,
        "{layer?} — removes the lock of a layer whose content an import could not reproduce editably, so it can be edited (undoable)",
        always,
        unlock
    )]
}

fn unlock(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, "layer.unlock", p)?;
    let layer = s.doc.project.layer(id)?;
    let Some(lock) = layer.lock.clone() else {
        return Err(bad("layer.unlock", format!("layer {} (\"{}\") is not locked", id.0, layer.name)));
    };
    s.edit("Unlock layer", |s| {
        s.doc.project.layer_mut(id)?.lock = None;
        Ok(())
    })?;
    Ok(json!({ "layer": id.0, "unlocked": lock_json(&lock) }))
}

/// A lock as `project.inspect` shows it.
pub(crate) fn lock_json(lock: &texturecraft_core::Lock) -> Value {
    json!({
        "source": lock.source,
        "reasons": lock.reasons.iter().map(|r| json!({ "key": r.key, "text": r.text })).collect::<Vec<_>>(),
        "summary": lock.summary(),
    })
}

fn locked_error(layer: &Layer) -> EngineError {
    let why = layer.lock.as_ref().map(|l| l.summary()).unwrap_or_default();
    EngineError::Other(format!("layer {} (\"{}\") is locked, its content cannot change: {why}. Unlock it first (layer.unlock)", layer.id.0, layer.name))
}

/// Refuses painting into the content or mask of `owner` when it is locked.
pub(crate) fn refuse_locked(project: &Project, owner: LayerId) -> Result<()> {
    match project.layers.get(&owner) {
        Some(l) if l.is_locked() => Err(locked_error(l)),
        _ => Ok(()),
    }
}

/// The error for an edit from `before` to `after` that changed the content of a layer locked before it, or a
/// placeholder effect; None when the edit may stand.
pub(crate) fn locked_change(before: &Document, after: &Document) -> Option<EngineError> {
    for (id, old) in before.project.layers.iter().filter(|(_, l)| l.is_locked()) {
        // A deleted layer (or one in a deleted folder) is gone, not edited.
        let Some(new) = after.project.layers.get(id).filter(|_| after.project.locate(*id).is_some()) else { continue };
        if old.content_view() != new.content_view() || pixels_changed(before, after, old) {
            return Some(locked_error(old));
        }
    }
    for old in before.project.layers.values().flat_map(effects).filter(|e| matches!(e.kind, EffectKind::Placeholder { .. })) {
        if let Some((_, new)) = after.project.effect(old.id)
            && (new.kind != old.kind || new.name != old.name)
        {
            return Some(EngineError::Other(format!(
                "effect {} (\"{}\") is a placeholder for content TextureCraft cannot compute: it cannot change",
                old.id.0, old.name
            )));
        }
    }
    None
}

fn effects(l: &Layer) -> impl Iterator<Item = &Effect> {
    l.mask.iter().flat_map(|m| m.effects.iter()).chain(l.content_effects.iter())
}

/// Whether the pixels of `layer` (its own and its paint effects') differ between the documents. A texture set
/// resized between them resamples every layer: that is not an edit of the layer's content.
fn pixels_changed(before: &Document, after: &Document, layer: &Layer) -> bool {
    let size = |d: &Document| d.project.locate(layer.id).and_then(|(ts, _, _)| d.project.texture_sets.get(ts)).map(|t| t.size());
    if size(before) != size(after) {
        return false;
    }
    let ids: Vec<LayerId> = std::iter::once(layer.id).chain(effects(layer).map(|e| e.id)).collect();
    ids.iter().any(|id| {
        Channel::ALL.into_iter().any(|c| before.pixels.layer(*id, c) != after.pixels.layer(*id, c)) || before.pixels.masks.get(id) != after.pixels.masks.get(id)
    })
}
