use serde::Deserialize;

use super::LayerId;

/// Micro detail as saved before Micro Details: the anchor point whose painted height it read, and how strongly
#[derive(Deserialize)]
pub(crate) struct AnchoredMicroDetail {
    pub anchor: LayerId,
    /// 0..1, 0.5 by default
    #[serde(default = "AnchoredMicroDetail::default_amount")]
    pub amount: f32,
}

impl AnchoredMicroDetail {
    fn default_amount() -> f32 {
        0.5
    }
}
