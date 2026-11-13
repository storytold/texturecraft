//! What bake mode's 3D view draws while a bake is set up: the high poly, the cage, the missing seams and the low poly

use crate::bake_overlay::BakeOverlay;
use crate::neutral_material::NeutralMaterial;

/// Default overlay colors, linear RGB
const HIGH_POLY_COLOR: [f32; 3] = [0.0, 0.216, 0.521];
const MATCHING_ERROR_COLOR: [f32; 3] = [1.0, 0.0, 0.0];
const CAGE_SURFACE_COLOR: [f32; 3] = [0.604, 0.451, 0.216];
const CAGE_WIRE_COLOR: [f32; 3] = [0.604, 0.604, 0.604];
const MISSING_SEAM_COLOR: [f32; 3] = [0.451, 0.0, 0.451];
const LOW_POLY_COLOR: [f32; 3] = [0.216, 0.216, 0.216];

/// Which parts of the bake setup the 3D view draws, and how
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeVisualization {
    pub high_poly: BakeOverlay,
    /// Tints the high poly where the bake's rays miss it
    pub matching_error: BakeOverlay,
    pub cage_surface: BakeOverlay,
    pub cage_wire: BakeOverlay,
    /// The low poly's hard edges that are not UV seams
    pub missing_seams: BakeOverlay,
    /// The low poly; its color is the neutral material's color
    pub low_poly: BakeOverlay,
    /// Shade the low poly with this plain material instead of its painted one
    pub neutral: Option<NeutralMaterial>,
    /// Draw the cage and high poly of only this texture set (the high poly's vertices of other sets come marked hidden)
    pub only_set: Option<usize>,
}

impl Default for BakeVisualization {
    /// Every part hidden, in its default color, the low poly with its painted material
    fn default() -> Self {
        Self {
            high_poly: BakeOverlay::hidden(HIGH_POLY_COLOR),
            matching_error: BakeOverlay::hidden(MATCHING_ERROR_COLOR),
            cage_surface: BakeOverlay::hidden(CAGE_SURFACE_COLOR),
            cage_wire: BakeOverlay::hidden(CAGE_WIRE_COLOR),
            missing_seams: BakeOverlay::hidden(MISSING_SEAM_COLOR),
            low_poly: BakeOverlay::hidden(LOW_POLY_COLOR),
            neutral: None,
            only_set: None,
        }
    }
}

impl BakeVisualization {
    /// Whether any part of the cage shows
    pub fn cage_shown(&self) -> bool {
        self.cage_surface.shown || self.cage_wire.shown
    }

    /// The neutral material while the low poly shows with it
    pub fn shown_neutral(&self) -> Option<NeutralMaterial> {
        self.neutral.filter(|_| self.low_poly.shown)
    }
}
