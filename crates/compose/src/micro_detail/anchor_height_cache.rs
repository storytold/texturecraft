use std::sync::{Arc, Mutex, PoisonError, Weak};

use texturecraft_core::{BakedMap, LayerId};

use super::{AnchorHeights, CACHE_BYTES, anchor_entry::AnchorEntry};
use crate::below::StackKey;
use crate::eval::Inputs;

/// Committed anchor heights reused by a session between composites
#[derive(Default)]
pub(crate) struct AnchorHeightCache {
    entries: Mutex<Vec<AnchorEntry>>,
}

impl AnchorHeightCache {
    pub(crate) fn retain_readers(&self, texture_set: usize, anchors: &[(LayerId, usize)]) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|entry| entry.baked.strong_count() > 0 && (entry.texture_set != texture_set || anchors.contains(&entry.anchor)));
    }

    pub(crate) fn get_or_build(
        &self,
        inputs: &Inputs,
        anchor: (LayerId, usize),
        stack: &Arc<StackKey>,
        build: impl FnOnce() -> AnchorHeights,
    ) -> Arc<AnchorHeights> {
        let baked = [BakedMap::Curvature, BakedMap::AmbientOcclusion].iter().find_map(|map| inputs.baked.maps.get(map)).map(Arc::downgrade);
        let Some(baked) = baked else { return Arc::new(build()) };
        let same_anchor = |entry: &AnchorEntry| entry.texture_set == inputs.ts && entry.anchor == anchor;
        let layout = (inputs.gbuf.width, inputs.gbuf.height);

        {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(index) = entries.iter().position(|entry| {
                same_anchor(entry) && Weak::ptr_eq(&entry.baked, &baked) && &entry.stack == stack && entry.heights.matches_layout(layout, &inputs.block_scales)
            }) {
                let entry = entries.remove(index);
                let heights = entry.heights.clone();
                entries.push(entry);
                return heights;
            }
        }

        // Height construction plans and evaluates the stack in parallel
        let heights = Arc::new(build());
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|entry| !same_anchor(entry));
        entries.push(AnchorEntry { baked, texture_set: inputs.ts, anchor, stack: stack.clone(), heights: heights.clone() });

        let mut bytes = 0usize;
        let kept = entries
            .iter()
            .rev()
            .take_while(|entry| {
                bytes = bytes.saturating_add(entry.heights.bytes());
                bytes <= CACHE_BYTES
            })
            .count();
        let excess = entries.len() - kept;
        entries.drain(..excess);
        heights
    }
}
