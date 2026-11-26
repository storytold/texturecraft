use serde::{Deserialize, Serialize};

/// How a stencil sits on screen, without the image (which is a project resource)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StencilLook {
    pub center: [f32; 2],
    pub scale: f32,
    pub rotation: f32,
    pub tile: bool,
    pub opacity: f32,
}

impl Default for StencilLook {
    fn default() -> Self {
        StencilLook { center: [0.0, 0.0], scale: 0.8, rotation: 0.0, tile: false, opacity: 0.5 }
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
