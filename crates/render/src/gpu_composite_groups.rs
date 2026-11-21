use texturecraft_core::{Channel, LayerId};

use super::{Operation, channel_slot};

pub(super) const BEGIN_GROUP: u32 = 200;
pub(super) const END_GROUP: u32 = 201;

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
    pub(super) fn finish(&mut self, operations: &[Operation]) {
        if self.depth == 0 {
            return;
        }
        for op in operations {
            if is_group(op) {
                self.markers += 1;
            } else {
                self.channels |= 1 << op.ids[0];
            }
        }
        // One word for the mask, then compact snapshots of the channels that can change.
        self.words_per_level = 1 + Channel::ALL
            .iter()
            .filter(|&&channel| self.channels & (1 << channel_slot(channel)) != 0)
            .map(|channel| channel.components() as u32)
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
