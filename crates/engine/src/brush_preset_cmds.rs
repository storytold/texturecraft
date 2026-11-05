//! Brush presets: the built-in ones, and the user's own saved as `.tcbrush` files (the brush
//! settings as JSON). Applying or loading one changes the stroke and keeps the brush's size.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use texturecraft_core::Channel;
use texturecraft_formats::export::sanitize_file_stem;
use texturecraft_paint::BrushSettings;
use texturecraft_paint::presets::{PRESETS, find_preset, with_stroke_of};

use crate::commands::{CommandSpec, always, bad, boolp, strp};
use crate::tool_cmds::brush_json;
use crate::{Result, Session, cmd};

/// File extension of a saved brush preset.
pub const BRUSH_EXT: &str = "tcbrush";
/// Largest preset file read.
#[cfg(not(target_arch = "wasm32"))]
const MAX_BYTES: u64 = 1 << 20;

/// Most user presets kept (and read from the folder).
const MAX_USER: usize = 512;
/// Longest preset name.
const MAX_NAME: usize = 60;
/// Category the user's presets are listed under.
pub const USER_CATEGORY: &str = "My brushes";

/// A brush the user saved: the whole brush (size included) and the channels it paints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserBrush {
    pub name: String,
    pub brush: BrushSettings,
    /// Channels painted when it was saved (None: leave the channels alone).
    #[serde(default)]
    pub channels: Option<BTreeSet<Channel>>,
    /// The file it was read from or last written to (None: not on disk yet). Deleting and
    /// renaming act on this file, which for an older bare `.tcbrush` is not the name's file.
    #[serde(skip)]
    pub file: Option<std::path::PathBuf>,
}

/// The user's brush presets, mirrored as `<name>.tcbrush` files in a folder on the desktop.
#[derive(Clone, Debug, Default)]
pub struct UserBrushes {
    pub list: Vec<UserBrush>,
    /// The folder mirroring `list`; None keeps the presets in memory only.
    pub dir: Option<std::path::PathBuf>,
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Two names with the same file stem would share a file.
fn file_key(name: &str) -> String {
    sanitize_file_stem(name.trim()).to_ascii_lowercase()
}

impl UserBrushes {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|p| same_name(&p.name, name))
    }

    /// `name` belongs to a built-in or to a user brush other than `skip` (or shares its file).
    fn taken(&self, name: &str, skip: Option<usize>) -> bool {
        find_preset(name).is_some()
            || self.list.iter().enumerate().any(|(i, p)| Some(i) != skip && (same_name(&p.name, name) || file_key(&p.name) == file_key(name)))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file(&self, name: &str) -> Option<std::path::PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{}.{BRUSH_EXT}", sanitize_file_stem(name.trim()))))
    }

    /// The file a preset named `name` that was never written would have.
    #[cfg(not(target_arch = "wasm32"))]
    fn file_of(&self, name: &str) -> Option<std::path::PathBuf> {
        self.file(name)
    }

    #[cfg(target_arch = "wasm32")]
    fn file_of(&self, _: &str) -> Option<std::path::PathBuf> {
        None
    }

    /// Mirror `brush` to the folder under its name's file and return that file; `old_file` is
    /// the file it had before (an overwrite or a rename), which goes once the new one is written.
    #[cfg(not(target_arch = "wasm32"))]
    fn write(&self, brush: &UserBrush, old_file: Option<&std::path::Path>) -> std::result::Result<Option<std::path::PathBuf>, String> {
        let (Some(dir), Some(path)) = (&self.dir, self.file(&brush.name)) else { return Ok(None) };
        let json = serde_json::to_vec_pretty(brush).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let old_file = old_file.filter(|old| *old != path);
        // A change of case only: on a case-insensitive file system both paths are one file, so
        // move the old file to the new spelling first instead of removing it after the write.
        let case_only = old_file.is_some_and(|old| old.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy()));
        if let Some(old) = old_file.filter(|_| case_only) {
            std::fs::rename(old, &path).map_err(|e| format!("cannot rename {}: {e}", old.display()))?;
        }
        texturecraft_formats::write_atomic(&path, &json).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        if let Some(old) = old_file.filter(|_| !case_only) {
            remove_file(old)?;
        }
        Ok(Some(path))
    }

    #[cfg(target_arch = "wasm32")]
    fn write(&self, _: &UserBrush, _: Option<&std::path::Path>) -> std::result::Result<Option<std::path::PathBuf>, String> {
        Ok(None)
    }

    /// Remove `brush`'s file (the one it was read from, else its name's file).
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, brush: &UserBrush) -> std::result::Result<(), String> {
        match brush.file.clone().or_else(|| self.file(&brush.name)) {
            Some(path) => remove_file(&path),
            None => Ok(()),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn delete(&self, _: &UserBrush) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// Remove `path`; a file that is already gone is fine, any other error is reported.
#[cfg(not(target_arch = "wasm32"))]
fn remove_file(path: &std::path::Path) -> std::result::Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("cannot remove {}: {e}", path.display())),
        _ => Ok(()),
    }
}

impl Session {
    /// Use `dir` as the user brush folder: read its `*.tcbrush` files (replacing the list) and
    /// mirror later changes there. Returns a warning per file that was skipped. Files holding a
    /// bare brush (what `brush.preset.save` writes) are named after the file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_brush_presets(&mut self, dir: std::path::PathBuf) -> Vec<String> {
        use std::io::Read as _;
        let mut warnings = Vec::new();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        match std::fs::read_dir(&dir) {
            // No folder yet: no presets
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warnings.push(format!("{}: cannot read the brush folder: {e}", dir.display())),
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => files.push(entry.path()),
                        Err(e) => warnings.push(format!("{}: a file could not be read: {e}", dir.display())),
                    }
                }
            }
        }
        files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(BRUSH_EXT)));
        files.sort();
        let mut lib = UserBrushes { list: Vec::new(), dir: Some(dir) };
        for path in files.into_iter().take(MAX_USER) {
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path).and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes));
            if let Err(e) = read {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
            if bytes.len() as u64 > MAX_BYTES {
                warnings.push(format!("{}: too large for a brush preset", path.display()));
                continue;
            }
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("brush");
            let parsed = serde_json::from_slice::<UserBrush>(&bytes).or_else(|_| {
                serde_json::from_slice::<BrushSettings>(&bytes).map(|brush| UserBrush { name: stem.to_string(), brush, channels: None, file: None })
            });
            match parsed {
                Ok(mut b) => {
                    b.name = b.name.trim().chars().take(MAX_NAME).collect();
                    b.brush = b.brush.sanitized();
                    b.file = Some(path.clone());
                    if b.name.is_empty() || lib.taken(&b.name, None) {
                        warnings.push(format!("{}: the name \"{}\" is empty or taken", path.display(), b.name));
                    } else {
                        lib.list.push(b);
                    }
                }
                Err(e) => warnings.push(format!("{}: not a brush preset: {e}", path.display())),
            }
        }
        self.brush_presets = lib;
        warnings
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "brush.preset.list",
            "List brush presets",
            [],
            None,
            "{} — the built-in presets and the user's own: id, label, category, builtin",
            always,
            |s, _| Ok(list(s))
        ),
        cmd!(
            "brush.preset.apply",
            "Apply brush preset",
            [],
            None,
            "{preset: id or label (brush.preset.list)} — a built-in sets the stroke (alpha, spacing, flow, hardness, jitters…) and keeps the size; one of the user's sets the whole brush, size included, and the painted channels",
            always,
            apply
        ),
        cmd!(
            "brush.preset.store",
            "Save current brush as a preset",
            [],
            None,
            "{name, overwrite?: true = replace the user preset of that name} — the current brush and painted channels, kept in the data folder → {name}",
            always,
            store
        ),
        cmd!(
            "brush.preset.rename",
            "Rename a brush preset",
            [],
            None,
            "{name, to} — one of the user's presets (built-ins are read-only) → {name}",
            always,
            rename
        ),
        cmd!("brush.preset.delete", "Delete a brush preset", [], None, "{name} — one of the user's presets (built-ins are read-only)", always, delete),
        cmd!("brush.preset.save", "Save brush preset", [], None, "{path} — the current brush as a .tcbrush file", always, save),
        cmd!("brush.preset.load", "Load brush preset", [], None, "{path} — a .tcbrush file's stroke on the brush, keeping the size", always, load),
    ]
}

fn list(s: &Session) -> Value {
    let builtin = PRESETS.iter().map(|p| json!({"id": p.id, "label": p.label, "category": p.category, "builtin": true}));
    let user = s.brush_presets.list.iter().map(|p| json!({"id": p.name, "label": p.name, "category": USER_CATEGORY, "builtin": false}));
    json!(builtin.chain(user).collect::<Vec<_>>())
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.apply";
    let name = strp(p, "preset").ok_or_else(|| bad(CMD, "`preset` is required"))?;
    if let Some(preset) = find_preset(name) {
        return set(s, preset.applied(&s.tool.brush));
    }
    let user = s.brush_presets.find(name).and_then(|i| s.brush_presets.list.get(i)).cloned();
    let user = user.ok_or_else(|| bad(CMD, format!("unknown brush preset `{name}` (see brush.preset.list)")))?;
    let mut brush = user.brush.sanitized();
    // An image alpha belongs to the project that saved it.
    if brush.alpha_image.as_ref().is_some_and(|n| !s.images.contains_key(n)) {
        brush.alpha_image = None;
    }
    if let Some(channels) = user.channels {
        s.tool.enabled = channels;
    }
    set(s, brush)
}

fn name_param(p: &Value, key: &str, cmd: &str) -> Result<String> {
    let name = strp(p, key).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, format!("`{key}` is required")))?;
    if name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(bad(cmd, format!("`{key}` must be 1–{MAX_NAME} characters without control characters")));
    }
    Ok(name.to_string())
}

fn store(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.store";
    let name = name_param(p, "name", CMD)?;
    if find_preset(&name).is_some() {
        return Err(bad(CMD, format!("\"{name}\" is a built-in preset and can't be replaced")));
    }
    let lib = &s.brush_presets;
    let target = lib.find(&name);
    if target.is_some() && boolp(p, "overwrite") != Some(true) {
        return Err(bad(CMD, format!("a brush preset named \"{name}\" exists already (pass overwrite: true to replace it)")));
    }
    if lib.taken(&name, target) {
        return Err(bad(CMD, format!("\"{name}\" is too close to another preset's name")));
    }
    if target.is_none() && lib.list.len() >= MAX_USER {
        return Err(bad(CMD, format!("at most {MAX_USER} user brush presets")));
    }
    // Overwriting keeps the stored spelling of the name.
    let old = target.and_then(|i| lib.list.get(i));
    let name = old.map_or(name, |old| old.name.clone());
    let old_file = old.and_then(|old| old.file.clone());
    let mut entry = UserBrush { name: name.clone(), brush: s.tool.brush.sanitized(), channels: Some(s.tool.enabled.clone()), file: None };
    entry.file = lib.write(&entry, old_file.as_deref()).map_err(|e| bad(CMD, e))?;
    match target.and_then(|i| s.brush_presets.list.get_mut(i)) {
        Some(slot) => *slot = entry,
        None => s.brush_presets.list.push(entry),
    }
    Ok(json!({ "name": name }))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.rename";
    let old = name_param(p, "name", CMD)?;
    let to = name_param(p, "to", CMD)?;
    if find_preset(&old).is_some() {
        return Err(bad(CMD, format!("\"{old}\" is a built-in preset and can't be renamed")));
    }
    let lib = &s.brush_presets;
    let i = lib.find(&old).ok_or_else(|| bad(CMD, format!("no user brush preset `{old}`")))?;
    if find_preset(&to).is_some() || lib.taken(&to, Some(i)) {
        return Err(bad(CMD, format!("a brush preset named \"{to}\" exists already")));
    }
    let Some(mut entry) = lib.list.get(i).cloned() else { return Err(bad(CMD, format!("no user brush preset `{old}`"))) };
    let old_file = entry.file.clone().or_else(|| lib.file_of(&entry.name));
    entry.name = to.clone();
    entry.file = lib.write(&entry, old_file.as_deref()).map_err(|e| bad(CMD, e))?;
    if let Some(slot) = s.brush_presets.list.get_mut(i) {
        *slot = entry;
    }
    Ok(json!({ "name": to }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.delete";
    let name = name_param(p, "name", CMD)?;
    if find_preset(&name).is_some() {
        return Err(bad(CMD, format!("\"{name}\" is a built-in preset and can't be deleted")));
    }
    let i = s.brush_presets.find(&name).ok_or_else(|| bad(CMD, format!("no user brush preset `{name}`")))?;
    let Some(entry) = s.brush_presets.list.get(i) else { return Err(bad(CMD, format!("no user brush preset `{name}`"))) };
    // The file goes first: a preset whose file stays would come back at the next start
    s.brush_presets.delete(entry).map_err(|e| bad(CMD, e))?;
    s.brush_presets.list.remove(i);
    Ok(Value::Null)
}

fn set(s: &mut Session, brush: BrushSettings) -> Result<Value> {
    s.tool.brush = brush;
    s.revision = s.revision.wrapping_add(1);
    Ok(brush_json(s))
}

#[cfg(not(target_arch = "wasm32"))]
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = strp(p, "path").ok_or_else(|| bad("brush.preset.save", "`path` is required"))?;
    let bytes = serde_json::to_vec_pretty(&s.tool.brush).map_err(|e| bad("brush.preset.save", e.to_string()))?;
    std::fs::write(path, bytes).map_err(|e| bad("brush.preset.save", format!("cannot write {path}: {e}")))?;
    Ok(json!({ "path": path }))
}

#[cfg(not(target_arch = "wasm32"))]
fn load(s: &mut Session, p: &Value) -> Result<Value> {
    use std::io::Read as _;
    let cmd = "brush.preset.load";
    let path = strp(p, "path").ok_or_else(|| bad(cmd, "`path` is required"))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes)).map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(bad(cmd, format!("{path} is too large for a brush preset")));
    }
    let mut from: BrushSettings = serde_json::from_slice(&bytes).map_err(|e| bad(cmd, format!("{path} is not a brush preset: {e}")))?;
    // An image alpha belongs to the project that saved it.
    if from.alpha_image.as_ref().is_some_and(|n| !s.images.contains_key(n)) {
        from.alpha_image = None;
    }
    set(s, with_stroke_of(&s.tool.brush, &from))
}

#[cfg(target_arch = "wasm32")]
fn save(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("brush.preset.save", "file paths are not available in the browser"))
}

#[cfg(target_arch = "wasm32")]
fn load(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("brush.preset.load", "file paths are not available in the browser"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::primitives;

    fn session() -> Session {
        Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap()
    }

    fn run(s: &mut Session, id: &str, p: Value) -> Result<Value> {
        crate::execute(s, id, &p)
    }

    fn names(s: &Session) -> Vec<&str> {
        s.brush_presets.list.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn store_apply_overwrite_rename_and_delete() {
        let mut s = session();
        run(&mut s, "tool.brush", json!({"size": 40, "hardness": 20, "flow": 35, "spacing": 12, "jitterSize": 50})).unwrap();
        s.tool.enabled = [Channel::Roughness].into_iter().collect();
        assert_eq!(run(&mut s, "brush.preset.store", json!({"name": " Mine "})).unwrap()["name"], "Mine");
        let saved = s.tool.brush.clone();

        // Applying brings back the whole brush (size too) and the channels.
        run(&mut s, "brush.preset.apply", json!({"preset": "basic_soft"})).unwrap();
        run(&mut s, "tool.brush", json!({"size": 5})).unwrap();
        s.tool.enabled = [Channel::BaseColor].into_iter().collect();
        run(&mut s, "brush.preset.apply", json!({"preset": "mine"})).unwrap();
        assert_eq!(s.tool.brush, saved);
        assert_eq!(s.tool.enabled, [Channel::Roughness].into_iter().collect());

        // Listed after the built-ins, flagged as the user's.
        let listed = run(&mut s, "brush.preset.list", json!({})).unwrap();
        let last = listed.as_array().unwrap().last().unwrap().clone();
        assert_eq!((last["label"].as_str(), last["builtin"].as_bool(), last["category"].as_str()), (Some("Mine"), Some(false), Some(USER_CATEGORY)));
        assert_eq!(listed[0]["builtin"], true);

        // The same name needs `overwrite`; a built-in name is never taken.
        assert!(run(&mut s, "brush.preset.store", json!({"name": "MINE"})).is_err());
        run(&mut s, "tool.brush", json!({"size": 70})).unwrap();
        assert_eq!(run(&mut s, "brush.preset.store", json!({"name": "MINE", "overwrite": true})).unwrap()["name"], "Mine");
        assert_eq!(names(&s), ["Mine"], "overwriting does not add a second one");
        assert!((s.brush_presets.list[0].brush.size - saved.size).abs() > 1e-4);
        for builtin in ["Chalk", "basic-hard"] {
            assert!(run(&mut s, "brush.preset.store", json!({"name": builtin, "overwrite": true})).is_err());
            assert!(run(&mut s, "brush.preset.rename", json!({"name": builtin, "to": "x"})).is_err());
            assert!(run(&mut s, "brush.preset.delete", json!({"name": builtin})).is_err());
        }

        // Rename keeps the content, refuses clashes, and the old name is gone.
        run(&mut s, "brush.preset.store", json!({"name": "Second"})).unwrap();
        assert!(run(&mut s, "brush.preset.rename", json!({"name": "Mine", "to": "second"})).is_err());
        assert!(run(&mut s, "brush.preset.rename", json!({"name": "Mine", "to": "Airbrush"})).is_err());
        assert!(run(&mut s, "brush.preset.rename", json!({"name": "nope", "to": "x"})).is_err());
        assert_eq!(run(&mut s, "brush.preset.rename", json!({"name": "mine", "to": "Renamed"})).unwrap()["name"], "Renamed");
        assert_eq!(names(&s), ["Renamed", "Second"]);
        assert!(run(&mut s, "brush.preset.apply", json!({"preset": "Mine"})).is_err());

        // Presets survive opening another project; delete removes them.
        run(&mut s, "project.new", json!({"resolution": 128})).unwrap();
        assert_eq!(s.brush_presets.list.len(), 2);
        run(&mut s, "brush.preset.delete", json!({"name": "renamed"})).unwrap();
        assert_eq!(names(&s), ["Second"]);
        assert!(run(&mut s, "brush.preset.delete", json!({"name": "renamed"})).is_err());
    }

    #[test]
    fn hostile_names_are_refused() {
        let mut s = session();
        for name in [json!(""), json!("   "), json!("x".repeat(MAX_NAME + 1)), json!("a\nb"), json!(7), Value::Null] {
            assert!(run(&mut s, "brush.preset.store", json!({"name": name})).is_err());
        }
        run(&mut s, "brush.preset.store", json!({"name": "a/b"})).unwrap();
        assert!(run(&mut s, "brush.preset.store", json!({"name": "a?b"})).is_err(), "same file name as `a/b`");
        assert_eq!(names(&s), ["a/b"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn presets_persist_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("tc-brush-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = session();
        assert!(s.load_brush_presets(dir.clone()).is_empty(), "a missing folder is an empty library");
        run(&mut s, "tool.brush", json!({"size": 33, "hardness": 80})).unwrap();
        run(&mut s, "brush.preset.store", json!({"name": "Ink pen"})).unwrap();
        run(&mut s, "brush.preset.store", json!({"name": "Scratch", "overwrite": true})).unwrap();
        run(&mut s, "brush.preset.rename", json!({"name": "Scratch", "to": "Scratchy"})).unwrap();
        assert!(dir.join("Ink_pen.tcbrush").is_file() && dir.join("Scratchy.tcbrush").is_file());
        assert!(!dir.join("Scratch.tcbrush").exists(), "a rename moves the file");
        run(&mut s, "brush.preset.delete", json!({"name": "Scratchy"})).unwrap();
        assert!(!dir.join("Scratchy.tcbrush").exists());

        // A bare brush (`brush.preset.save`), a broken file and a built-in name in the folder.
        run(&mut s, "brush.preset.save", json!({"path": dir.join("Old style.tcbrush").to_str().unwrap()})).unwrap();
        std::fs::write(dir.join("broken.tcbrush"), b"{ nope").unwrap();
        std::fs::write(dir.join("Chalk.tcbrush"), serde_json::to_vec(&BrushSettings::default()).unwrap()).unwrap();

        let mut fresh = session();
        let warnings = fresh.load_brush_presets(dir.clone());
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(names(&fresh), ["Ink pen", "Old style"]);
        assert!((fresh.brush_presets.list[0].brush.size - s.tool.brush.size).abs() < 1e-6);
        assert!(fresh.brush_presets.list[0].channels.is_some() && fresh.brush_presets.list[1].channels.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The files in a brush folder, sorted.
    fn files(dir: &std::path::Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        out.sort();
        out
    }

    #[test]
    fn older_bare_files_are_renamed_replaced_and_deleted_in_place() {
        let dir = std::env::temp_dir().join(format!("tc-brush-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = session();
        // A bare brush whose file name is not what the name would give ("Old_style.tcbrush")
        run(&mut s, "brush.preset.save", json!({"path": dir.join("Old style.tcbrush").to_str().unwrap()})).unwrap();
        run(&mut s, "brush.preset.save", json!({"path": dir.join("Second one.tcbrush").to_str().unwrap()})).unwrap();
        assert!(s.load_brush_presets(dir.clone()).is_empty());

        run(&mut s, "brush.preset.delete", json!({"name": "Old style"})).unwrap();
        assert_eq!(files(&dir), ["Second one.tcbrush"], "the file it was read from is deleted");
        run(&mut s, "brush.preset.rename", json!({"name": "Second one", "to": "Renamed"})).unwrap();
        assert_eq!(files(&dir), ["Renamed.tcbrush"], "a rename moves the file it was read from");
        run(&mut s, "brush.preset.store", json!({"name": "Renamed", "overwrite": true})).unwrap();
        assert_eq!(files(&dir), ["Renamed.tcbrush"]);

        let mut fresh = session();
        assert!(fresh.load_brush_presets(dir.clone()).is_empty(), "nothing comes back or clashes on the next start");
        assert_eq!(names(&fresh), ["Renamed"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rename_that_only_changes_case_keeps_one_file() {
        let dir = std::env::temp_dir().join(format!("tc-brush-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = session();
        assert!(s.load_brush_presets(dir.clone()).is_empty());
        run(&mut s, "brush.preset.store", json!({"name": "mine"})).unwrap();
        run(&mut s, "brush.preset.rename", json!({"name": "mine", "to": "Mine"})).unwrap();
        assert_eq!(files(&dir), ["Mine.tcbrush"]);
        let mut fresh = session();
        assert!(fresh.load_brush_presets(dir.clone()).is_empty());
        assert_eq!(names(&fresh), ["Mine"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_brush_folder_that_cannot_be_read_is_reported() {
        // A file where the folder should be: reading it as a folder fails with an error other than NotFound
        let path = std::env::temp_dir().join(format!("tc-brush-not-a-dir-{}", std::process::id()));
        std::fs::write(&path, b"not a folder").unwrap();
        let mut s = session();
        let warnings = s.load_brush_presets(path.clone());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(s.brush_presets.list.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
