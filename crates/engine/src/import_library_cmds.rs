//! The asset-library offer of a project file of the reference app: `import.library.list` says what the project brings
//! for the user's own library (its brushes, the bitmaps it uses as alphas, stencils and textures, its environment
//! map), where each came from and whether a copy may be shared; `import.library.add` puts the chosen ones into
//! TextureCraft's asset library. No window: the two import windows stay as they are, and these are commands for
//! agents, scripts and a later shelf entry. The project file is only read, and the imported document keeps its own
//! copies whatever is added.
//!
//! TextureCraft's asset library keeps the user's brush presets as `.tcbrush` files in `brushes/` of its data folder
//! (`brush.preset.save`, the shelf's "My brushes"). It has no place yet for the other kinds: alpha images, stencils,
//! textures and environment maps belong to one project there (its image resources). Those are listed with
//! `place: null` and are not added.

use serde_json::{Value, json};
use texturecraft_import::library::{BrushTip, ItemKind, ItemSource, LibraryItem, LibraryOffer, OfferedBrush};
use texturecraft_paint::{BrushAlpha, BrushSettings};

use crate::commands::{CommandSpec, always, bad, strp};
use crate::{Result, Session, cmd};

const LIST: &str = "import.library.list";
const ADD: &str = "import.library.add";
/// The folder of the data folder holding brush presets (as the desktop app keeps them).
pub const BRUSHES_FOLDER: &str = "brushes";
/// Most keys one add takes.
const MAX_KEYS: usize = 4096;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "import.library.list",
            "List a project's assets for the library",
            [],
            None,
            "{path?: project file of the reference app (native only), bytes?: the file as base64, name?: its file name (with bytes)} → {project, items: [{key, kind: brush|alpha|stencil|texture|environment, name, origin: project|shelf, from, shareable (only what the file itself stores), uses, place: where import.library.add puts it in the data folder, or null where TextureCraft's library has no place for the kind}], leftOut: [text]}. Only reads the file",
            always,
            list
        ),
        cmd!(
            "import.library.add",
            "Add a project's assets to the library",
            [],
            None,
            "{path: project file of the reference app, keys: [item keys from import.library.list], folder: TextureCraft's data folder} → {added: [{key, name, path}], notAdded: [{key, reason}]}. Native only; brushes are written as .tcbrush presets in <folder>/brushes; kinds without a place in the library are not added",
            always,
            add
        ),
    ]
}

/// Where an item goes in the data folder, `None` for kinds the library has no place for.
pub fn place(kind: ItemKind) -> Option<&'static str> {
    match kind {
        ItemKind::Brush => Some(BRUSHES_FOLDER),
        ItemKind::Alpha | ItemKind::Stencil | ItemKind::Texture | ItemKind::Environment => None,
    }
}

/// Why a kind is not added.
fn no_place(kind: ItemKind) -> String {
    format!("TextureCraft's asset library has no place for {} images yet (they belong to one project there)", kind.id())
}

fn read_offer(cmd: &str, p: &Value) -> Result<LibraryOffer> {
    if let Some(v) = p.get("bytes").filter(|v| !v.is_null()) {
        let bytes = crate::import_cmds::bytes_param(v)?;
        let name = strp(p, "name").unwrap_or("project");
        return texturecraft_import::library::offer_bytes(bytes, name).map_err(|e| bad(cmd, e.to_string()));
    }
    offer_at_path(cmd, p)
}

#[cfg(not(target_arch = "wasm32"))]
fn offer_at_path(cmd: &str, p: &Value) -> Result<LibraryOffer> {
    let path = strp(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(cmd, "give `path` (a project file) or `bytes`"))?;
    texturecraft_import::library::offer_path(std::path::Path::new(path)).map_err(|e| bad(cmd, format!("{path}: {e}")))
}

#[cfg(target_arch = "wasm32")]
fn offer_at_path(cmd: &str, _: &Value) -> Result<LibraryOffer> {
    Err(bad(cmd, "`path` is not available in the browser: give `bytes`"))
}

fn item_json(i: &LibraryItem) -> Value {
    json!({
        "key": i.key, "kind": i.kind.id(), "name": i.name, "origin": i.origin.id(), "from": i.origin.describe(),
        "shareable": i.shareable(), "uses": i.uses, "place": place(i.kind),
    })
}

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let offer = read_offer(LIST, p)?;
    Ok(json!({"project": offer.project, "items": offer.items.iter().map(item_json).collect::<Vec<_>>(), "leftOut": offer.left_out}))
}

/// TextureCraft's brush settings for an offered brush: the replay's mapping, with its tip. The size stays the default
/// (loading a preset keeps the brush's size); the flow and the opacity are the ones used most.
pub fn brush_settings(b: &OfferedBrush) -> BrushSettings {
    let (mut settings, _) = crate::import_replay::settings(&b.brush, 0.0);
    match b.tip {
        BrushTip::Round { hardness, .. } => {
            settings.alpha = BrushAlpha::Round;
            settings.hardness = hardness;
        }
    }
    settings.opacity = b.opacity;
    settings
}

#[cfg(not(target_arch = "wasm32"))]
fn add(_: &mut Session, p: &Value) -> Result<Value> {
    let keys: Vec<&str> = p
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(ADD, "`keys` must be a list of item keys (import.library.list)"))?
        .iter()
        .map(|k| k.as_str().ok_or_else(|| bad(ADD, "`keys` must hold strings")))
        .collect::<Result<_>>()?;
    if keys.len() > MAX_KEYS {
        return Err(bad(ADD, format!("at most {MAX_KEYS} keys at once")));
    }
    let folder = strp(p, "folder").filter(|f| !f.trim().is_empty()).ok_or_else(|| bad(ADD, "`folder` (TextureCraft's data folder) is required"))?;
    let offer = offer_at_path(ADD, p)?;
    let (mut added, mut not_added) = (Vec::new(), Vec::new());
    for key in keys {
        let Some(item) = offer.get(key) else {
            not_added.push(json!({"key": key, "reason": "no such item in the project (see import.library.list)"}));
            continue;
        };
        match write(item, std::path::Path::new(folder)) {
            Ok(path) => added.push(json!({"key": key, "name": item.name, "path": path.display().to_string()})),
            Err(reason) => not_added.push(json!({"key": key, "reason": reason})),
        }
    }
    Ok(json!({"added": added, "notAdded": not_added}))
}

/// Writes `item` into the library in `folder`, under a file name no other file has.
#[cfg(not(target_arch = "wasm32"))]
fn write(item: &LibraryItem, folder: &std::path::Path) -> std::result::Result<std::path::PathBuf, String> {
    let (ItemSource::Brush(b), Some(sub)) = (&item.source, place(item.kind)) else { return Err(no_place(item.kind)) };
    let dir = folder.join(sub);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let bytes = serde_json::to_vec_pretty(&brush_settings(b)).map_err(|e| e.to_string())?;
    let stem = texturecraft_formats::export::sanitize_file_stem(item.name.trim());
    let ext = crate::brush_preset_cmds::BRUSH_EXT;
    for n in 1..=1000 {
        let name = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} {n}.{ext}") };
        let path = dir.join(name);
        // create_new: never replaces a preset of the user's
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write as _;
                return f.write_all(&bytes).map(|()| path.clone()).map_err(|e| format!("cannot write {}: {e}", path.display()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", path.display())),
        }
    }
    Err(format!("{} already holds 1000 presets named {stem}", dir.display()))
}

#[cfg(target_arch = "wasm32")]
fn add(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad(ADD, "the asset library is a folder: not available in the browser"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_import::library::Origin;
    use texturecraft_import::replay::{ReplayBrush, ReplayTip, TipAlignment};

    #[test]
    fn only_brushes_have_a_place() {
        assert_eq!(place(ItemKind::Brush), Some("brushes"));
        for k in [ItemKind::Alpha, ItemKind::Stencil, ItemKind::Texture, ItemKind::Environment] {
            assert_eq!(place(k), None);
            assert!(no_place(k).contains(k.id()));
        }
    }

    fn item(kind: ItemKind, source: ItemSource) -> LibraryItem {
        LibraryItem { key: "k".into(), kind, name: "Thing brush 1 / hard".into(), origin: Origin::Project, uses: 1, source }
    }

    #[test]
    fn a_brush_is_written_as_a_preset_beside_the_users_and_never_over_one() {
        let brush = ReplayBrush {
            radius: 0.1,
            spacing: 0.15,
            flow: 0.5,
            size_pressure: None,
            flow_pressure: None,
            size_jitter: 0.0,
            flow_jitter: 0.25,
            position_jitter: 0.0,
            position_both: false,
            position_normal: false,
            rotation: 0.0,
            rotation_jitter: 0.0,
            follow: false,
            alignment: TipAlignment::Tangent,
            culling: None,
            tip: ReplayTip::Round { hardness: 0.8 },
            cycle: Vec::new(),
            cycle_drawn_start: false,
            up: None,
        };
        let offered = OfferedBrush { brush, tip: BrushTip::Round { hardness: 0.8, exact: true }, opacity: 0.7 };
        let settings = brush_settings(&offered);
        assert_eq!((settings.alpha, settings.hardness, settings.flow, settings.opacity, settings.spacing), (BrushAlpha::Round, 0.8, 0.5, 0.7, 0.15));
        let dir = std::env::temp_dir().join(format!("tc-library-add-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let it = item(ItemKind::Brush, ItemSource::Brush(offered));
        let first = write(&it, &dir).unwrap();
        let second = write(&it, &dir).unwrap();
        assert_ne!(first, second, "a second add makes a second file");
        assert_eq!(first.parent(), Some(dir.join(BRUSHES_FOLDER).as_path()));
        assert_eq!(first.extension().and_then(|e| e.to_str()), Some(crate::brush_preset_cmds::BRUSH_EXT));
        // the file loads as the brush preset it is
        let mut s = Session::new_project("t", texturecraft_geom::primitives::builtin("sphere").unwrap(), "builtin:sphere", None, 128).unwrap();
        crate::execute(&mut s, "brush.preset.load", &json!({"path": first.display().to_string()})).unwrap();
        assert_eq!((s.tool.brush.hardness, s.tool.brush.flow), (0.8, 0.5));
        let image = item(ItemKind::Texture, ItemSource::Image { url: "/Wood?version=1".into() });
        assert!(write(&image, &dir).is_err_and(|e| e.contains("no place")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_params_are_errors() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::builtin("sphere").unwrap(), "builtin:sphere", None, 128).unwrap();
        for p in [json!({}), json!({"path": "/nonexistent/x"}), json!({"bytes": "AAAA"}), json!({"bytes": [1, 2, 3]})] {
            assert!(crate::execute(&mut s, LIST, &p).is_err(), "{p}");
        }
        for p in
            [json!({}), json!({"keys": "x"}), json!({"keys": [1]}), json!({"keys": ["a"]}), json!({"keys": ["a"], "folder": "/tmp", "path": "/nonexistent/x"})]
        {
            assert!(crate::execute(&mut s, ADD, &p).is_err(), "{p}");
        }
    }
}
