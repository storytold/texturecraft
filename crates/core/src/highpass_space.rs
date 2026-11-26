//! The color space a Highpass filter works in

use serde::{Deserialize, Serialize};

use crate::gradient_input::{linear_to_srgb, srgb_to_linear};

/// ACEScg (AP1) to linear sRGB (Rec. 709) primaries, white point adapted (the published ACES matrix)
const AP1_TO_REC709: [[f32; 3]; 3] = [[1.705_05, -0.621_79, -0.083_26], [-0.130_26, 1.140_80, -0.010_55], [-0.024_00, -0.128_97, 1.152_97]];

/// Linear sRGB (Rec. 709) to ACEScg (AP1) primaries: the inverse of [`AP1_TO_REC709`]
const REC709_TO_AP1: [[f32; 3]; 3] = [[0.613_10, 0.339_52, 0.047_37], [0.070_19, 0.916_35, 0.013_45], [0.020_62, 0.109_57, 0.869_81]];

/// Where a Highpass takes its detail: in the working space (masks always), or in sRGB from linear sRGB or ACEScg colors
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HighpassSpace {
    /// The values as they are (what a Highpass did in projects saved before the Color Space)
    #[default]
    Working,
    /// Colors read as linear sRGB, filtered sRGB encoded
    SrgbFromLinear,
    /// Colors read as ACEScg, filtered sRGB encoded
    SrgbFromAcescg,
}

impl HighpassSpace {
    /// Every space, in the order a list shows them
    pub const ALL: [HighpassSpace; 3] = [HighpassSpace::Working, HighpassSpace::SrgbFromLinear, HighpassSpace::SrgbFromAcescg];

    /// The name a panel shows
    pub fn label(self) -> &'static str {
        match self {
            HighpassSpace::Working => "Working space / Mask",
            HighpassSpace::SrgbFromLinear => "sRGB (from Linear sRGB)",
            HighpassSpace::SrgbFromAcescg => "sRGB (from ACEScg)",
        }
    }

    /// The id `effect.set {colorSpace}` takes
    pub fn id(self) -> &'static str {
        match self {
            HighpassSpace::Working => "working",
            HighpassSpace::SrgbFromLinear => "srgbFromLinear",
            HighpassSpace::SrgbFromAcescg => "srgbFromAcescg",
        }
    }

    /// Straight color `rgb` in this space
    pub fn encode(self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            HighpassSpace::Working => rgb,
            HighpassSpace::SrgbFromLinear => rgb.map(linear_to_srgb),
            HighpassSpace::SrgbFromAcescg => times(&AP1_TO_REC709, rgb).map(linear_to_srgb),
        }
    }

    /// Straight color `rgb` of this space back in the working space
    pub fn decode(self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            HighpassSpace::Working => rgb,
            HighpassSpace::SrgbFromLinear => rgb.map(srgb_to_linear),
            HighpassSpace::SrgbFromAcescg => times(&REC709_TO_AP1, rgb.map(srgb_to_linear)),
        }
    }
}

/// Matrix `m` times column `v`
fn times(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_space_round_trips_a_color_in_its_gamut() {
        for space in HighpassSpace::ALL {
            let back = space.decode(space.encode([0.5, 0.4, 0.3]));
            assert!(back.iter().zip([0.5, 0.4, 0.3]).all(|(a, b)| (a - b).abs() < 2e-3), "{space:?}: {back:?}");
        }
        assert!((HighpassSpace::SrgbFromLinear.encode([0.214; 3])[0] - 0.5).abs() < 1e-2, "linear mid gray is sRGB 0.5");
    }
}
