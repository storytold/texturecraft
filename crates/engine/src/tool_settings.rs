//! The tool settings that outlive a project: they are kept when another project opens and are
//! saved with the UI state, so the next session starts with the same brush.

use serde::{Deserialize, Serialize};

use crate::session::{CloneFrom, StencilLook, ToolState};
use texturecraft_paint::BrushSettings;

/// Brush, symmetry, stencil placement, smudge strength, clone mode and alignment and path switches. Left out
/// because they belong to one project: the clone source point, the stencil image, the points of
/// the path being drawn and the brush's image alpha (an image resource of that project).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSettings {
    pub brush: BrushSettings,
    pub stencil: StencilLook,
    pub smudge_strength: f32,
    pub clone_from: CloneFrom,
    /// Settings saved before the Aligned switch existed keep its default (on)
    #[serde(default = "aligned_by_default")]
    pub clone_aligned: bool,
    pub path_closed: bool,
    pub path_fill: bool,
    /// Settings saved before the switch existed keep its default (on)
    #[serde(default = "crate::session::default_hide_stencil")]
    pub hide_stencil_while_painting: bool,
}

fn aligned_by_default() -> bool {
    true
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
            clone_aligned: tool.clone_aligned,
            path_closed: tool.path.closed,
            path_fill: tool.path.fill,
            hide_stencil_while_painting: tool.hide_stencil_while_painting,
        }
    }

    /// Make `tool` use these settings, leaving what belongs to its project alone.
    pub fn apply(&self, tool: &mut ToolState) {
        let alpha_image = tool.brush.alpha_image.take();
        tool.brush = self.brush.clone().sanitized();

        // An image alpha names a resource of the project that was open; the library alpha is global
        tool.brush.alpha_image = alpha_image;
        tool.stencil_look = self.stencil.sanitized();
        tool.smudge_strength = if self.smudge_strength.is_finite() { self.smudge_strength.clamp(0.0, 1.0) } else { crate::session::default_smudge_strength() };
        tool.clone_from = self.clone_from;
        tool.clone_aligned = self.clone_aligned;
        tool.path.closed = self.path_closed;
        tool.path.fill = self.path_fill;
        tool.hide_stencil_while_painting = self.hide_stencil_while_painting;
    }
}
