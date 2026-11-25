use serde::{Deserialize, Serialize};

/// The 3D Distance generator's controls: white around a point across the mesh's bounds, fading to black at the radius
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistanceParameters {
    /// The point (Position X, Y and Z) in the mesh's bounds scaled evenly, as the reference's position map is: the longest side spans 0..1, the others are centred on 0.5
    pub point: [f32; 3],
    /// How far the gradient reaches, in units of the bounds' longest side ([`DistanceParameters::RADIUS_RANGE`])
    pub radius: f32,
    /// Slides the gradient through: 0 all black, 1 all white
    pub offset: f32,
    /// Narrows the gradient (0..1; 1 a hard edge)
    pub contrast: f32,
}

impl DistanceParameters {
    pub const RADIUS_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.5;

    /// Every value brought into its range, non-finite ones to the defaults
    pub fn clamped(self) -> DistanceParameters {
        let defaults = DistanceParameters::default();
        let within = |value: f32, default: f32, range: std::ops::RangeInclusive<f32>| {
            if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default }
        };

        DistanceParameters {
            point: std::array::from_fn(|axis| within(self.point[axis], defaults.point[axis], 0.0..=1.0)),
            radius: within(self.radius, defaults.radius, DistanceParameters::RADIUS_RANGE),
            offset: within(self.offset, defaults.offset, 0.0..=1.0),
            contrast: within(self.contrast, defaults.contrast, 0.0..=1.0),
        }
    }
}

impl Default for DistanceParameters {
    /// The reference app's defaults: the bounds' centre, radius 1
    fn default() -> DistanceParameters {
        DistanceParameters { point: [0.5; 3], radius: 1.0, offset: 0.5, contrast: 0.5 }
    }
}
