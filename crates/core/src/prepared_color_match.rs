//! Reusable OKLab references for moving a region of colors

use crate::color_match::{matched_linear, polar};
use crate::{ColorMatch, oklab};

/// The settings and converted references for a region with constant source and target colors
#[derive(Clone, Copy, Debug)]
pub struct PreparedColorMatch {
    settings: ColorMatch,
    source: [f32; 3],
    target: [f32; 3],
}

impl PreparedColorMatch {
    /// Prepare the fixed reference pair once for the region
    pub fn new(settings: ColorMatch, source: [f32; 3], target: [f32; 3]) -> Self {
        let reference = |color: [f32; 3]| {
            let lab = oklab::from_linear(color.map(|component| component.max(0.0)));
            if settings.custom_variation { polar(lab) } else { lab }
        };
        Self { settings, source: reference(source), target: reference(target) }
    }

    /// Move a texel using the prepared reference pair
    pub fn apply(&self, color: [f32; 3]) -> [f32; 3] {
        let lab = oklab::from_linear(color.map(|component| component.max(0.0)));
        let moved = if self.settings.custom_variation {
            self.settings.move_polar(polar(lab), self.source, self.target)
        } else {
            std::array::from_fn(|index| self.target[index] + lab[index] - self.source[index])
        };

        matched_linear(color, moved)
    }
}
