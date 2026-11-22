use texturecraft_core::{Channel, EffectKind, LayerId, Project};

use super::{Operation, channel_slot};

pub(super) const BEGIN_GROUP: u32 = 200;
pub(super) const END_GROUP: u32 = 201;
pub(super) const PASSTHROUGH_GROUP: u32 = u32::MAX;

pub(super) enum StackWork {
    Layer(LayerId, usize, u32),
    EndGroup(usize),
}

#[derive(Debug, Default)]
pub(super) struct GroupPlan {
    pub(super) depth: u32,
    pub(super) channels: u32,
    pub(super) words_per_level: u32,
    markers: u64,
}

impl GroupPlan {
    pub(super) fn finish(&mut self, operations: &[Operation], measured_blend: bool) {
        if self.depth == 0 {
            return;
        }
        for op in operations {
            if is_group(op) {
                self.markers += 1;
            } else if let Some(bit) = 1u32.checked_shl(op.ids[0]) {
                self.channels |= bit;
            }
        }
        // One word for the mask, then compact snapshots of the channels that can change.
        // Measured rules also preserve coverage for every saved channel.
        self.words_per_level = 1 + Channel::ALL
            .iter()
            .filter(|&&channel| self.channels & (1 << channel_slot(channel)) != 0)
            .map(|channel| channel.components() as u32 + u32::from(measured_blend))
            .sum::<u32>();
    }

    pub(super) fn words(&self, pixels: u64) -> u64 {
        pixels * u64::from(self.depth) * u64::from(self.words_per_level)
    }

    pub(super) fn work_per_pixel(&self) -> u64 {
        self.markers * u64::from(self.words_per_level)
    }
}

pub(super) fn is_group(op: &Operation) -> bool {
    matches!(op.ids[1], BEGIN_GROUP | END_GROUP)
}

/// Children still publish their anchors where the enclosing group does not draw.
pub(super) fn children_publish_anchors(project: &Project, children: &[LayerId], depth: usize) -> bool {
    if depth > texturecraft_core::project::MAX_DEPTH {
        return false;
    }
    children.iter().any(|&id| {
        if !project.layers.get(&id).is_some_and(|layer| layer.visible) {
            return false;
        }
        let Some(layer) = project.content_layer(id) else { return false };
        let mask = project.layers.get(&project.mask_owner(id)).and_then(|owner| owner.mask.as_ref()).filter(|mask| mask.enabled);
        layer.content_effects.iter().chain(mask.into_iter().flat_map(|mask| &mask.effects)).any(|eff| eff.visible && matches!(eff.kind, EffectKind::Anchor))
            || children_publish_anchors(project, layer.children(), depth + 1)
    })
}
