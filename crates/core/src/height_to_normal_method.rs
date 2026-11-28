//! How a texture set turns its Height into normal slopes

use serde::{Deserialize, Serialize};

/// Height to normal method: the slope filter a texture set's height-to-normal uses
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HeightToNormalMethod {
    /// Sharp: the difference of the two neighbours across, so single-texel detail stays crisp
    Sharp,
    /// Smooth (Sobel): a 3×3 Sobel filter, so the slope also reads the diagonal neighbours
    #[default]
    Smooth,
}

impl HeightToNormalMethod {
    pub const ALL: [HeightToNormalMethod; 2] = [HeightToNormalMethod::Sharp, HeightToNormalMethod::Smooth];

    /// Stable id for commands and inspect
    pub fn id(self) -> &'static str {
        match self {
            HeightToNormalMethod::Sharp => "sharp",
            HeightToNormalMethod::Smooth => "smooth",
        }
    }

    pub fn parse(id: &str) -> Option<HeightToNormalMethod> {
        HeightToNormalMethod::ALL.into_iter().find(|value| value.id() == id)
    }

    /// The change of height per texel to the right and down, from the heights `h(dx, dy)` around a texel
    pub fn slope(self, h: impl Fn(i32, i32) -> f32) -> [f32; 2] {
        match self {
            HeightToNormalMethod::Sharp => [(h(1, 0) - h(-1, 0)) * 0.5, (h(0, 1) - h(0, -1)) * 0.5],
            HeightToNormalMethod::Smooth => {
                // Both axes read the same four corner heights
                let upper_left = h(-1, -1);
                let upper_right = h(1, -1);
                let lower_left = h(-1, 1);
                let lower_right = h(1, 1);

                [
                    (upper_right + 2.0 * h(1, 0) + lower_right - upper_left - 2.0 * h(-1, 0) - lower_left) / 8.0,
                    (lower_left + 2.0 * h(0, 1) + lower_right - upper_left - 2.0 * h(0, -1) - upper_right) / 8.0,
                ]
            }
        }
    }
}
