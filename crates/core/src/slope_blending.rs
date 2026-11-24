//! How a Blur Slope combines what it reads downhill

use serde::{Deserialize, Serialize};

/// What a Blur Slope keeps of the values it reads along the slope
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlopeBlending {
    /// Their average: the values smeared downhill
    #[default]
    Blur,
    /// The smallest of them, per component: light shapes eaten away downhill
    Min,
    /// The largest of them, per component: light shapes grown downhill
    Max,
}

impl SlopeBlending {
    /// Every choice, in the order a panel lists them
    pub const ALL: [SlopeBlending; 3] = [SlopeBlending::Blur, SlopeBlending::Min, SlopeBlending::Max];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            SlopeBlending::Blur => "Blur",
            SlopeBlending::Min => "Min",
            SlopeBlending::Max => "Max",
        }
    }

    /// The id `effect.set` takes
    pub fn id(self) -> &'static str {
        match self {
            SlopeBlending::Blur => "blur",
            SlopeBlending::Min => "min",
            SlopeBlending::Max => "max",
        }
    }
}
