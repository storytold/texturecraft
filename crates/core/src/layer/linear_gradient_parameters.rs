use serde::{Deserialize, Serialize};

/// The 3D Linear Gradient generator's controls: black at the start point, white at the end point, across the mesh's bounds
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearGradientParameters {
    /// Where the gradient is black, in the mesh's bounds scaled evenly like 3D Distance's point (the longest side spans 0..1, the others are centred on 0.5)
    pub start: [f32; 3],
    /// Where the gradient is white
    pub end: [f32; 3],
    /// Shifts the gradient darker (below 0.5) or brighter (above 0.5)
    pub balance: f32,
    /// Narrows the gradient around its middle (0..1; 1 a hard step)
    pub contrast: f32,
}

impl LinearGradientParameters {
    /// Every value brought into its range, non-finite ones to the defaults
    pub fn clamped(self) -> LinearGradientParameters {
        let defaults = LinearGradientParameters::default();
        let unit = |value: f32, default: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { default };

        LinearGradientParameters {
            start: std::array::from_fn(|axis| unit(self.start[axis], defaults.start[axis])),
            end: std::array::from_fn(|axis| unit(self.end[axis], defaults.end[axis])),
            balance: unit(self.balance, defaults.balance),
            contrast: unit(self.contrast, defaults.contrast),
        }
    }
}

impl Default for LinearGradientParameters {
    /// The reference app's defaults: from y 0 to y 1 through the bounds' centre, which spans the mesh when its longest side is vertical
    fn default() -> LinearGradientParameters {
        LinearGradientParameters { start: [0.5, 0.0, 0.5], end: [0.5, 1.0, 0.5], balance: 0.5, contrast: 0.0 }
    }
}
