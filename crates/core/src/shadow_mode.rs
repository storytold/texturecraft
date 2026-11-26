use serde::{Deserialize, Serialize};

/// How the key light's shadows are computed (Display settings > Computation mode): the shadow map's resolution
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShadowMode {
    Intensive,
    Average,
    #[default]
    Lightweight,
}

impl ShadowMode {
    pub const ALL: [ShadowMode; 3] = [ShadowMode::Intensive, ShadowMode::Average, ShadowMode::Lightweight];

    pub fn label(self) -> &'static str {
        match self {
            ShadowMode::Intensive => "Intensive",
            ShadowMode::Average => "Average",
            ShadowMode::Lightweight => "Lightweight",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            ShadowMode::Intensive => "intensive",
            ShadowMode::Average => "average",
            ShadowMode::Lightweight => "lightweight",
        }
    }

    pub fn parse(id: &str) -> Option<ShadowMode> {
        Self::ALL.into_iter().find(|mode| mode.id().eq_ignore_ascii_case(id))
    }

    /// Edge of the square shadow map, in texels
    pub fn map_size(self) -> u32 {
        match self {
            ShadowMode::Intensive => 4096,
            ShadowMode::Average => 3072,
            ShadowMode::Lightweight => 2048,
        }
    }
}
