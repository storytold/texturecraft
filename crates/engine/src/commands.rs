//! The command registry. Every user-visible action is a command with a stable id; the menus,
//! shortcuts, CLI, control channel and MCP server all dispatch them by id. Ids follow
//! the menu structure (see `menu_catalog`).

use serde_json::{Value, json};
use texturecraft_core::{BlendMode, Channel, NormalFormat};

use crate::{EngineError, Result, Session};

pub type Run = fn(&mut Session, &Value) -> Result<Value>;
pub type Enabled = fn(&Session) -> std::result::Result<(), String>;

pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Edit"]`. Empty = not in menus.
    pub menu: &'static [&'static str],
    /// Default shortcut in egui notation (`Ctrl+Shift+E`), mapped per platform by the UI.
    pub shortcut: Option<&'static str>,
    /// Parameters, for humans and agents.
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
}

pub(crate) fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub(crate) fn has_layer(s: &Session) -> std::result::Result<(), String> {
    s.selected_layer().map(|_| ()).ok_or_else(|| "no layer selected".into())
}

pub(crate) fn can_undo(s: &Session) -> std::result::Result<(), String> {
    if s.can_undo() { Ok(()) } else { Err("nothing to undo".into()) }
}

pub(crate) fn can_redo(s: &Session) -> std::result::Result<(), String> {
    if s.can_redo() { Ok(()) } else { Err("nothing to redo".into()) }
}

#[macro_export]
#[doc(hidden)]
macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::commands::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run }
    };
}

pub fn command_specs() -> &'static [CommandSpec] {
    static SPECS: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    SPECS.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(crate::view_cmds::edit_specs());
        v.extend(crate::layer_cmds::specs());
        v.extend(crate::layer_ops::specs());
        v.extend(crate::instance_cmds::specs());
        v.extend(crate::mask_cmds::specs());
        v.extend(crate::color_selection_cmds::specs());
        v.extend(crate::effect_clipboard::specs());
        v.extend(crate::tool_cmds::specs());
        v.extend(crate::brush_preset_cmds::specs());
        v.extend(crate::path_tool::specs());
        v.extend(crate::view_cmds::specs());
        v.extend(crate::camera_cmds::specs());
        v.extend(crate::quick_mask::specs());
        v.extend(crate::set_cmds::specs());
        v.extend(crate::meshmap_cmds::specs());
        v.extend(crate::meshmap_import_cmds::specs());
        v.extend(crate::uv_tile_cmds::specs());
        v.extend(crate::template_cmds::specs());
        v.extend(crate::project_cmds::specs());
        v.extend(crate::bake::specs());
        v.extend(crate::bake_preview::specs());
        v.extend(crate::materialx_cmds::specs());
        v.extend(crate::file_cmds::specs());
        v.extend(crate::preset_cmds::specs());
        v.extend(crate::resource_cmds::specs());
        v.extend(crate::material_lib::specs());
        v.extend(crate::link_cmds::specs());
        v.extend(crate::smart_masks::specs());
        v.extend(crate::filter_lib::specs());
        v.extend(crate::decal_cmds::specs());
        v.extend(crate::text_cmds::specs());
        v
    })
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    command_specs().iter().find(|c| c.id == id)
}

/// Run a command by id with JSON params (empty params = defaults, never a dialog). With
/// `merge: true` its edits continue the previous undo step of the same label (drags).
pub fn execute(s: &mut Session, id: &str, params: &Value) -> Result<Value> {
    let spec = find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
    (spec.enabled)(s).map_err(|why| EngineError::Disabled { cmd: id.to_string(), why })?;
    let outer = std::mem::replace(&mut s.merge_edits, boolp(params, "merge").unwrap_or(false));
    let result = (spec.run)(s, params);
    s.merge_edits = outer;
    result
}

/// Every command, as JSON (for `commands` listings and MCP).
pub fn list(s: &Session, filter: Option<&str>) -> Value {
    let f = filter.map(str::to_ascii_lowercase);
    let items: Vec<Value> = command_specs()
        .iter()
        .filter(|c| f.as_ref().is_none_or(|f| c.id.to_ascii_lowercase().contains(f) || c.label.to_ascii_lowercase().contains(f)))
        .map(|c| {
            json!({
                "id": c.id,
                "label": c.label,
                "menu": c.menu,
                "shortcut": c.shortcut,
                "params": c.params,
                "enabled": (c.enabled)(s).is_ok(),
            })
        })
        .collect();
    Value::Array(items)
}

// ---------- param helpers ----------

pub(crate) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

pub(crate) fn f32p(p: &Value, key: &str) -> Option<f32> {
    p.get(key).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite())
}

pub(crate) fn u64p(p: &Value, key: &str) -> Option<u64> {
    p.get(key).and_then(|v| v.as_u64().or_else(|| v.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f.round() as u64)))
}

pub(crate) fn boolp(p: &Value, key: &str) -> Option<bool> {
    p.get(key).and_then(Value::as_bool)
}

/// The `normalFormat` param (`openGl` or `directX`), if given.
pub(crate) fn normal_format_param(cmd: &str, p: &Value) -> Result<Option<NormalFormat>> {
    match p.get("normalFormat") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if matches!(s.as_str(), "openGl" | "opengl") => Ok(Some(NormalFormat::OpenGl)),
        Some(Value::String(s)) if matches!(s.as_str(), "directX" | "directx") => Ok(Some(NormalFormat::DirectX)),
        Some(other) => Err(bad(cmd, format!("unknown normal format {other} (openGl or directX)"))),
    }
}

pub(crate) fn strp<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}

/// A layer id param (`"layer"`), defaulting to the selected layer.
pub(crate) fn layer_param(s: &Session, cmd: &str, p: &Value) -> Result<texturecraft_core::LayerId> {
    match u64p(p, "layer") {
        Some(id) => {
            let id = texturecraft_core::LayerId(id);
            s.doc.project.layer(id).map_err(|_| bad(cmd, format!("no layer {}", id.0)))?;
            Ok(id)
        }
        None => s.selected_layer().ok_or_else(|| bad(cmd, "no layer selected and no `layer` given")),
    }
}

/// [`layer_param`], following an instance to the layer whose content it draws.
pub(crate) fn content_param(s: &Session, cmd: &str, p: &Value) -> Result<texturecraft_core::LayerId> {
    let id = layer_param(s, cmd, p)?;
    Ok(s.doc.project.content_layer(id).map_or(id, |l| l.id))
}

pub(crate) fn channel_param(cmd: &str, p: &Value, key: &str) -> Result<Option<Channel>> {
    match strp(p, key) {
        None => Ok(None),
        Some(s) => Channel::parse(s).map(Some).ok_or_else(|| bad(cmd, format!("unknown channel `{s}`"))),
    }
}

pub(crate) fn blend_param(cmd: &str, p: &Value, key: &str) -> Result<Option<BlendMode>> {
    match strp(p, key) {
        None => Ok(None),
        Some(s) => BlendMode::parse(s).map(Some).ok_or_else(|| bad(cmd, format!("unknown blend mode `{s}`"))),
    }
}

/// A color: `[r, g, b]` (linear 0..1), a grey number or `"#rrggbb"` (sRGB hex, converted to
/// linear). Absent or `null` is `None`; any other value that is not a color is an error rather than
/// the default.
pub(crate) fn color_param(cmd: &str, p: &Value, key: &str) -> Result<Option<[f32; 3]>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => color_of(v)
            .map(Some)
            .ok_or_else(|| bad(cmd, format!("`{key}` must be a color, [r, g, b] (linear 0..1), a grey number or \"#rrggbb\" (sRGB), not {v}"))),
    }
}

/// `N` finite numbers: a point or a direction (`center`, `direction`, color balance offsets), not a
/// color, so no grey or hex form. Absent or `null` is `None`; any other value is an error.
pub(crate) fn numbers_param<const N: usize>(cmd: &str, p: &Value, key: &str) -> Result<Option<[f32; N]>> {
    let v = match p.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v,
    };
    let numbers: Option<Vec<f32>> =
        v.as_array().filter(|a| a.len() == N).and_then(|a| a.iter().map(|x| x.as_f64().map(|n| n as f32).filter(|n| n.is_finite())).collect());
    match numbers.and_then(|n| <[f32; N]>::try_from(n).ok()) {
        Some(n) => Ok(Some(n)),
        None => Err(bad(cmd, format!("`{key}` must be a list of {N} numbers, not {v}"))),
    }
}

/// `color`, or `value` when there is no `color` (a fill or brush channel takes either). `value` is
/// read only then, so it cannot fail a call that has a good `color`.
pub(crate) fn color_or_value(cmd: &str, p: &Value) -> Result<Option<[f32; 3]>> {
    match color_param(cmd, p, "color")? {
        Some(color) => Ok(Some(color)),
        None => color_param(cmd, p, "value"),
    }
}

/// A color value, or `None` when `v` is not one: `[r, g, b]` numbers (linear; a longer array gives
/// its first three, so an alpha is ignored), one number for a grey, or `"#rrggbb"` (sRGB). Params
/// go through [`color_param`], which turns `None` for a given value into an error.
pub(crate) fn color_of(v: &Value) -> Option<[f32; 3]> {
    match v {
        Value::Array(a) if a.len() >= 3 => {
            let v: Vec<f32> = a.iter().take(3).map(|x| x.as_f64().map(|n| n as f32)).collect::<Option<_>>()?;
            if v.iter().all(|x| x.is_finite()) { Some([v[0], v[1], v[2]]) } else { None }
        }
        Value::Number(n) => n.as_f64().map(|v| [v as f32; 3]).filter(|v| v[0].is_finite()),
        Value::String(s) => {
            let s = s.trim_start_matches('#');
            if s.len() != 6 {
                return None;
            }
            let c = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|v| texturecraft_texel::srgb_to_linear(v as f32 / 255.0));
            Some([c(0)?, c(2)?, c(4)?])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#c8102e` in linear: 200, 16 and 46 out of 255, each through the sRGB curve.
    fn c8102e() -> [f32; 3] {
        [200.0, 16.0, 46.0].map(|v| texturecraft_texel::srgb_to_linear(v / 255.0))
    }

    #[test]
    fn a_param_that_is_not_a_color_is_an_error() {
        let rows = [
            ("absent", json!({}), Ok(None)),
            ("null", json!({"color": null}), Ok(None)),
            ("array", json!({"color": [0.5, 0.25, 1]}), Ok(Some([0.5, 0.25, 1.0]))),
            ("grey", json!({"color": 0.5}), Ok(Some([0.5; 3]))),
            ("hex", json!({"color": "#c8102e"}), Ok(Some(c8102e()))),
            ("upper case hex", json!({"color": "#C8102E"}), Ok(Some(c8102e()))),
            ("bad hex", json!({"color": "#fff"}), Err(())),
            ("short array", json!({"color": [1, 0]}), Err(())),
            ("a word for a component", json!({"color": [1, "bad", 0]}), Err(())),
            ("a null component", json!({"color": [0.5, null, 0.5]}), Err(())),
            ("object", json!({"color": {"r": 1}}), Err(())),
        ];
        for (name, params, expected) in rows {
            let got = color_param("test.cmd", &params, "color");
            match (&got, expected) {
                (Ok(color), Ok(want)) => assert_eq!(*color, want, "{name}"),
                (Err(e), Err(())) => assert!(e.to_string().contains("`color` must be a color"), "{name}: {e}"),
                _ => panic!("{name}: {got:?}"),
            }
        }
    }

    #[test]
    fn value_is_read_only_without_a_color() {
        let rows = [
            ("color", json!({"color": [0.5, 0.25, 1]}), Ok(Some([0.5, 0.25, 1.0]))),
            ("value", json!({"value": 0.5}), Ok(Some([0.5; 3]))),
            ("color wins over value", json!({"color": [1, 0, 0], "value": 0.5}), Ok(Some([1.0, 0.0, 0.0]))),
            ("good color, bad value", json!({"color": [1, 0, 0], "value": "bad"}), Ok(Some([1.0, 0.0, 0.0]))),
            ("no color, bad value", json!({"value": "bad"}), Err("`value`")),
            ("bad color, good value", json!({"color": "bad", "value": 0.5}), Err("`color`")),
            ("null color, good value", json!({"color": null, "value": 0.5}), Ok(Some([0.5; 3]))),
            ("null color, bad value", json!({"color": null, "value": "bad"}), Err("`value`")),
            ("neither", json!({}), Ok(None)),
        ];
        for (name, params, expected) in rows {
            let got = color_or_value("test.cmd", &params);
            match (&got, expected) {
                (Ok(color), Ok(want)) => assert_eq!(*color, want, "{name}"),
                (Err(e), Err(key)) => assert!(e.to_string().contains(key), "{name}: {e}"),
                _ => panic!("{name}: {got:?}"),
            }
        }
    }

    #[test]
    fn a_vector_param_is_exactly_its_numbers() {
        let rows = [
            ("absent", json!({}), Ok(None)),
            ("null", json!({"v": null}), Ok(None)),
            ("three numbers", json!({"v": [0.0, 1, -0.5]}), Ok(Some([0.0, 1.0, -0.5]))),
            ("two numbers", json!({"v": [0, 1]}), Err(())),
            ("four numbers", json!({"v": [0, 1, 0, 1]}), Err(())),
            ("a word", json!({"v": [0, "up", 0]}), Err(())),
            ("hex", json!({"v": "#ffffff"}), Err(())),
            ("one number", json!({"v": 0.5}), Err(())),
            ("too big for f32", json!({"v": [0, 1e300, 0]}), Err(())),
        ];
        for (name, params, expected) in rows {
            let got = numbers_param::<3>("test.cmd", &params, "v");
            match (&got, expected) {
                (Ok(v), Ok(want)) => assert_eq!(*v, want, "{name}"),
                (Err(e), Err(())) => assert!(e.to_string().contains("`v` must be a list of 3 numbers"), "{name}: {e}"),
                _ => panic!("{name}: {got:?}"),
            }
        }
    }
}
