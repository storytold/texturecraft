//! Which texels PBR Validate checks

use serde::{Deserialize, Serialize};

/// PBR Validate's Validation Mode: the non-metal base colors, the metal ones, or both
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationMode {
    Albedo,
    Metal,
    #[default]
    Combined,
}

impl ValidationMode {
    /// Every mode, in the order a switch shows them
    pub const ALL: [ValidationMode; 3] = [ValidationMode::Albedo, ValidationMode::Metal, ValidationMode::Combined];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            ValidationMode::Albedo => "Albedo",
            ValidationMode::Metal => "Metal",
            ValidationMode::Combined => "Combined",
        }
    }

    /// The id `effect.set {validationMode}` takes
    pub fn id(self) -> &'static str {
        match self {
            ValidationMode::Albedo => "albedo",
            ValidationMode::Metal => "metal",
            ValidationMode::Combined => "combined",
        }
    }

    /// Whether non-metal base colors are checked
    pub fn checks_albedo(self) -> bool {
        self != ValidationMode::Metal
    }

    /// Whether metal base colors are checked
    pub fn checks_metal(self) -> bool {
        self != ValidationMode::Albedo
    }
}
