use serde::{Deserialize, Serialize};

/// How a stencil repeats along its own X (horizontal) and Y (vertical) axes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StencilTiling {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

impl StencilTiling {
    pub const ALL: [StencilTiling; 4] = [StencilTiling::None, StencilTiling::Horizontal, StencilTiling::Vertical, StencilTiling::Both];

    /// Repeats along the stencil's X axis
    pub fn horizontal(self) -> bool {
        matches!(self, StencilTiling::Horizontal | StencilTiling::Both)
    }

    /// Repeats along the stencil's Y axis
    pub fn vertical(self) -> bool {
        matches!(self, StencilTiling::Vertical | StencilTiling::Both)
    }

    /// The name in saved projects and commands
    pub fn key(self) -> &'static str {
        match self {
            StencilTiling::None => "none",
            StencilTiling::Horizontal => "horizontal",
            StencilTiling::Vertical => "vertical",
            StencilTiling::Both => "both",
        }
    }

    /// A saved or commanded value: a mode name, or the old on/off switch (on meant both directions)
    pub fn from_value(value: &serde_json::Value) -> Option<StencilTiling> {
        match value {
            serde_json::Value::Bool(on) => Some(if *on { StencilTiling::Both } else { StencilTiling::None }),
            serde_json::Value::String(name) => StencilTiling::ALL.into_iter().find(|mode| mode.key() == name),
            _ => None,
        }
    }
}

impl Serialize for StencilTiling {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for StencilTiling {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        StencilTiling::from_value(&value).ok_or_else(|| serde::de::Error::custom("stencil tiling must be none, horizontal, vertical or both (or true/false)"))
    }
}

/// How a stencil sits on screen, without the image (which is a project resource)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StencilLook {
    pub center: [f32; 2],
    pub scale: f32,
    pub rotation: f32,
    #[serde(alias = "tile")]
    pub tiling: StencilTiling,
    pub opacity: f32,
}

impl Default for StencilLook {
    fn default() -> Self {
        StencilLook { center: [0.0, 0.0], scale: 0.8, rotation: 0.0, tiling: StencilTiling::None, opacity: 0.5 }
    }
}

impl StencilLook {
    pub(crate) const MAX_CENTER_OFFSET: f32 = 4.0;
    pub(crate) const MIN_SCALE: f32 = 0.01;
    pub(crate) const MAX_SCALE: f32 = 10.0;

    pub(crate) fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        let finite = |value: f32, fallback| if value.is_finite() { value } else { fallback };
        self.center = self.center.map(|v| finite(v, 0.0).clamp(-Self::MAX_CENTER_OFFSET, Self::MAX_CENTER_OFFSET));
        self.scale = finite(self.scale, defaults.scale).clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        self.rotation = finite(self.rotation, defaults.rotation).rem_euclid(360.0);
        self.opacity = finite(self.opacity, defaults.opacity).clamp(0.0, 1.0);
        self
    }
}
