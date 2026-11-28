use std::sync::Arc;

use texturecraft_core::Channel;
use texturecraft_texel::{TILE, TILE_AREA};

use super::gpu_composite::channel_slot;

/// Sparse tiles of the linear material at the point where CPU compositing ends.
#[derive(Debug)]
pub struct MaterialUnderlay {
    pub(crate) size: (u32, u32),
    pub(crate) words: Vec<u32>,
    channels: [bool; Channel::ALL.len()],
    limit: usize,
}

/// The top bit of a channel's offset marks a single value shared by every texel in that tile.
const UNIFORM: u32 = 1 << 31;

impl MaterialUnderlay {
    pub fn new(size: (u32, u32), byte_limit: u64) -> Option<Self> {
        let limit = (byte_limit / 4).min(u64::from(UNIFORM)) as usize;
        let columns = size.0.div_ceil(TILE);
        let rows = size.1.div_ceil(TILE);
        let length = usize::try_from(1 + u64::from(columns) * u64::from(rows)).ok()?;
        if length > limit {
            return None;
        }

        let mut words = vec![0; length];
        words[0] = columns;
        Some(Self { size, words, channels: [false; Channel::ALL.len()], limit })
    }

    pub(crate) fn has_channel(&self, channel: Channel) -> bool {
        self.channels[channel_slot(channel) as usize]
    }

    /// Pack a tile. Untouched channels need no data, and a constant channel stores one texel.
    /// Discard the underlay if the tile is invalid or exceeds the device's storage binding limit.
    pub fn record_tile(&mut self, (x, y): (u32, u32), channels: &[Channel], tiles: Vec<Option<Arc<Vec<f32>>>>) -> bool {
        if x >= self.words[0] || y >= self.size.1.div_ceil(TILE) || channels.len() != tiles.len() {
            return false;
        }

        let mut offsets = [0; Channel::ALL.len()];
        for (&channel, tile) in channels.iter().zip(tiles) {
            let Some(values) = tile else { continue };
            let components = channel.components();
            if values.len() != TILE_AREA * components {
                return false;
            }

            let first = &values[..components];
            let uniform = values.chunks_exact(components).all(|pixel| pixel == first);
            if uniform && first == &channel.default_value()[..components] {
                continue;
            }

            let stored = if uniform { first } else { values.as_slice() };
            if self.words.len() + stored.len() > self.limit {
                return false;
            }

            let slot = channel_slot(channel) as usize;
            offsets[slot] = self.words.len() as u32 | if uniform { UNIFORM } else { 0 };
            self.words.extend(stored.iter().map(|value| value.to_bits()));
            self.channels[slot] = true;
        }

        if offsets.iter().any(|&offset| offset != 0) {
            if self.words.len() + offsets.len() > self.limit {
                return false;
            }
            let index = 1 + (y * self.words[0] + x) as usize;
            self.words[index] = self.words.len() as u32;
            self.words.extend(offsets);
        }
        true
    }
}
