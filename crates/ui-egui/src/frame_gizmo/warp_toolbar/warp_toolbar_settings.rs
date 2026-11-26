use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use super::{GizmoMode, GizmoSpace};

/// The manipulator size the gizmo's measurements are given at
pub(super) const DEFAULT_MANIPULATOR_SIZE: f32 = 0.2;
pub(super) const MANIPULATOR_SIZES: RangeInclusive<f32> = 0.1..=0.5;

/// The default and range of the move step (world units) and of the turn step (degrees)
pub(super) const DEFAULT_GRID_STEP: f32 = 0.2;
pub(super) const GRID_STEPS: RangeInclusive<f32> = 0.01..=1.0;
pub(super) const DEFAULT_ANGLE_STEP: f32 = 45.0;
pub(super) const ANGLE_STEPS: RangeInclusive<f32> = 1.0..=90.0;

/// The warp tool bar's settings: the gizmo's mode, space and size, and the steps its drags snap to while Ctrl is held
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WarpToolbarSettings {
    pub mode: GizmoMode,
    pub space: GizmoSpace,
    /// Manipulator size: [`DEFAULT_MANIPULATOR_SIZE`] draws the gizmo at its base size
    pub manipulator_size: f32,
    /// Move step in world units
    pub grid_steps: f32,
    /// Turn step in degrees
    pub angle_steps: f32,
}

impl Default for WarpToolbarSettings {
    fn default() -> Self {
        WarpToolbarSettings {
            mode: GizmoMode::default(),
            space: GizmoSpace::default(),
            manipulator_size: DEFAULT_MANIPULATOR_SIZE,
            grid_steps: DEFAULT_GRID_STEP,
            angle_steps: DEFAULT_ANGLE_STEP,
        }
    }
}

impl WarpToolbarSettings {
    /// How many times its base size the gizmo's arrows and ring are drawn
    pub(crate) fn gizmo_scale(&self) -> f32 {
        within(self.manipulator_size, MANIPULATOR_SIZES, DEFAULT_MANIPULATOR_SIZE) / DEFAULT_MANIPULATOR_SIZE
    }

    /// The move step in world units
    pub(crate) fn grid_step(&self) -> f32 {
        within(self.grid_steps, GRID_STEPS, DEFAULT_GRID_STEP)
    }

    /// The turn step in degrees
    pub(crate) fn angle_step(&self) -> f32 {
        within(self.angle_steps, ANGLE_STEPS, DEFAULT_ANGLE_STEP)
    }
}

/// `value` held to `range`; `fallback` for a saved value that is not a number
fn within(value: f32, range: RangeInclusive<f32>, fallback: f32) -> f32 {
    if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { fallback }
}
