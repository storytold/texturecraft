use egui_phosphor::regular as ic;
use serde::{Deserialize, Serialize};

use crate::i18n::n_;

/// The Baking Log panel's tabs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BakingLogTab {
    /// Every map of the latest bake and how it went
    #[default]
    BakingProcess,
    /// The high poly files each texture set bakes from
    HighPolyParameters,
    /// Which low poly part bakes from which high poly part
    MatchingByName,
}

impl BakingLogTab {
    pub const ALL: [BakingLogTab; 3] = [BakingLogTab::BakingProcess, BakingLogTab::HighPolyParameters, BakingLogTab::MatchingByName];

    pub fn name(self) -> &'static str {
        match self {
            BakingLogTab::BakingProcess => n_("Baking Process"),
            BakingLogTab::HighPolyParameters => n_("High Poly Parameters"),
            BakingLogTab::MatchingByName => n_("Matching by Name"),
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            BakingLogTab::BakingProcess => ic::LIST_CHECKS,
            BakingLogTab::HighPolyParameters => ic::CUBE,
            BakingLogTab::MatchingByName => ic::ARROWS_LEFT_RIGHT,
        }
    }
}
