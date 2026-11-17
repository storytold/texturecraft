//! One overlay of bake mode's 3D view: whether it shows, its color and its opacity

/// Whether a part of the bake setup shows in the 3D view, and in which color and opacity
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BakeOverlay {
    pub shown: bool,
    /// Linear RGB
    pub color: [f32; 3],
    /// 0 transparent, 1 opaque; the matching error mixes into the high poly by it
    pub opacity: f32,
}

impl BakeOverlay {
    /// A hidden overlay in `color`, fully opaque
    pub const fn hidden(color: [f32; 3]) -> Self {
        Self { shown: false, color, opacity: 1.0 }
    }

    /// The overlay's shader globals: rgb its color, a its opacity (0 while hidden)
    pub(crate) fn globals(&self) -> [f32; 4] {
        let [red, green, blue] = self.color;
        [red, green, blue, if self.shown { self.opacity.clamp(0.0, 1.0) } else { 0.0 }]
    }
}
