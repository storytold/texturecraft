use serde::{Deserialize, Serialize};

use super::{Filtering, ScaleMode, UvWrap};
use crate::FillSymmetry;

/// How a fill samples its images and what its size is set in; older fills read with [`Sampling::SAVED_BEFORE`]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Sampling {
    #[serde(default)]
    pub filtering: Filtering,
    #[serde(default)]
    pub wrap: UvWrap,
    #[serde(default)]
    pub scale: ScaleMode,
    #[serde(default, skip_serializing_if = "FillSymmetry::is_default")]
    pub symmetry: FillSymmetry,
}

impl Sampling {
    /// What fills saved before the sampling choices existed did: plain bilinear
    pub const SAVED_BEFORE: Sampling =
        Sampling { filtering: Filtering::BilinearSharp, wrap: UvWrap::Repeat, scale: ScaleMode::Tiling, symmetry: FillSymmetry::OFF };

    /// Serde default for fills saved before the field existed
    pub fn saved_before() -> Sampling {
        Sampling::SAVED_BEFORE
    }

    /// Whether it samples as fills saved before the field existed did (not saved then, so they save as they were)
    pub fn is_saved_before(&self) -> bool {
        *self == Sampling::SAVED_BEFORE
    }
}
