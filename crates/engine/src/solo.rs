//! Solo: a view-only preview of one layer on its own. The compositor draws a copy of the project
//! in which the other layers of the soloed layer's texture set are hidden (see
//! [`texturecraft_compose::solo_project`]); the document, the undo history, saving, exporting and
//! baking never see it.

use serde_json::{Value, json};
use texturecraft_core::{LayerId, Project};

use crate::commands::{CommandSpec, always, bad, boolp, layer_param};
use crate::{Result, Session, cmd};

/// The layer shown alone and whether its mask is ignored (full coverage).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Solo {
    pub layer: LayerId,
    pub ignore_mask: bool,
}

pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "view.solo",
        "Solo layer",
        [],
        None,
        "{layer?: id | null (default: the selected layer; null ends the solo), ignoreMask?: bool (show the layer at full coverage, default false)} → {layer: id | null, ignoreMask}. View only: nothing is saved, exported, baked or put in the undo history",
        always,
        solo
    )]
}

fn solo(s: &mut Session, p: &Value) -> Result<Value> {
    let next = match p.get("layer") {
        Some(Value::Null) => None,
        _ => {
            let layer = layer_param(s, "view.solo", p)?;
            if s.doc.project.locate(layer).is_none() {
                return Err(bad("view.solo", "the layer is not in a stack"));
            }

            Some(Solo { layer, ignore_mask: boolp(p, "ignoreMask").unwrap_or(false) })
        }
    };
    s.set_solo(next);
    Ok(json!({ "layer": s.solo.map(|solo| solo.layer.0), "ignoreMask": s.solo.is_some_and(|solo| solo.ignore_mask) }))
}

impl Session {
    /// Show `solo` alone (None: the whole stack again). Changes only the view: every covered
    /// tile of every set is composited again, and the next frame draws it.
    pub fn set_solo(&mut self, solo: Option<Solo>) {
        if self.solo == solo {
            return;
        }

        self.solo = solo;
        for cache in &mut self.caches {
            // What the cache holds was composited from the other stack
            cache.below = Default::default();
            cache.invalidate_all();
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// The project as the compositor draws it: the soloed layer's, or None for the document's own.
    pub(crate) fn solo_view(&self) -> Option<Project> {
        let solo = self.solo?;
        texturecraft_compose::solo_project(&self.doc.project, solo.layer, solo.ignore_mask)
    }

    /// Sample document pixels while solo keeps its own composite in the viewport cache.
    pub(crate) fn document_composite(&self, ts: usize, tiles: &[(u32, u32)]) -> Option<std::borrow::Cow<'_, texturecraft_compose::Composite>> {
        use std::borrow::Cow;
        use texturecraft_compose::{Composite, composite_tiles, pad_tiles};

        let cache = self.caches.get(ts)?;
        if self.solo.is_none() {
            return Some(Cow::Borrowed(&cache.composite));
        }

        self.with_inputs(ts, |inputs| {
            let set = inputs.project.texture_sets.get(ts)?;
            let mut output = Composite { block_scales: inputs.block_scales.clone(), ..Composite::with_size(&set.channel_list(), cache.size.0, cache.size.1) };
            composite_tiles(inputs, &mut output, tiles);
            pad_tiles(&mut output, &cache.dilation, tiles);
            Some(Cow::Owned(output))
        })
    }
}
