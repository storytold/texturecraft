//! Depth culling of planar and warp projections

use serde::{Deserialize, Serialize};

/// Surfaces hidden from the projector behind other surfaces get none of the projection, faded softer as `hardness` drops
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepthCulling {
    pub on: bool,
    pub hardness: f32,
}

impl DepthCulling {
    /// What a new planar or warp projection starts with
    pub const NEW: DepthCulling = DepthCulling { on: true, hardness: 0.75 };
}

impl Default for DepthCulling {
    /// Off, as for every projection saved before depth culling existed
    fn default() -> Self {
        DepthCulling { on: false, ..DepthCulling::NEW }
    }
}
