//! What a compare mask holds, as its properties show it

use texturecraft_engine::core::{Channel, CompareOp, CompareSource};

/// What a compare mask holds: its operator, constant, tolerance, sources, channel and hardness
#[derive(Clone, Copy)]
pub(crate) struct CompareSettings {
    pub(crate) op: CompareOp,
    pub(crate) constant: f32,
    pub(crate) tolerance: f32,
    pub(crate) left: CompareSource,
    pub(crate) right: CompareSource,
    pub(crate) channel: Channel,
    pub(crate) hardness: f32,
}
