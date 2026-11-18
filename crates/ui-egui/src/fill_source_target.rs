//! Which fill a source slot sets

use serde_json::Value;
use texturecraft_engine::core::Channel;

/// Which fill the slot sets, and how
pub(crate) enum FillSourceTarget {
    /// A mask fill effect: one grayscale value, set with `effect.set`
    Mask { effect: u64 },
    /// One channel of a fill layer or content fill (`base` names it), set with `layer.fill.set`
    Channel { base: Value, channel: Channel },
}
