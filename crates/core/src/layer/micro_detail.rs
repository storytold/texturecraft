use serde::{Deserialize, Serialize};

use super::LayerId;

/// Painted height read from an anchor point below a generator
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MicroDetail {
    /// The anchor point effect id
    pub anchor: LayerId,
    /// Strength of the height signal from 0 to 1
    pub amount: f32,
}

impl MicroDetail {
    pub const DEFAULT_AMOUNT: f32 = 0.5;
}
