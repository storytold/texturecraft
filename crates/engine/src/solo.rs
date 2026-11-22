//! Solo: a view-only preview of one layer on its own. The compositor draws a copy of the project
//! in which the other layers of the soloed layer's texture set are hidden (see
//! [`texturecraft_compose::solo_project`]); the document, the undo history, saving, exporting and
//! baking never see it.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde_json::{Value, json};
use texturecraft_compose::{Composite, composite_tiles};
use texturecraft_core::{Channel, LayerId, Project};
use texturecraft_geom::NO_TRI;
use texturecraft_texel::TILE;

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

    /// Sample the document even when the viewport has a soloed layer or owns the result on GPU.
    pub(crate) fn document_composite(&self, ts: usize, tiles: &[(u32, u32)]) -> Option<Cow<'_, Composite>> {
        self.read_composite(ts, tiles, true)
    }

    /// Sample a surface hit without building an atlas-wide padding map for a covered texel.
    pub(crate) fn document_sample(&self, ts: usize, x: u32, y: u32) -> Option<BTreeMap<Channel, [f32; 3]>> {
        let cache = self.caches.get(ts)?;
        let needs_padding = cache.gbuf.tri.get((y * cache.size.0 + x) as usize) == Some(&NO_TRI);
        let (x, y) = if needs_padding { cache.dilation.source_for_sample(x, y).unwrap_or((x, y)) } else { (x, y) };
        let composite = self.read_composite(ts, &[(x / TILE, y / TILE)], false)?;
        Some(composite.channels.keys().map(|&channel| (channel, composite.value(channel, x, y))).collect())
    }

    fn read_composite(&self, ts: usize, tiles: &[(u32, u32)], pad_output: bool) -> Option<Cow<'_, Composite>> {
        let cache = self.caches.get(ts)?;
        let document_in_solo = self.solo.is_some();
        if !document_in_solo && cache.dirty.is_empty() && cache.settle.is_empty() {
            return Some(Cow::Borrowed(&cache.composite));
        }
        let pending = if document_in_solo {
            Cow::Borrowed(tiles)
        } else {
            Cow::Owned(tiles.iter().copied().filter(|tile| cache.dirty.contains(tile) || cache.settle.contains(tile)).collect())
        };
        if pending.is_empty() && !document_in_solo {
            return Some(Cow::Borrowed(&cache.composite));
        }

        self.with_inputs(ts, |inputs| {
            let mut output = if document_in_solo {
                let set = inputs.project.texture_sets.get(ts)?;
                Composite { block_scales: inputs.block_scales.clone(), ..Composite::with_size(&set.channel_list(), cache.size.0, cache.size.1) }
            } else {
                cache.composite.clone()
            };
            composite_tiles(inputs, &mut output, &pending);
            if pad_output {
                cache.dilation.apply_to_tiles(&mut output, &pending);
            }
            Some(Cow::Owned(output))
        })
    }
}
