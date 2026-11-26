//! The 3D projection settings shared by tri-planar, planar, spherical and cylindrical projections

use serde::{Deserialize, Serialize};

use crate::{Backface, DepthCulling, ShapeCrop};

/// Rotation (degrees about X, then Y, then Z) and per-axis scale of a 3D projection's shape; default = unchanged
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Projector {
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    pub crop: ShapeCrop,
    pub backface: Backface,
    /// Preserve axis-planar coverage when the projection is placed on a frame.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ignore_image_alpha: bool,
    /// Planar and warp projections only
    pub depth: DepthCulling,
}

impl Default for Projector {
    fn default() -> Self {
        Projector {
            rotation: [0.0; 3],
            scale: [1.0; 3],
            crop: ShapeCrop::default(),
            backface: Backface::default(),
            ignore_image_alpha: false,
            depth: DepthCulling::default(),
        }
    }
}

impl Projector {
    /// The settings a new planar or warp projection starts with: depth culled
    pub fn depth_culled() -> Projector {
        Projector { depth: DepthCulling::NEW, ..Projector::default() }
    }

    /// Whether it is the default, which projections saved before it existed load as (not saved then)
    pub fn is_default(&self) -> bool {
        *self == Projector::default()
    }

    /// Whether it turns or stretches the projection at all
    pub fn turns(&self) -> bool {
        self.rotation != [0.0; 3] || self.scale != [1.0; 3]
    }
}
