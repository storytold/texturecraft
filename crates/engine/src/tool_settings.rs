//! The tool settings that outlive a project: they are kept when another project opens and are
//! saved with the UI state, so the next session starts with the same brush.

use serde::{Deserialize, Serialize};

use crate::session::{CloneFrom, StencilLook, ToolState};
use texturecraft_paint::BrushSettings;

/// Brush, symmetry, stencil placement, smudge strength, clone mode and path switches. Left out
/// because they belong to one project: the clone source point, the stencil image, the points of
/// the path being drawn and the brush's image alpha (an image resource of that project).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSettings {
    pub brush: BrushSettings,
    pub stencil: StencilLook,
    pub smudge_strength: f32,
    pub clone_from: CloneFrom,
    pub path_closed: bool,
    pub path_fill: bool,
}

impl Default for ToolSettings {
    fn default() -> Self {
        ToolSettings::of(&ToolState::default())
    }
}

impl ToolSettings {
    /// The settings `tool` has now.
    pub fn of(tool: &ToolState) -> ToolSettings {
        let mut brush = tool.brush.clone();
        brush.alpha_image = None;
        ToolSettings {
            brush,
            stencil: tool.stencil.as_ref().map_or(tool.stencil_look, |stencil| stencil.look()),
            smudge_strength: tool.smudge_strength,
            clone_from: tool.clone_from,
            path_closed: tool.path.closed,
            path_fill: tool.path.fill,
        }
    }

    /// Make `tool` use these settings, leaving what belongs to its project alone.
    pub fn apply(&self, tool: &mut ToolState) {
        let alpha_image = tool.brush.alpha_image.take();
        tool.brush = self.brush.clone().sanitized();
        // An image alpha names a resource of the project that was open; the library alpha is global
        tool.brush.alpha_image = alpha_image;
        tool.stencil_look = self.stencil;
        tool.smudge_strength = self.smudge_strength;
        tool.clone_from = self.clone_from;
        tool.path.closed = self.path_closed;
        tool.path.fill = self.path_fill;
    }
}
