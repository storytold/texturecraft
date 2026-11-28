//! Named ways of using a tool, each mapped onto settings the tool already has

use crate::session::{Tool, ToolState};

/// One variant of a tool, as the tool shelf's variant menu lists it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolVariant {
    Paint,
    PhysicalPaint,
    Eraser,
    PhysicalEraser,
    Projection,
    PhysicalProjection,
    CloneRelative,
    CloneAbsolute,
    PathAlong,
    PathFilled,
}

impl ToolVariant {
    pub const ALL: [ToolVariant; 10] = [
        ToolVariant::Paint,
        ToolVariant::PhysicalPaint,
        ToolVariant::Eraser,
        ToolVariant::PhysicalEraser,
        ToolVariant::Projection,
        ToolVariant::PhysicalProjection,
        ToolVariant::CloneRelative,
        ToolVariant::CloneAbsolute,
        ToolVariant::PathAlong,
        ToolVariant::PathFilled,
    ];

    /// The id after `tool.variant.` in its command id
    pub fn id(self) -> &'static str {
        match self {
            ToolVariant::Paint => "paint",
            ToolVariant::PhysicalPaint => "physicalPaint",
            ToolVariant::Eraser => "eraser",
            ToolVariant::PhysicalEraser => "physicalEraser",
            ToolVariant::Projection => "projection",
            ToolVariant::PhysicalProjection => "physicalProjection",
            ToolVariant::CloneRelative => "cloneRelative",
            ToolVariant::CloneAbsolute => "cloneAbsolute",
            ToolVariant::PathAlong => "pathAlong",
            ToolVariant::PathFilled => "pathFilled",
        }
    }

    pub fn command_id(self) -> &'static str {
        match self {
            ToolVariant::Paint => "tool.variant.paint",
            ToolVariant::PhysicalPaint => "tool.variant.physicalPaint",
            ToolVariant::Eraser => "tool.variant.eraser",
            ToolVariant::PhysicalEraser => "tool.variant.physicalEraser",
            ToolVariant::Projection => "tool.variant.projection",
            ToolVariant::PhysicalProjection => "tool.variant.physicalProjection",
            ToolVariant::CloneRelative => "tool.variant.cloneRelative",
            ToolVariant::CloneAbsolute => "tool.variant.cloneAbsolute",
            ToolVariant::PathAlong => "tool.variant.pathAlong",
            ToolVariant::PathFilled => "tool.variant.pathFilled",
        }
    }

    /// The English label, also the translation key
    pub fn label(self) -> &'static str {
        match self {
            ToolVariant::Paint => "Paint",
            ToolVariant::PhysicalPaint => "Physical paint",
            ToolVariant::Eraser => "Eraser",
            ToolVariant::PhysicalEraser => "Physical eraser",
            ToolVariant::Projection => "Projection",
            ToolVariant::PhysicalProjection => "Physical projection",
            ToolVariant::CloneRelative => "Clone (relative source)",
            ToolVariant::CloneAbsolute => "Clone (absolute source)",
            ToolVariant::PathAlong => "Paint along path",
            ToolVariant::PathFilled => "Filled path",
        }
    }

    pub fn tool(self) -> Tool {
        match self {
            ToolVariant::Paint | ToolVariant::PhysicalPaint => Tool::Paint,
            ToolVariant::Eraser | ToolVariant::PhysicalEraser => Tool::Eraser,
            ToolVariant::Projection | ToolVariant::PhysicalProjection => Tool::Projection,
            ToolVariant::CloneRelative | ToolVariant::CloneAbsolute => Tool::Clone,
            ToolVariant::PathAlong | ToolVariant::PathFilled => Tool::Path,
        }
    }

    /// Whether this variant emits particles (the shared brush's particle switch)
    pub fn physical(self) -> bool {
        matches!(self, ToolVariant::PhysicalPaint | ToolVariant::PhysicalEraser | ToolVariant::PhysicalProjection)
    }

    /// The variants of `tool`, plain first; empty for a tool without variants
    pub fn of(tool: Tool) -> &'static [ToolVariant] {
        match tool {
            Tool::Paint => &[ToolVariant::Paint, ToolVariant::PhysicalPaint],
            Tool::Eraser => &[ToolVariant::Eraser, ToolVariant::PhysicalEraser],
            Tool::Projection => &[ToolVariant::Projection, ToolVariant::PhysicalProjection],
            Tool::Clone => &[ToolVariant::CloneRelative, ToolVariant::CloneAbsolute],
            Tool::Path => &[ToolVariant::PathAlong, ToolVariant::PathFilled],
            Tool::PolygonFill | Tool::Smudge | Tool::ColorPicker | Tool::GeometryMask => &[],
        }
    }

    /// The variant `tool` is in with today's settings; None for a tool without variants
    pub fn current(state: &ToolState, tool: Tool) -> Option<ToolVariant> {
        let physical = state.brush.particles.enabled;
        let path = &state.path;
        let variant = match tool {
            Tool::Paint if physical => ToolVariant::PhysicalPaint,
            Tool::Paint => ToolVariant::Paint,
            Tool::Eraser if physical => ToolVariant::PhysicalEraser,
            Tool::Eraser => ToolVariant::Eraser,
            Tool::Projection if physical => ToolVariant::PhysicalProjection,
            Tool::Projection => ToolVariant::Projection,
            Tool::Clone if state.clone_aligned => ToolVariant::CloneRelative,
            Tool::Clone => ToolVariant::CloneAbsolute,

            // A filled outline only fills while closed
            Tool::Path if path.closed && path.fill => ToolVariant::PathFilled,
            Tool::Path => ToolVariant::PathAlong,
            Tool::PolygonFill | Tool::Smudge | Tool::ColorPicker | Tool::GeometryMask => return None,
        };
        Some(variant)
    }

    /// The variant with command id `id`
    pub fn from_command_id(id: &str) -> Option<ToolVariant> {
        ToolVariant::ALL.into_iter().find(|variant| variant.command_id() == id)
    }
}
