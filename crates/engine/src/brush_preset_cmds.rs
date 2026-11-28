//! Brush presets: the built-in ones, and the user's own saved as `.tcbrush` files (the brush
//! settings as JSON). Built-ins keep size; user presets restore size and channels.

use serde_json::{Value, json};
#[cfg(test)]
use texturecraft_core::Channel;
use texturecraft_paint::BrushSettings;
#[cfg(not(target_arch = "wasm32"))]
use texturecraft_paint::presets::with_stroke_of;
use texturecraft_paint::presets::{PRESETS, find_preset};

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

mod user_brush;
mod user_brushes;
pub use user_brush::UserBrush;
pub use user_brushes::UserBrushes;

impl Session {
    /// Use `dir` as the user brush folder: read its `*.tcbrush` files (replacing the list) and
    /// mirror later changes there. Returns a warning per file that was skipped. Files holding a
    /// bare brush (what `brush.preset.save` writes) are named after the file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_brush_presets(&mut self, dir: std::path::PathBuf) -> Vec<String> {
        use std::io::Read as _;

        let mut warnings = Vec::new();
        let mut files = std::collections::BinaryHeap::new();
        match std::fs::read_dir(&dir) {
            // No folder yet: no presets
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warnings.push(format!("{}: cannot read the brush folder: {error}", dir.display())),
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            let path = entry.path();
                            if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(BRUSH_EXT)) {
                                continue;
                            }

                            if files.len() < MAX_USER {
                                files.push(path);
                            } else if files.peek().is_some_and(|last| path < *last) {
                                files.pop();
                                files.push(path);
                            }
                        }
                        Err(error) => warnings.push(format!("{}: a file could not be read: {error}", dir.display())),
                    }
                }
            }
        }

        let mut library = UserBrushes { list: Vec::new(), dir: Some(dir) };
        for path in files.into_sorted_vec() {
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path).and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes));
            if let Err(error) = read {
                warnings.push(format!("{}: {error}", path.display()));
                continue;
            }

            if bytes.len() as u64 > MAX_BYTES {
                warnings.push(format!("{}: too large for a brush preset", path.display()));
                continue;
            }

            let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("brush");
            let parsed = serde_json::from_slice::<UserBrush>(&bytes).or_else(|_| {
                serde_json::from_slice::<BrushSettings>(&bytes).map(|brush| UserBrush { name: stem.to_string(), brush, channels: None, file: None })
            });
            match parsed {
                Ok(mut preset) => {
                    preset.name = preset.name.trim().chars().take(MAX_NAME).collect();
                    preset.brush = preset.brush.sanitized();
                    preset.file = Some(path.clone());
                    if preset.name.is_empty() || preset.name.chars().any(char::is_control) || library.taken(&preset.name, None) {
                        warnings.push(format!("{}: the name \"{}\" is empty or taken", path.display(), preset.name));
                    } else {
                        library.list.push(preset);
                    }
                }
                Err(error) => warnings.push(format!("{}: not a brush preset: {error}", path.display())),
            }
        }

        self.brush_presets = library;
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
            |session, _| Ok(list(session))
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
            "{name, overwrite?: true = replace the user preset of that name} — the current brush and painted channels; desktop and trusted CLI mirror to the data folder, browser and untrusted headless keep them until reload → {name}",
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

fn list(session: &Session) -> Value {
    let builtin_presets = PRESETS.iter().map(|preset| json!({"id": preset.id, "label": preset.label, "category": preset.category, "builtin": true}));
    let user_presets =
        session.brush_presets.list.iter().map(|preset| json!({"id": preset.name, "label": preset.name, "category": USER_CATEGORY, "builtin": false}));
    json!(builtin_presets.chain(user_presets).collect::<Vec<_>>())
}

fn apply(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.apply";
    let name = strp(params, "preset").ok_or_else(|| bad(CMD, "`preset` is required"))?;
    if let Some(preset) = find_preset(name) {
        return set(session, preset.applied(&session.tool.brush));
    }

    let user_preset = session.brush_presets.find(name).and_then(|index| session.brush_presets.list.get(index)).cloned();
    let user_preset = user_preset.ok_or_else(|| bad(CMD, format!("unknown brush preset `{name}` (see brush.preset.list)")))?;
    let mut brush = user_preset.brush.sanitized();

    // Clear an image alpha when its project image is unavailable
    if brush.alpha_image.as_ref().is_some_and(|image_name| !session.images.contains_key(image_name)) {
        brush.alpha_image = None;
    }

    if let Some(channels) = user_preset.channels {
        session.tool.enabled = channels;
    }

    set(session, brush)
}

fn name_param(params: &Value, key: &str, command: &str) -> Result<String> {
    let name = strp(params, key).map(str::trim).filter(|name| !name.is_empty()).ok_or_else(|| bad(command, format!("`{key}` is required")))?;
    if name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(bad(command, format!("`{key}` must be 1–{MAX_NAME} characters without control characters")));
    }

    Ok(name.to_string())
}

fn store(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.store";
    let name = name_param(params, "name", CMD)?;
    if find_preset(&name).is_some() {
        return Err(bad(CMD, format!("\"{name}\" is a built-in preset and can't be replaced")));
    }

    let library = &session.brush_presets;
    let existing_index = library.find(&name);
    if existing_index.is_some() && boolp(params, "overwrite") != Some(true) {
        return Err(bad(CMD, format!("a brush preset named \"{name}\" exists already (pass overwrite: true to replace it)")));
    }

    if library.taken(&name, existing_index) {
        return Err(bad(CMD, format!("\"{name}\" is too close to another preset's name")));
    }

    if existing_index.is_none() && library.list.len() >= MAX_USER {
        return Err(bad(CMD, format!("at most {MAX_USER} user brush presets")));
    }

    // Keep the stored spelling when replacing a preset
    let existing = existing_index.and_then(|index| library.list.get(index));
    let name = existing.map_or(name, |preset| preset.name.clone());
    let old_file = existing.and_then(|preset| preset.file.clone());
    let mut stored_preset = UserBrush { name: name.clone(), brush: session.tool.brush.sanitized(), channels: Some(session.tool.enabled.clone()), file: None };
    stored_preset.file = library.write(&stored_preset, old_file.as_deref()).map_err(|error| bad(CMD, error))?;

    match existing_index.and_then(|index| session.brush_presets.list.get_mut(index)) {
        Some(slot) => *slot = stored_preset,
        None => session.brush_presets.list.push(stored_preset),
    }

    Ok(json!({ "name": name }))
}

fn rename(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.rename";
    let current_name = name_param(params, "name", CMD)?;
    let new_name = name_param(params, "to", CMD)?;
    if find_preset(&current_name).is_some() {
        return Err(bad(CMD, format!("\"{current_name}\" is a built-in preset and can't be renamed")));
    }

    let library = &session.brush_presets;
    let preset_index = library.find(&current_name).ok_or_else(|| bad(CMD, format!("no user brush preset `{current_name}`")))?;
    if find_preset(&new_name).is_some() || library.taken(&new_name, Some(preset_index)) {
        return Err(bad(CMD, format!("a brush preset named \"{new_name}\" exists already")));
    }

    let Some(mut renamed_preset) = library.list.get(preset_index).cloned() else {
        return Err(bad(CMD, format!("no user brush preset `{current_name}`")));
    };

    let old_file = renamed_preset.file.clone().or_else(|| library.file_of(&renamed_preset.name));
    renamed_preset.name = new_name.clone();
    renamed_preset.file = library.write(&renamed_preset, old_file.as_deref()).map_err(|error| bad(CMD, error))?;
    if let Some(slot) = session.brush_presets.list.get_mut(preset_index) {
        *slot = renamed_preset;
    }

    Ok(json!({ "name": new_name }))
}

fn delete(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "brush.preset.delete";
    let name = name_param(params, "name", CMD)?;
    if find_preset(&name).is_some() {
        return Err(bad(CMD, format!("\"{name}\" is a built-in preset and can't be deleted")));
    }

    let preset_index = session.brush_presets.find(&name).ok_or_else(|| bad(CMD, format!("no user brush preset `{name}`")))?;
    let Some(preset) = session.brush_presets.list.get(preset_index) else {
        return Err(bad(CMD, format!("no user brush preset `{name}`")));
    };

    // The file goes first: a preset whose file stays would come back at the next start
    session.brush_presets.delete(preset).map_err(|error| bad(CMD, error))?;
    session.brush_presets.list.remove(preset_index);

    Ok(Value::Null)
}

fn set(session: &mut Session, brush: BrushSettings) -> Result<Value> {
    session.tool.brush = brush;
    session.revision = session.revision.wrapping_add(1);

    Ok(brush_json(session))
}

#[cfg(not(target_arch = "wasm32"))]
fn save(session: &mut Session, params: &Value) -> Result<Value> {
    let path = strp(params, "path").ok_or_else(|| bad("brush.preset.save", "`path` is required"))?;
    let bytes = serde_json::to_vec_pretty(&session.tool.brush).map_err(|error| bad("brush.preset.save", error.to_string()))?;

    std::fs::write(path, bytes).map_err(|error| bad("brush.preset.save", format!("cannot write {path}: {error}")))?;

    Ok(json!({ "path": path }))
}

#[cfg(not(target_arch = "wasm32"))]
fn load(session: &mut Session, params: &Value) -> Result<Value> {
    use std::io::Read as _;

    let command = "brush.preset.load";
    let path = strp(params, "path").ok_or_else(|| bad(command, "`path` is required"))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| bad(command, format!("cannot read {path}: {error}")))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(bad(command, format!("{path} is too large for a brush preset")));
    }

    let mut loaded_brush = serde_json::from_slice::<UserBrush>(&bytes)
        .map(|preset| preset.brush)
        .or_else(|_| serde_json::from_slice::<BrushSettings>(&bytes))
        .map_err(|error| bad(command, format!("{path} is not a brush preset: {error}")))?;

    // Clear an image alpha when its project image is unavailable
    if loaded_brush.alpha_image.as_ref().is_some_and(|image_name| !session.images.contains_key(image_name)) {
        loaded_brush.alpha_image = None;
    }

    set(session, with_stroke_of(&session.tool.brush, &loaded_brush))
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

    fn run(test_session: &mut Session, id: &str, params: Value) -> Result<Value> {
        crate::execute(test_session, id, &params)
    }

    fn names(test_session: &Session) -> Vec<&str> {
        test_session.brush_presets.list.iter().map(|preset| preset.name.as_str()).collect()
    }

    #[test]
    fn store_apply_overwrite_rename_and_delete() {
        let mut test_session = session();
        run(&mut test_session, "tool.brush", json!({"size": 40, "hardness": 20, "flow": 35, "spacing": 12, "jitterSize": 50})).unwrap();
        test_session.tool.enabled = [Channel::Roughness].into_iter().collect();
        assert_eq!(run(&mut test_session, "brush.preset.store", json!({"name": " Mine "})).unwrap()["name"], "Mine");
        let saved = test_session.tool.brush.clone();

        // User presets restore size and painted channels
        run(&mut test_session, "brush.preset.apply", json!({"preset": "basic_soft"})).unwrap();
        run(&mut test_session, "tool.brush", json!({"size": 5})).unwrap();
        test_session.tool.enabled = [Channel::BaseColor].into_iter().collect();
        run(&mut test_session, "brush.preset.apply", json!({"preset": "mine"})).unwrap();
        assert_eq!(test_session.tool.brush, saved);
        assert_eq!(test_session.tool.enabled, [Channel::Roughness].into_iter().collect());

        // List user presets after built-ins
        let listed = run(&mut test_session, "brush.preset.list", json!({})).unwrap();
        let last = listed.as_array().unwrap().last().unwrap().clone();
        assert_eq!((last["label"].as_str(), last["builtin"].as_bool(), last["category"].as_str()), (Some("Mine"), Some(false), Some(USER_CATEGORY)));
        assert_eq!(listed[0]["builtin"], true);

        // Replacements require overwrite and built-ins stay read-only
        assert!(run(&mut test_session, "brush.preset.store", json!({"name": "MINE"})).is_err());
        run(&mut test_session, "tool.brush", json!({"size": 70})).unwrap();
        assert_eq!(run(&mut test_session, "brush.preset.store", json!({"name": "MINE", "overwrite": true})).unwrap()["name"], "Mine");
        assert_eq!(names(&test_session), ["Mine"], "overwriting does not add a second one");
        assert!((test_session.brush_presets.list[0].brush.size - saved.size).abs() > 1e-4);

        for builtin in ["Chalk", "basic-hard"] {
            assert!(run(&mut test_session, "brush.preset.store", json!({"name": builtin, "overwrite": true})).is_err());
            assert!(run(&mut test_session, "brush.preset.rename", json!({"name": builtin, "to": "x"})).is_err());
            assert!(run(&mut test_session, "brush.preset.delete", json!({"name": builtin})).is_err());
        }

        // Renaming preserves content and removes the old name
        run(&mut test_session, "brush.preset.store", json!({"name": "Second"})).unwrap();
        assert!(run(&mut test_session, "brush.preset.rename", json!({"name": "Mine", "to": "second"})).is_err());
        assert!(run(&mut test_session, "brush.preset.rename", json!({"name": "Mine", "to": "Airbrush"})).is_err());
        assert!(run(&mut test_session, "brush.preset.rename", json!({"name": "nope", "to": "x"})).is_err());
        assert_eq!(run(&mut test_session, "brush.preset.rename", json!({"name": "mine", "to": "Renamed"})).unwrap()["name"], "Renamed");
        assert_eq!(names(&test_session), ["Renamed", "Second"]);
        assert!(run(&mut test_session, "brush.preset.apply", json!({"preset": "Mine"})).is_err());

        // Presets survive project replacement
        run(&mut test_session, "project.new", json!({"resolution": 128})).unwrap();
        assert_eq!(test_session.brush_presets.list.len(), 2);
        run(&mut test_session, "brush.preset.delete", json!({"name": "renamed"})).unwrap();
        assert_eq!(names(&test_session), ["Second"]);
        assert!(run(&mut test_session, "brush.preset.delete", json!({"name": "renamed"})).is_err());
    }

    #[test]
    fn hostile_names_are_refused() {
        let mut test_session = session();
        for name in [json!(""), json!("   "), json!("x".repeat(MAX_NAME + 1)), json!("a\nb"), json!(7), Value::Null] {
            assert!(run(&mut test_session, "brush.preset.store", json!({"name": name})).is_err());
        }

        run(&mut test_session, "brush.preset.store", json!({"name": "a/b"})).unwrap();
        assert!(run(&mut test_session, "brush.preset.store", json!({"name": "a?b"})).is_err(), "same file name as `a/b`");
        assert_eq!(names(&test_session), ["a/b"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn presets_persist_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("tc-brush-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut test_session = session();
        assert!(test_session.load_brush_presets(dir.clone()).is_empty(), "a missing folder is an empty library");
        run(&mut test_session, "tool.brush", json!({"size": 33, "hardness": 80})).unwrap();
        run(&mut test_session, "brush.preset.store", json!({"name": "Ink pen"})).unwrap();
        run(&mut test_session, "brush.preset.store", json!({"name": "Scratch", "overwrite": true})).unwrap();
        run(&mut test_session, "brush.preset.rename", json!({"name": "Scratch", "to": "Scratchy"})).unwrap();
        assert!(dir.join("Ink_pen.tcbrush").is_file() && dir.join("Scratchy.tcbrush").is_file());
        assert!(!dir.join("Scratch.tcbrush").exists(), "a rename moves the file");
        run(&mut test_session, "brush.preset.delete", json!({"name": "Scratchy"})).unwrap();
        assert!(!dir.join("Scratchy.tcbrush").exists());

        // Legacy files and invalid files share the brush folder
        run(&mut test_session, "brush.preset.save", json!({"path": dir.join("Old style.tcbrush").to_str().unwrap()})).unwrap();
        std::fs::write(dir.join("broken.tcbrush"), b"{ nope").unwrap();
        std::fs::write(dir.join("Chalk.tcbrush"), serde_json::to_vec(&BrushSettings::default()).unwrap()).unwrap();

        let mut fresh = session();
        let warnings = fresh.load_brush_presets(dir.clone());

        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(names(&fresh), ["Ink pen", "Old style"]);
        assert!((fresh.brush_presets.list[0].brush.size - test_session.tool.brush.size).abs() < 1e-6);
        assert!(fresh.brush_presets.list[0].channels.is_some() && fresh.brush_presets.list[1].channels.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn named_and_bare_files_load_the_saved_stroke_without_changing_setup() {
        let directory = std::env::temp_dir().join(format!("tc-brush-load-formats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut source = session();
        source.load_brush_presets(directory.clone());
        source.tool.brush = BrushSettings { size: 0.08, flow: 0.3, hardness: 0.7, ..BrushSettings::default() };
        source.tool.enabled = [Channel::Roughness].into_iter().collect();
        run(&mut source, "brush.preset.store", json!({"name": "Ink"})).unwrap();
        let bare_path = directory.join("bare.tcbrush");
        run(&mut source, "brush.preset.save", json!({"path": bare_path})).unwrap();

        for path in [bare_path, directory.join("Ink.tcbrush")] {
            let mut destination = session();
            destination.tool.brush.size = 0.15;
            destination.tool.enabled = [Channel::BaseColor].into_iter().collect();

            run(&mut destination, "brush.preset.load", json!({"path": path})).unwrap();

            assert_eq!(destination.tool.brush.flow, source.tool.brush.flow);
            assert_eq!(destination.tool.brush.hardness, source.tool.brush.hardness);
            assert_eq!(destination.tool.brush.size, 0.15);
            assert_eq!(destination.tool.enabled, [Channel::BaseColor].into_iter().collect());
        }

        let _ = std::fs::remove_dir_all(directory);
    }

    /// The files in a brush folder, sorted.
    fn files(dir: &std::path::Path) -> Vec<String> {
        let mut file_names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        file_names.sort();
        file_names
    }

    #[test]
    fn older_bare_files_are_renamed_replaced_and_deleted_in_place() {
        let dir = std::env::temp_dir().join(format!("tc-brush-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut test_session = session();

        // A bare brush whose file name is not what the name would give ("Old_style.tcbrush")
        run(&mut test_session, "brush.preset.save", json!({"path": dir.join("Old style.tcbrush").to_str().unwrap()})).unwrap();
        run(&mut test_session, "brush.preset.save", json!({"path": dir.join("Second one.tcbrush").to_str().unwrap()})).unwrap();
        assert!(test_session.load_brush_presets(dir.clone()).is_empty());

        run(&mut test_session, "brush.preset.delete", json!({"name": "Old style"})).unwrap();
        assert_eq!(files(&dir), ["Second one.tcbrush"], "the file it was read from is deleted");
        run(&mut test_session, "brush.preset.rename", json!({"name": "Second one", "to": "Renamed"})).unwrap();
        assert_eq!(files(&dir), ["Renamed.tcbrush"], "a rename moves the file it was read from");
        run(&mut test_session, "brush.preset.store", json!({"name": "Renamed", "overwrite": true})).unwrap();
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
        let mut test_session = session();
        assert!(test_session.load_brush_presets(dir.clone()).is_empty());
        run(&mut test_session, "brush.preset.store", json!({"name": "mine"})).unwrap();

        run(&mut test_session, "brush.preset.rename", json!({"name": "mine", "to": "Mine"})).unwrap();

        assert_eq!(files(&dir), ["Mine.tcbrush"]);
        let mut fresh = session();
        assert!(fresh.load_brush_presets(dir.clone()).is_empty());
        assert_eq!(names(&fresh), ["Mine"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_filename_owned_by_another_preset_cannot_be_overwritten() {
        let dir = std::env::temp_dir().join(format!("tc-brush-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let original = UserBrush { name: "Ink".into(), brush: BrushSettings::default(), channels: None, file: None };
        let original_bytes = serde_json::to_vec(&original).unwrap();
        let path = dir.join("Pencil.tcbrush");
        std::fs::write(&path, &original_bytes).unwrap();
        let mut session = session();
        assert!(session.load_brush_presets(dir.clone()).is_empty());

        assert!(run(&mut session, "brush.preset.store", json!({"name": "Pencil"})).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
        assert_eq!(names(&session), ["Ink"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_brush_folder_that_cannot_be_read_is_reported() {
        // A file where the folder should be: reading it as a folder fails with an error other than NotFound
        let path = std::env::temp_dir().join(format!("tc-brush-not-a-dir-{}", std::process::id()));
        std::fs::write(&path, b"not a folder").unwrap();
        let mut test_session = session();
        let warnings = test_session.load_brush_presets(path.clone());

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(test_session.brush_presets.list.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn loaded_names_with_control_characters_are_skipped() {
        let dir = std::env::temp_dir().join(format!("tc-brush-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let preset = UserBrush { name: "bad\nname".into(), brush: BrushSettings::default(), channels: None, file: None };
        std::fs::write(dir.join("bad.tcbrush"), serde_json::to_vec(&preset).unwrap()).unwrap();
        let mut session = session();
        let warnings = session.load_brush_presets(dir.clone());

        assert_eq!(warnings.len(), 1);
        assert!(session.brush_presets.list.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
