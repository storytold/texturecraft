//! Commands that select a tool in one of its variants, and list the variant each tool is in

use serde_json::{Map, Value, json};

use crate::commands::{CommandSpec, always, execute};
use crate::session::Tool;
use crate::tool_cmds::set_tool;
use crate::tool_variant::ToolVariant;
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("tool.variant.paint", "Paint", [], None, "{} — the Paint tool without particles", always, |s, _| pick(s, ToolVariant::Paint)),
        cmd!(
            "tool.variant.physicalPaint",
            "Physical paint",
            [],
            None,
            "{} — the Paint tool with particles (the shared brush's particle switch; 3D view strokes only)",
            always,
            |s, _| pick(s, ToolVariant::PhysicalPaint)
        ),
        cmd!("tool.variant.eraser", "Eraser", [], None, "{} — the Eraser without particles", always, |s, _| pick(s, ToolVariant::Eraser)),
        cmd!(
            "tool.variant.physicalEraser",
            "Physical eraser",
            [],
            None,
            "{} — the Eraser with particles (the shared brush's particle switch; 3D view strokes only)",
            always,
            |s, _| pick(s, ToolVariant::PhysicalEraser)
        ),
        cmd!("tool.variant.projection", "Projection", [], None, "{} — the Projection tool without particles", always, |s, _| pick(
            s,
            ToolVariant::Projection
        )),
        cmd!(
            "tool.variant.physicalProjection",
            "Physical projection",
            [],
            None,
            "{} — the Projection tool with particles (the shared brush's particle switch; 3D view strokes only)",
            always,
            |s, _| pick(s, ToolVariant::PhysicalProjection)
        ),
        cmd!(
            "tool.variant.cloneRelative",
            "Clone (relative source)",
            [],
            None,
            "{} — the Clone tool with an aligned source: the first stroke fixes the offset for the following strokes",
            always,
            |s, _| pick(s, ToolVariant::CloneRelative)
        ),
        cmd!(
            "tool.variant.cloneAbsolute",
            "Clone (absolute source)",
            [],
            None,
            "{} — the Clone tool with a fixed source: every stroke starts sampling at the source point",
            always,
            |s, _| pick(s, ToolVariant::CloneAbsolute)
        ),
        cmd!(
            "tool.variant.pathAlong",
            "Paint along path",
            [],
            None,
            "{} — the Path tool painting along its curve (Fill off; an outline that is closed stays closed)",
            always,
            |s, _| pick(s, ToolVariant::PathAlong)
        ),
        cmd!(
            "tool.variant.pathFilled",
            "Filled path",
            [],
            None,
            "{} — the Path tool filling a closed outline (Closed and Fill on, kept for the next path)",
            always,
            |s, _| pick(s, ToolVariant::PathFilled)
        ),
        cmd!("tool.variants", "Tool variants", [], None, "{} — the variant each tool with variants is in", always, |s, _| Ok(variants(s))),
    ]
}

/// Select the variant's tool, then set the settings the variant maps to
fn pick(session: &mut Session, variant: ToolVariant) -> Result<Value> {
    let tool = variant.tool();
    set_tool(session, tool)?;
    match tool {
        Tool::Paint | Tool::Eraser | Tool::Projection => {
            execute(session, "tool.brush", &json!({ "particles": { "enabled": variant.physical() } }))?;
        }

        // Through `tool.brush`, so a new mode drops the offset of an earlier stroke
        Tool::Clone => {
            execute(session, "tool.brush", &json!({ "cloneAligned": variant == ToolVariant::CloneRelative }))?;
        }
        Tool::Path => {
            let path = &session.tool.path;

            // Paint along path keeps a closed outline closed, but with no points there is no outline to keep
            let keeps_closed = path.closed && !(path.fill && path.points.is_empty());
            let (closed, fill) = if variant == ToolVariant::PathFilled { (true, true) } else { (keeps_closed, false) };
            if (closed, fill) != (path.closed, path.fill) {
                session.path_checkpoint();
            }
            execute(session, "tool.path", &json!({ "closed": closed, "fill": fill }))?;
        }
        Tool::PolygonFill | Tool::Smudge | Tool::ColorPicker | Tool::GeometryMask => {}
    }

    let mut result = json!({ "tool": tool.id(), "variant": variant.id() });
    if variant.physical() {
        result["particles"] = json!("3D view strokes only");
    }
    Ok(result)
}

fn variants(session: &Session) -> Value {
    let mut by_tool = Map::new();
    for tool in Tool::ALL {
        if let Some(variant) = ToolVariant::current(&session.tool, tool) {
            by_tool.insert(tool.id().to_string(), json!(variant.id()));
        }
    }
    Value::Object(by_tool)
}
