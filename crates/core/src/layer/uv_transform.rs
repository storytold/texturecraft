use serde::{Deserialize, Serialize};

/// Tiling, rotation (degrees) and offset a 3D projection lays on the surface; the default changes nothing
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UvTransform {
    pub tiling: [f32; 2],
    pub rotation: f32,
    pub offset: [f32; 2],
}

impl Default for UvTransform {
    fn default() -> Self {
        UvTransform { tiling: [1.0, 1.0], rotation: 0.0, offset: [0.0, 0.0] }
    }
}

impl UvTransform {
    /// Whether it changes nothing, as projections saved before it existed did (not saved then)
    pub fn is_default(&self) -> bool {
        *self == UvTransform::default()
    }
}
