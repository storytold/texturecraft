//! The project's colour swatches: the row under the colour picker, saved with the project

use serde_json::{Value, json};
use texturecraft_core::MAX_SWATCHES;

use crate::commands::{CommandSpec, always, bad, color_param, u64p};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("swatch.list", "Swatches", [], None, "{}", always, |session, _| Ok(list(session))),
        cmd!("swatch.add", "Add swatch", [], None, "{color: [r, g, b] (linear 0..1) | \"#rrggbb\"}", always, add),
        cmd!("swatch.remove", "Remove swatch", [], None, "{index}", has_swatches, remove),
    ]
}

fn has_swatches(session: &Session) -> std::result::Result<(), String> {
    if session.doc.project.swatches.is_empty() { Err("the project has no swatches".into()) } else { Ok(()) }
}

fn list(session: &Session) -> Value {
    json!({ "swatches": session.doc.project.swatches })
}

/// Adds `color` (clamped to 0..1) after the project's last swatch
fn add(session: &mut Session, params: &Value) -> Result<Value> {
    let color = color_param("swatch.add", params, "color")?.ok_or_else(|| bad("swatch.add", "`color` is required: [r, g, b] or \"#rrggbb\""))?;
    if session.doc.project.swatches.len() >= MAX_SWATCHES {
        return Err(bad("swatch.add", format!("a project keeps at most {MAX_SWATCHES} swatches; remove one first")));
    }

    let color = color.map(|part| part.clamp(0.0, 1.0));
    session.edit("Add swatch", |session| {
        session.doc.project.swatches.push(color);
        Ok(json!({ "index": session.doc.project.swatches.len() - 1, "color": color }))
    })
}

fn remove(session: &mut Session, params: &Value) -> Result<Value> {
    let index = u64p(params, "index")
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < session.doc.project.swatches.len())
        .ok_or_else(|| bad("swatch.remove", "`index` must name a swatch (see swatch.list)"))?;
    session.edit("Remove swatch", |session| {
        let color = session.doc.project.swatches.remove(index);
        Ok(json!({ "removed": color }))
    })
}
