//! Colour swatches: the project's own (saved in the `.tcraft`), the user's global ones (kept in the
//! app's preferences by the UI) and the last colours painted with. Colours are linear in the
//! document and in the commands; `hex` fields are the sRGB form people see.

use serde_json::{Value, json};
use texturecraft_core::Channel;
use texturecraft_core::swatch::{self, MAX_RECENT, MAX_SWATCHES, Swatch};

use crate::commands::{CommandSpec, always, bad, color_param, strp, u64p};
use crate::{Result, Session, cmd};

/// Swatches that are not part of the project: the user's global ones and the recent colours.
/// They stay when another project opens; the UI keeps them in the preferences.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwatchBook {
    pub global: Vec<Swatch>,
    /// Linear colours, newest first, at most [`MAX_RECENT`].
    pub recent: Vec<[f32; 3]>,
}

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Project,
    Global,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "swatch.list",
            "List colour swatches",
            [],
            None,
            "{} — project swatches, global swatches and the recent colours: {index, name, hex (sRGB), color (linear)}",
            always,
            |s, _| Ok(list(s))
        ),
        cmd!(
            "swatch.add",
            "Add colour swatch",
            [],
            None,
            "{scope?: project|global (default project), color?: linear [r,g,b] or \"#rrggbb\" (default: the brush's base color), name?}",
            always,
            add
        ),
        cmd!(
            "swatch.apply",
            "Apply colour swatch",
            [],
            None,
            "{scope?: project|global, index?|name?  (or recent?: index in the recent row), target?: brush (default) | layer (the base color of fill `layer`, default the selected one, or of a content fill `effect`)}",
            always,
            apply
        ),
        cmd!("swatch.rename", "Rename colour swatch", [], None, "{scope?, index, newName}", always, rename),
        cmd!("swatch.replace", "Replace colour swatch", [], None, "{scope?, index?|name?, color?: default the brush's base color}", always, replace),
        cmd!("swatch.remove", "Delete colour swatch", [], None, "{scope?, index?|name?}", always, remove),
        cmd!("swatch.move", "Reorder colour swatch", [], None, "{scope?, from: index, to: index}", always, move_to),
        cmd!(
            "swatch.import",
            "Import colour palette",
            [],
            None,
            "{path (desktop) | text, scope?, mode?: append (default) | replace} — a GIMP palette (.gpl) or a hex list (#rrggbb [name] per line)",
            always,
            import
        ),
        cmd!(
            "swatch.export",
            "Export colour palette",
            [],
            None,
            "{scope?, format?: gpl | hex (default: from the path's extension, else gpl), path? (desktop)} — returns {text, count}",
            always,
            export
        ),
    ]
}

fn scope(cmd: &str, p: &Value) -> Result<Scope> {
    match strp(p, "scope") {
        None | Some("project") => Ok(Scope::Project),
        Some("global") => Ok(Scope::Global),
        Some(other) => Err(bad(cmd, format!("unknown scope `{other}` (project or global)"))),
    }
}

fn items(s: &Session, scope: Scope) -> &[Swatch] {
    match scope {
        Scope::Project => &s.doc.project.swatches,
        Scope::Global => &s.swatches.global,
    }
}

/// Change a list: a project list as one undo step, the global list directly.
fn edit_list<T>(s: &mut Session, scope: Scope, label: &str, f: impl FnOnce(&mut Vec<Swatch>) -> Result<T>) -> Result<T> {
    match scope {
        Scope::Project => s.edit(label, |s| f(&mut s.doc.project.swatches)),
        Scope::Global => {
            let mut next = s.swatches.global.clone();
            let out = f(&mut next)?;
            s.swatches.global = next;
            s.revision = s.revision.wrapping_add(1);
            Ok(out)
        }
    }
}

fn sw_json(i: usize, s: &Swatch) -> Value {
    json!({ "index": i, "name": s.name, "hex": s.hex(), "color": s.rgb })
}

fn list(s: &Session) -> Value {
    json!({
        "project": s.doc.project.swatches.iter().enumerate().map(|(i, x)| sw_json(i, x)).collect::<Vec<_>>(),
        "global": s.swatches.global.iter().enumerate().map(|(i, x)| sw_json(i, x)).collect::<Vec<_>>(),
        "recent": s.swatches.recent.iter().enumerate().map(|(i, c)| json!({ "index": i, "hex": swatch::to_hex(*c), "color": c })).collect::<Vec<_>>(),
    })
}

/// The brush's base colour.
pub fn current_colour(s: &Session) -> [f32; 3] {
    s.tool.values.get(&Channel::BaseColor).copied().unwrap_or_else(|| Channel::BaseColor.default_value())
}

/// The `color` param, or the brush's colour.
fn colour_param(s: &Session, cmd: &str, p: &Value) -> Result<[f32; 3]> {
    match p.get("color") {
        None | Some(Value::Null) => Ok(current_colour(s)),
        Some(_) => color_param(p, "color").map(swatch::sanitize).ok_or_else(|| bad(cmd, "`color` must be [r,g,b] (linear) or \"#rrggbb\"")),
    }
}

/// The swatch an `index` or `name` param points at.
fn find(s: &Session, cmd: &str, scope: Scope, p: &Value) -> Result<usize> {
    let list = items(s, scope);
    if let Some(name) = strp(p, "name") {
        let wanted = swatch::clean_name(name).to_lowercase();
        return list.iter().position(|x| x.name.to_lowercase() == wanted).ok_or_else(|| bad(cmd, format!("no swatch named `{name}`")));
    }
    let i = u64p(p, "index").ok_or_else(|| bad(cmd, "give `index` or `name`"))?;
    usize::try_from(i).ok().filter(|i| *i < list.len()).ok_or_else(|| bad(cmd, format!("no swatch {i} ({} swatches)", list.len())))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.add";
    let scope = scope(cmd, p)?;
    let colour = colour_param(s, cmd, p)?;
    let name = strp(p, "name").unwrap_or("");
    let at = edit_list(s, scope, "Add swatch", |l| {
        if l.len() >= MAX_SWATCHES {
            return Err(bad(cmd, format!("a palette holds at most {MAX_SWATCHES} swatches")));
        }
        l.push(Swatch::new(name, colour));
        Ok(l.len() - 1)
    })?;
    Ok(json!({ "index": at, "hex": swatch::to_hex(colour) }))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.apply";
    let colour = if let Some(i) = u64p(p, "recent") {
        usize::try_from(i).ok().and_then(|i| s.swatches.recent.get(i)).copied().ok_or_else(|| bad(cmd, format!("no recent colour {i}")))?
    } else {
        let scope = scope(cmd, p)?;
        let i = find(s, cmd, scope, p)?;
        items(s, scope).get(i).map(|x| x.rgb).ok_or_else(|| bad(cmd, "no such swatch"))?
    };
    match strp(p, "target").unwrap_or("brush") {
        "brush" => {
            s.tool.values.insert(Channel::BaseColor, colour);
            s.tool.enabled.insert(Channel::BaseColor);
            s.revision = s.revision.wrapping_add(1);
        }
        "layer" => {
            let mut params = json!({ "channel": Channel::BaseColor.id(), "color": colour });
            // The fill is the `layer` (default: the selected one) or a content fill `effect`.
            if let Some(o) = params.as_object_mut() {
                for key in ["layer", "effect"] {
                    if let Some(v) = p.get(key) {
                        o.insert(key.into(), v.clone());
                    }
                }
            }
            crate::execute(s, "layer.fill.set", &params)?;
        }
        other => return Err(bad(cmd, format!("unknown target `{other}` (brush or layer)"))),
    }
    note_used(s, colour);
    Ok(json!({ "hex": swatch::to_hex(colour), "color": colour }))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.rename";
    let scope = scope(cmd, p)?;
    let i = find(s, cmd, scope, &json!({ "index": p.get("index").cloned().unwrap_or(Value::Null) }))?;
    let name = strp(p, "newName").ok_or_else(|| bad(cmd, "`newName` is required"))?;
    edit_list(s, scope, "Rename swatch", |l| {
        if let Some(x) = l.get_mut(i) {
            x.name = swatch::clean_name(name);
        }
        Ok(())
    })?;
    Ok(list(s))
}

fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.replace";
    let scope = scope(cmd, p)?;
    let i = find(s, cmd, scope, p)?;
    let colour = colour_param(s, cmd, p)?;
    edit_list(s, scope, "Replace swatch", |l| {
        if let Some(x) = l.get_mut(i) {
            x.rgb = colour;
        }
        Ok(())
    })?;
    Ok(sw_json(i, &Swatch::new("", colour)))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.remove";
    let scope = scope(cmd, p)?;
    let i = find(s, cmd, scope, p)?;
    edit_list(s, scope, "Delete swatch", |l| {
        if i < l.len() {
            l.remove(i);
        }
        Ok(())
    })?;
    Ok(json!({ "count": items(s, scope).len() }))
}

fn move_to(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.move";
    let scope = scope(cmd, p)?;
    let from = find(s, cmd, scope, &json!({ "index": p.get("from").cloned().unwrap_or(Value::Null) }))?;
    let to = u64p(p, "to").ok_or_else(|| bad(cmd, "`to` is required"))?;
    let to = usize::try_from(to).unwrap_or(usize::MAX);
    edit_list(s, scope, "Reorder swatches", |l| {
        swatch::move_swatch(l, from, to);
        Ok(())
    })?;
    Ok(list(s))
}

/// Read palette text, from the `text` param or (desktop) the file at `path`.
fn palette_text(cmd: &str, p: &Value) -> Result<String> {
    if let Some(t) = strp(p, "text") {
        return Ok(t.to_string());
    }
    let path = strp(p, "path").ok_or_else(|| bad(cmd, "give `path` or `text`"))?;
    read_file(cmd, path)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(cmd: &str, path: &str) -> Result<String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(swatch::MAX_PALETTE_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))?;
    if bytes.len() > swatch::MAX_PALETTE_BYTES {
        return Err(bad(cmd, format!("{path} is too large for a palette")));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(target_arch = "wasm32")]
fn read_file(cmd: &str, _: &str) -> Result<String> {
    Err(bad(cmd, "file paths are not available in the browser; give `text`"))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.import";
    let scope = scope(cmd, p)?;
    let text = palette_text(cmd, p)?;
    let replace = match strp(p, "mode") {
        None | Some("append") => false,
        Some("replace") => true,
        Some(other) => return Err(bad(cmd, format!("unknown mode `{other}` (append or replace)"))),
    };
    let (read, skipped) = if text.trim_start_matches('\u{feff}').trim_start().starts_with("GIMP Palette") {
        let (_, l, skipped) = swatch::parse_gpl(&text).map_err(|e| bad(cmd, e))?;
        (l, skipped)
    } else {
        swatch::parse_hex_list(&text).map_err(|e| bad(cmd, e))?
    };
    if read.is_empty() {
        return Err(bad(cmd, format!("no colours found ({skipped} unreadable lines)")));
    }
    let added = edit_list(s, scope, "Import palette", |l| {
        if replace {
            l.clear();
        }
        let room = MAX_SWATCHES.saturating_sub(l.len());
        let n = read.len().min(room);
        l.extend(read.into_iter().take(n));
        Ok(n)
    })?;
    Ok(json!({ "added": added, "skipped": skipped, "count": items(s, scope).len() }))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "swatch.export";
    let scope = scope(cmd, p)?;
    let path = strp(p, "path");
    let hex = match strp(p, "format") {
        Some("gpl") => false,
        Some("hex") => true,
        Some(other) => return Err(bad(cmd, format!("unknown format `{other}` (gpl or hex)"))),
        None => path.is_some_and(|p| p.to_ascii_lowercase().ends_with(".txt") || p.to_ascii_lowercase().ends_with(".hex")),
    };
    let list = items(s, scope);
    let name = if scope == Scope::Project { s.doc.project.name.as_str() } else { "TextureCraft" };
    let text = if hex { swatch::write_hex_list(list) } else { swatch::write_gpl(name, list) };
    if let Some(path) = path {
        write_file(cmd, path, &text)?;
    }
    Ok(json!({ "text": text, "count": list.len(), "path": path }))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(cmd: &str, path: &str, text: &str) -> Result<()> {
    std::fs::write(path, text).map_err(|e| bad(cmd, format!("cannot write {path}: {e}")))
}

#[cfg(target_arch = "wasm32")]
fn write_file(cmd: &str, _: &str, _: &str) -> Result<()> {
    Err(bad(cmd, "file paths are not available in the browser; use the returned `text`"))
}

/// Remember `colour` as just used.
pub(crate) fn note_used(s: &mut Session, colour: [f32; 3]) {
    swatch::push_recent(&mut s.swatches.recent, colour);
    s.swatches.recent.truncate(MAX_RECENT);
}
