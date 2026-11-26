//! What Baked Lighting Stylized paints into the base color

use serde::{Deserialize, Serialize};

/// The lit parts Baked Lighting Stylized keeps
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LightingOutput {
    /// The lit surface and its highlights
    #[default]
    DiffuseSpecular,
    /// The lit surface only (what Baked Lighting painted before it had highlights)
    Diffuse,
    /// The highlights only
    Specular,
}

impl LightingOutput {
    /// Every output, in the order a list shows them
    pub const ALL: [LightingOutput; 3] = [LightingOutput::DiffuseSpecular, LightingOutput::Diffuse, LightingOutput::Specular];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            LightingOutput::DiffuseSpecular => "Diffuse + Specular",
            LightingOutput::Diffuse => "Diffuse only",
            LightingOutput::Specular => "Specular only",
        }
    }

    /// The id `effect.set {output}` takes
    pub fn id(self) -> &'static str {
        match self {
            LightingOutput::DiffuseSpecular => "diffuseSpecular",
            LightingOutput::Diffuse => "diffuse",
            LightingOutput::Specular => "specular",
        }
    }

    /// Whether the lit surface is kept
    pub fn diffuse(self) -> bool {
        self != LightingOutput::Specular
    }

    /// Whether the highlights are kept
    pub fn specular(self) -> bool {
        self != LightingOutput::Diffuse
    }
}
