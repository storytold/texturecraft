use texturecraft_core::MicroDetails;

use super::micro_slot::MicroSlot;

/// One generator's micro details, as the maps the GPU builds for it
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MicroRequest {
    pub(super) micro: MicroDetails,
    pub(super) height: Option<MicroSlot>,
    pub(super) normal: Option<MicroSlot>,
    /// Build the curvature term (the generator reads its curvature input)
    pub(super) curvature: bool,
    /// Build the occlusion factor (it reads its occlusion input and has a micro height)
    pub(super) occlusion: bool,
}
