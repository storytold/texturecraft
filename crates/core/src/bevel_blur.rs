//! How a Bevel's Smoothing spreads over its slope

use serde::{Deserialize, Serialize};

/// Where a Bevel's Smoothing blurs its slope
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BevelBlur {
    /// Sharp at the slope's foot, smoother towards its top
    #[default]
    NonUniform,
    /// The same blur over the whole slope
    Uniform,
}

impl BevelBlur {
    /// Every blur mode, in the order a switch shows them
    pub const ALL: [BevelBlur; 2] = [BevelBlur::NonUniform, BevelBlur::Uniform];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            BevelBlur::NonUniform => "Non-Uniform",
            BevelBlur::Uniform => "Uniform",
        }
    }

    /// The id `effect.set {blurMode}` takes
    pub fn id(self) -> &'static str {
        match self {
            BevelBlur::NonUniform => "nonUniform",
            BevelBlur::Uniform => "uniform",
        }
    }
}
