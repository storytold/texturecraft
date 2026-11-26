//! How a wear generator's level and contrast turn its signal into the mask (constants fitted to the reference app's sweeps)

use texturecraft_core::WearParameters;

/// Where a kind's level slides its edge and how wide the edge is
pub(crate) struct WearScan {
    /// The edge at level 0, falling towards `floor` as the level rises
    pub travel: f32,
    pub floor: f32,
    /// The edge's width at contrast 0, narrowed by (1 − contrast) to the power `narrowing`
    pub width: f32,
    pub narrowing: f32,
}

pub(crate) const EDGE_WEAR_SCAN: WearScan = WearScan { travel: 0.284, floor: -0.009, width: 0.504, narrowing: 0.95 };
pub(crate) const FIBER_GLASS_SCAN: WearScan = WearScan { travel: 0.162, floor: 0.029, width: 0.358, narrowing: 0.872 };
pub(crate) const DIRT_SCAN: WearScan = WearScan { travel: 0.593, floor: -0.088, width: 1.386, narrowing: 1.42 };

/// The narrowest edge, so a level or contrast of 1 is a hard step rather than a division by zero
const MIN_EDGE_WIDTH: f32 = 1e-4;

impl WearScan {
    /// `signal` through the level and contrast: the level brings the edge down and narrows it, the contrast narrows it further
    pub(crate) fn apply(&self, signal: f32, wear: &WearParameters) -> f32 {
        let remaining = (1.0 - wear.level).max(0.0);
        let width = (remaining * self.width * (1.0 - wear.contrast).max(0.0).powf(self.narrowing)).max(MIN_EDGE_WIDTH);
        ((signal - self.floor - remaining * self.travel) / width + 0.5).clamp(0.0, 1.0)
    }
}
