use serde::{Deserialize, Serialize};

/// The UV Border Distance generator's controls: white at the UV islands' borders, fading towards their insides
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BorderDistanceParameters {
    /// How far in from the border the gradient reaches (0..1; 1 is half a UV tile)
    pub distance: f32,
    /// Softens the gradient, the empty space around the islands darkening their borders (0..1)
    pub smoothness: f32,
    /// Shifts the gradient darker (below 0.5) or brighter (above 0.5)
    pub balance: f32,
    /// Narrows the gradient around its middle (0..1; 1 a hard step)
    pub contrast: f32,
}

impl BorderDistanceParameters {
    /// Every value brought into 0..1, non-finite ones to the defaults
    pub fn clamped(self) -> BorderDistanceParameters {
        let defaults = BorderDistanceParameters::default();
        let unit = |value: f32, default: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { default };

        BorderDistanceParameters {
            distance: unit(self.distance, defaults.distance),
            smoothness: unit(self.smoothness, defaults.smoothness),
            balance: unit(self.balance, defaults.balance),
            contrast: unit(self.contrast, defaults.contrast),
        }
    }
}

impl Default for BorderDistanceParameters {
    /// The reference app's defaults
    fn default() -> BorderDistanceParameters {
        BorderDistanceParameters { distance: 0.1, smoothness: 0.0, balance: 0.5, contrast: 0.0 }
    }
}
