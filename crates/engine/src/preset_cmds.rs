//! Export presets: the built-in ones (read-only) and the user's, which agents and the export
//! dialog's Output templates tab list, read, save and remove. On the desktop the user presets
//! are mirrored as `<name>.json` files in a library folder (see [`Session::load_export_presets`]).

use std::borrow::Cow;
use std::collections::HashSet;

use serde_json::{Value, json};
use texturecraft_formats::export::{MAX_PRESET_NAME, sanitize_file_stem};
use texturecraft_formats::{ExportPreset, NEW_BUILTIN_PRESET_NAMES, builtin_preset, builtin_preset_catalog, preset_from_json};

use crate::commands::{CommandSpec, always, bad, strp};
use crate::{Result, Session, cmd};

/// Most user presets kept (and read from the library folder).
const MAX_USER_PRESETS: usize = 512;
const BUILTIN_PRESET_PREFIX: &str = "builtin:";

/// The user's export presets.
#[derive(Clone, Debug, Default)]
pub struct UserPresets {
    pub list: Vec<ExportPreset>,
    /// Folder mirroring `list` as `<name>.json` files (desktop); None keeps them in memory.
    pub dir: Option<std::path::PathBuf>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("export.presets", "Export presets", [], None, "{} → [{name, maps, builtin}]", always, list),
        cmd!(
            "export.preset.get",
            "Get an export preset",
            [],
            None,
            "{name} → {name, maps: [{file, format, bit_depth, channels, inverted?: [dest, ...]}]}",
            always,
            get
        ),
        cmd!(
            "export.preset.save",
            "Save a user export preset",
            [],
            None,
            "{preset: {name, maps (a map may list `inverted` destinations such as [\"R\"] to write 1 minus the value)}, unique?: true = add \" 2\", \" 3\"… on a name clash instead of failing, rename?: name of the user preset this one replaces} → {name}",
            always,
            save
        ),
        cmd!("export.preset.remove", "Remove a user export preset", [], None, "{name}", always, remove),
    ]
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// A qualified selector names a built-in independently of a saved library entry.
pub fn unqualified_preset_name(name: &str) -> &str {
    let mut name = name.trim();
    while name.get(..BUILTIN_PRESET_PREFIX.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(BUILTIN_PRESET_PREFIX)) {
        name = &name[BUILTIN_PRESET_PREFIX.len()..];
    }

    name
}

/// Library file stem: two names with the same stem would share a file.
fn file_key(name: &str) -> String {
    sanitize_file_stem(name.trim()).to_ascii_lowercase()
}

impl UserPresets {
    fn find(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|p| same_name(&p.name, name))
    }

    /// `name` belongs to a built-in or to a user preset other than `skip` (or shares its file).
    fn taken(&self, name: &str, skip: Option<usize>) -> bool {
        let keeps_name = skip.and_then(|index| self.list.get(index)).is_some_and(|preset| same_name(&preset.name, name));
        (!keeps_name && builtin_preset(unqualified_preset_name(name)).is_some()) || self.user_name_taken(name, skip)
    }

    fn user_name_taken(&self, name: &str, skip: Option<usize>) -> bool {
        let key = file_key(name);
        self.list.iter().enumerate().any(|(i, preset)| Some(i) != skip && (same_name(&preset.name, name) || file_key(&preset.name) == key))
    }

    /// Restore a saved preset under the name existing settings use.
    pub fn restore(&mut self, preset: ExportPreset) -> std::result::Result<(), String> {
        preset.validate().map_err(|error| error.to_string())?;
        let reserved = builtin_preset(&preset.name).is_some_and(|builtin| !NEW_BUILTIN_PRESET_NAMES.contains(&builtin.name.as_str()));
        if reserved || self.user_name_taken(&preset.name, None) || self.list.len() >= MAX_USER_PRESETS {
            return Err(format!("preset \"{}\" exists already or the library is full", preset.name));
        }

        self.list.push(preset);
        Ok(())
    }

    /// The first available name, adding a numbered suffix and shortening the base to fit.
    fn unique_name(&self, name: &str, skip: Option<usize>) -> Option<String> {
        // File keys also cover case-insensitive name matches
        let file_keys: HashSet<_> = self.list.iter().enumerate().filter(|(index, _)| Some(*index) != skip).map(|(_, preset)| file_key(&preset.name)).collect();

        (1..=MAX_USER_PRESETS + 1)
            .map(|number| {
                let suffix = if number == 1 { String::new() } else { format!(" {number}") };
                let base: String = name.chars().take(MAX_PRESET_NAME.saturating_sub(suffix.len())).collect();
                format!("{}{suffix}", base.trim_end())
            })
            .find(|candidate| !file_keys.contains(&file_key(candidate)) && builtin_preset(candidate).is_none())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file(&self, name: &str) -> Option<std::path::PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{}.json", sanitize_file_stem(name.trim()))))
    }

    /// Mirror a saved preset to the library folder; `replaced` is the name it had before.
    #[cfg(not(target_arch = "wasm32"))]
    fn write(&self, preset: &ExportPreset, replaced: Option<&str>) -> std::result::Result<(), String> {
        let (Some(dir), Some(path)) = (&self.dir, self.file(&preset.name)) else { return Ok(()) };
        let json = texturecraft_formats::preset_to_json(preset).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        texturecraft_formats::write_atomic(&path, json.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        if let Some(old) = replaced.filter(|old| file_key(old) != file_key(&preset.name)) {
            self.delete(old);
        }
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn write(&self, _: &ExportPreset, _: Option<&str>) -> std::result::Result<(), String> {
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, name: &str) {
        if let Some(path) = self.file(name) {
            // A file that is already gone is fine.
            let _ = std::fs::remove_file(path);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn delete(&self, _: &str) {}
}

impl Session {
    /// A built-in or user export preset by name (case-insensitive).
    pub fn export_preset(&self, name: &str) -> Option<ExportPreset> {
        self.export_presets.find(name).and_then(|index| self.export_presets.list.get(index).cloned()).or_else(|| builtin_preset(unqualified_preset_name(name)))
    }

    pub fn has_user_export_preset(&self, name: &str) -> bool {
        self.export_presets.find(name).is_some()
    }

    /// Use the familiar built-in name unless the user's library already owns that selector.
    pub fn builtin_export_preset_id<'a>(&self, name: &'a str) -> Cow<'a, str> {
        let mut id = Cow::Borrowed(name);
        while self.has_user_export_preset(&id) {
            id = Cow::Owned(format!("{BUILTIN_PRESET_PREFIX}{id}"));
        }

        id
    }

    /// Use `dir` as the user preset library: read its `*.json` presets (replacing the list) and
    /// mirror later saves there. Returns a warning per file that was skipped.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_export_presets(&mut self, dir: std::path::PathBuf) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))).collect())
            .unwrap_or_default();
        files.sort();
        let mut lib = UserPresets { list: Vec::new(), dir: Some(dir) };
        for path in files.into_iter().take(MAX_USER_PRESETS) {
            if let Err(error) = texturecraft_formats::load_preset(&path).map_err(|error| error.to_string()).and_then(|preset| lib.restore(preset)) {
                warnings.push(format!("{}: {error}", path.display()));
            }
        }
        self.export_presets = lib;
        warnings
    }
}

fn list(session: &mut Session, _: &Value) -> Result<Value> {
    let entry = |name: &str, maps: usize, builtin: bool| json!({"name": name, "maps": maps, "builtin": builtin});
    let builtins = builtin_preset_catalog().iter().map(|preset| entry(&session.builtin_export_preset_id(&preset.name), preset.maps.len(), true));
    let custom = session.export_presets.list.iter().map(|preset| entry(&preset.name, preset.maps.len(), false));

    Ok(json!(builtins.chain(custom).collect::<Vec<_>>()))
}

fn name_param<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    strp(p, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| bad(cmd, "`name` is required"))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "export.preset.get")?;
    let preset = s.export_preset(name).ok_or_else(|| bad("export.preset.get", format!("unknown export preset `{name}`")))?;
    serde_json::to_value(preset).map_err(|e| bad("export.preset.get", e.to_string()))
}

/// A preset from untrusted JSON (size, names, templates and sources checked).
pub fn preset_param(v: &Value, cmd: &str) -> Result<ExportPreset> {
    let mut preset = preset_from_json(&v.to_string()).map_err(|e| bad(cmd, e.to_string()))?;
    preset.name = preset.name.trim().to_string();
    Ok(preset)
}

fn save(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "export.preset.save";
    let preset_value = params.get("preset").ok_or_else(|| bad(CMD, "`preset` is required"))?;
    let rename_from = match params.get("rename") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name),
        Some(_) => return Err(bad(CMD, "`rename` must be a preset name")),
    };
    let unique = match params.get("unique") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(unique)) => *unique,
        Some(_) => return Err(bad(CMD, "`unique` must be a boolean")),
    };

    let mut preset = preset_param(preset_value, CMD)?;
    let library = &mut session.export_presets;
    let target_index = match rename_from {
        Some(old_name) => Some(library.find(old_name).ok_or_else(|| bad(CMD, format!("no user export preset `{old_name}` to rename")))?),
        None if unique => None,
        None => library.find(&preset.name),
    };

    if !unique && library.taken(&preset.name, target_index) {
        return Err(bad(CMD, format!("an export preset named \"{}\" exists already (built-in presets are read-only)", preset.name)));
    }

    if target_index.is_none() && library.list.len() >= MAX_USER_PRESETS {
        return Err(bad(CMD, format!("at most {MAX_USER_PRESETS} user export presets")));
    }

    if unique {
        preset.name = library.unique_name(&preset.name, target_index).ok_or_else(|| bad(CMD, "no unique export preset name is available"))?;
    }

    let previous_name = target_index.and_then(|index| library.list.get(index)).map(|previous| previous.name.as_str());
    library.write(&preset, previous_name).map_err(|error| bad(CMD, error))?;

    let response = json!({ "name": preset.name });
    match target_index.and_then(|index| library.list.get_mut(index)) {
        Some(slot) => *slot = preset,
        None => library.list.push(preset),
    }

    Ok(response)
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "export.preset.remove";
    let name = name_param(p, CMD)?;
    let Some(index) = s.export_presets.find(name) else {
        let message = if builtin_preset(unqualified_preset_name(name)).is_some() {
            format!("\"{name}\" is a built-in preset and can't be removed")
        } else {
            format!("no user export preset `{name}`")
        };
        return Err(bad(CMD, message));
    };

    let removed = s.export_presets.list.remove(index);
    s.export_presets.delete(&removed.name);
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::primitives;

    fn session() -> Session {
        Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap()
    }

    fn preset(name: &str) -> Value {
        json!({"name": name, "maps": [{"file": "$mesh_$textureSet_R", "format": "Png", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness", "component": 0}}]]}]})
    }

    fn run(s: &mut Session, id: &str, p: Value) -> Result<Value> {
        crate::execute(s, id, &p)
    }

    #[test]
    fn save_get_rename_and_remove() {
        let mut s = session();
        assert_eq!(run(&mut s, "export.preset.save", json!({"preset": preset(" Mine ")})).unwrap()["name"], "Mine");
        let got = run(&mut s, "export.preset.get", json!({"name": "mine"})).unwrap();
        assert_eq!(got, preset("Mine"), "round trip through the command");

        // A same-name save replaces the current preset
        let mut edited = preset("Mine");
        edited["maps"][0]["bit_depth"] = json!(16);
        run(&mut s, "export.preset.save", json!({"preset": edited})).unwrap();
        assert_eq!(s.export_presets.list.len(), 1);
        assert_eq!(s.export_preset("Mine").unwrap().maps[0].bit_depth, 16);

        // Built-ins need a unique name for a user copy
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset("Separate maps")})).is_err());
        assert_eq!(run(&mut s, "export.preset.save", json!({"preset": preset("Separate maps"), "unique": true})).unwrap()["name"], "Separate maps 2");
        assert_eq!(run(&mut s, "export.preset.save", json!({"preset": preset("mine"), "unique": true})).unwrap()["name"], "mine 2");

        // Renaming to a free name keeps the library position
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset("Mine"), "rename": "mine 2"})).is_err());
        run(&mut s, "export.preset.save", json!({"preset": preset("Renamed"), "rename": "Mine"})).unwrap();
        let names: Vec<&str> = s.export_presets.list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Renamed", "Separate maps 2", "mine 2"]);

        let listed = run(&mut s, "export.presets", json!({})).unwrap();
        assert!(listed.as_array().unwrap().iter().any(|p| p["name"] == "Renamed" && p["builtin"] == false));
        assert!(run(&mut s, "export.preset.remove", json!({"name": "Separate maps"})).is_err(), "built-ins are read-only");
        run(&mut s, "export.preset.remove", json!({"name": "renamed"})).unwrap();
        assert!(run(&mut s, "export.preset.get", json!({"name": "Renamed"})).is_err());

        // Presets survive opening another project
        run(&mut s, "project.new", json!({"resolution": 128})).unwrap();
        assert_eq!(s.export_presets.list.len(), 2);
    }

    #[test]
    fn malformed_save_options_do_not_replace_a_user_preset() {
        let mut session = session();
        let original = preset("Mine");
        run(&mut session, "export.preset.save", json!({"preset": original})).unwrap();
        let mut replacement = original.clone();
        replacement["maps"][0]["bit_depth"] = json!(16);

        for (key, invalid) in [("unique", json!("true")), ("rename", json!(7))] {
            let mut params = json!({"preset": replacement});
            params[key] = invalid;

            let error = run(&mut session, "export.preset.save", params).unwrap_err();

            assert_eq!(run(&mut session, "export.preset.get", json!({"name": "Mine"})).unwrap(), original);
            assert!(error.to_string().contains(&format!("`{key}`")), "{error}");
        }

        run(&mut session, "export.preset.save", json!({"preset": replacement, "unique": null, "rename": null})).unwrap();
        assert_eq!(run(&mut session, "export.preset.get", json!({"name": "Mine"})).unwrap(), replacement);
    }

    #[test]
    fn unique_saves_skip_presets_with_colliding_file_stems() {
        let mut session = Session::closed().unwrap();
        for name in ["My Preset", "My_Preset_2"] {
            run(&mut session, "export.preset.save", json!({"preset": preset(name)})).unwrap();
        }

        let saved = run(&mut session, "export.preset.save", json!({"preset": preset("My Preset"), "unique": true})).unwrap();

        assert_eq!(saved["name"], "My Preset 3");
        for name in ["My Preset", "My_Preset_2", "My Preset 3"] {
            assert_eq!(session.export_preset(name).unwrap().name, name);
        }
    }

    #[test]
    fn unique_save_at_capacity_requires_an_explicit_rename() {
        let mut session = Session::closed().unwrap();

        // At the name length limit, adding the first suffix can reproduce the requested name
        for number in 2..=MAX_USER_PRESETS + 1 {
            let suffix = format!(" {number}");
            let name = format!("{}{suffix}", "x".repeat(MAX_PRESET_NAME - suffix.len()));
            run(&mut session, "export.preset.save", json!({"preset": preset(&name)})).unwrap();
        }

        let first_name = session.export_presets.list[0].name.clone();
        let original = run(&mut session, "export.preset.get", json!({"name": first_name})).unwrap();
        let mut replacement = original.clone();
        replacement["maps"][0]["bit_depth"] = json!(16);

        let result = run(&mut session, "export.preset.save", json!({"preset": replacement, "unique": true}));

        assert_eq!(run(&mut session, "export.preset.get", json!({"name": first_name})).unwrap(), original);
        assert!(result.is_err());

        replacement["name"] = json!("Renamed");
        run(&mut session, "export.preset.save", json!({"preset": replacement, "unique": true, "rename": first_name})).unwrap();

        assert_eq!(run(&mut session, "export.preset.get", json!({"name": "Renamed"})).unwrap(), replacement);
        assert!(run(&mut session, "export.preset.get", json!({"name": first_name})).is_err());
    }

    #[test]
    fn hostile_presets_are_refused() {
        let mut s = session();
        let mut evil = preset("Evil");
        evil["maps"][0]["file"] = json!("../../$mesh");

        assert!(run(&mut s, "export.preset.save", json!({"preset": evil})).is_err());
        assert!(run(&mut s, "export.preset.save", json!({"preset": "nope"})).is_err());
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset("")})).is_err());
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset(&"x".repeat(200))})).is_err());
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset("a"), "rename": "missing"})).is_err());

        assert!(s.export_presets.list.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn exporting_with_a_user_preset_packs_channels() {
        let mut s = session();
        run(&mut s, "layer.add.fill", json!({"color": [0.9, 0.1, 0.1], "roughness": 0.3})).unwrap();
        let normal = |map: &str| {
            let slots: Vec<Value> = ["R", "G", "B"].iter().zip(0..).map(|(d, k)| json!([d, {"Virtual": {"map": map, "component": k}}])).collect();
            json!(slots)
        };
        let packed = json!({"name": "Packed", "maps": [
            {"file": "$textureSet_RMG", "format": "Png", "bit_depth": 8, "channels": [
                ["R", {"Channel": {"channel": "Roughness"}}], ["G", {"Channel": {"channel": "Metallic"}}],
                ["B", {"Virtual": {"map": "Glossiness"}}], ["A", {"Constant": 0.25}]]},
            {"file": "$textureSet_gl", "format": "Png", "bit_depth": 8, "channels": normal("NormalOpenGl")},
            {"file": "$textureSet_dx", "format": "Tga", "bit_depth": 8, "channels": normal("NormalDirectX")},
        ]});
        run(&mut s, "export.preset.save", json!({"preset": packed})).unwrap();
        s.update();
        let coverage = s.caches[0].gbuf.coverage_mask();
        let i = coverage.iter().position(|c| *c).unwrap() as u32;
        let res = s.doc.project.texture_sets[0].resolution;
        let (x, y) = (i % res, i / res);
        let comp = &s.caches[0].composite;
        let (rough, metal) = (comp.value(texturecraft_core::Channel::Roughness, x, y)[0], comp.value(texturecraft_core::Channel::Metallic, x, y)[0]);
        assert!((rough - 0.3).abs() < 0.01);
        let set = s.doc.project.texture_sets[0].name.clone();
        let dir = std::env::temp_dir().join(format!("tc_user_preset_export_{}", std::process::id()));

        // Named and inline presets produce the same files
        for preset in [json!("packed"), packed] {
            let _ = std::fs::remove_dir_all(&dir);
            let r = run(&mut s, "file.exportTextures", json!({"dir": dir.display().to_string(), "preset": preset, "ts": 0})).unwrap();
            let mut names: Vec<String> =
                r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().rsplit(['/', '\\']).next().unwrap().to_string()).collect();
            names.sort();
            assert_eq!(names, [format!("{set}_RMG.png"), format!("{set}_dx.tga"), format!("{set}_gl.png")]);
            let px = image::open(dir.join(format!("{set}_RMG.png"))).unwrap().to_rgba8().get_pixel(x, y).0;
            let byte = |v: f32| (v * 255.0).round() as i32;
            for (got, want) in px.iter().zip([byte(rough), byte(metal), byte(1.0 - rough), 64]) {
                assert!((*got as i32 - want).abs() <= 1, "{px:?}");
            }
            let gl = image::open(dir.join(format!("{set}_gl.png"))).unwrap().to_rgb8().get_pixel(x, y).0;
            let dx = image::open(dir.join(format!("{set}_dx.tga"))).unwrap().to_rgb8().get_pixel(x, y).0;
            assert_eq!((gl[0], gl[2]), (dx[0], dx[2]));
            assert!((gl[1] as i32 + dx[1] as i32 - 255).abs() <= 1, "DirectX flips green: {gl:?} {dx:?}");
        }
        assert!(run(&mut s, "file.exportTextures", json!({"dir": dir.display().to_string(), "preset": "missing"})).is_err());
        assert!(run(&mut s, "file.exportTextures", json!({"dir": dir.display().to_string(), "preset": {"name": "x", "maps": [{"file": "../x", "format": "Png", "bit_depth": 8, "channels": [["L", {"Constant": 1.0}]]}]}})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn feedback_existing_preset_keeps_its_name_and_export_maps() {
        let dir = tempfile::tempdir().unwrap();
        let name = "Roblox SurfaceAppearance (separate maps)";
        let custom = preset(name);
        std::fs::write(dir.path().join(format!("{}.json", sanitize_file_stem(name))), custom.to_string()).unwrap();
        let mut s = session();
        assert_eq!(run(&mut s, "export.preset.get", json!({"name": name})).unwrap()["maps"].as_array().unwrap().len(), 4);

        let warnings = s.load_export_presets(dir.path().to_path_buf());

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(run(&mut s, "export.preset.get", json!({"name": name.to_ascii_lowercase()})).unwrap(), custom);
        let plan = run(&mut s, "export.plan", json!({"preset": name, "ts": 0})).unwrap();

        assert_eq!(plan.as_array().unwrap().len(), 1);
        assert!(plan[0]["file"].as_str().unwrap().ends_with("_R.png"));

        let listed = run(&mut s, "export.presets", json!({})).unwrap();
        let names = listed.as_array().unwrap();
        assert!(names.iter().any(|entry| entry["name"] == name && entry["builtin"] == false));
        let builtin = format!("builtin:{name}");
        assert!(names.iter().any(|entry| entry["name"] == builtin && entry["builtin"] == true));
        let builtin_plan = run(&mut s, "export.plan", json!({"preset": builtin, "ts": 0})).unwrap();

        assert!(builtin_plan.as_array().unwrap().len() > 1);
        run(&mut s, "export.preset.save", json!({"preset": custom})).unwrap();
        run(&mut s, "export.preset.remove", json!({"name": name})).unwrap();

        assert_eq!(run(&mut s, "export.preset.get", json!({"name": name})).unwrap()["maps"].as_array().unwrap().len(), 4);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_library_folder_mirrors_saves_renames_and_removals() {
        let dir = std::env::temp_dir().join(format!("tc_presets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.json"), "{").unwrap();
        let mut s = session();
        assert_eq!(s.load_export_presets(dir.clone()).len(), 1, "the broken file is reported");
        run(&mut s, "export.preset.save", json!({"preset": preset("My Preset")})).unwrap();
        assert!(dir.join("My_Preset.json").is_file());

        // Reject aliases that would overwrite the same library file
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset("My_Preset")})).is_err());
        run(&mut s, "export.preset.save", json!({"preset": preset("Other"), "rename": "My Preset"})).unwrap();
        assert!(!dir.join("My_Preset.json").exists() && dir.join("Other.json").is_file());

        let mut fresh = session();
        assert_eq!(fresh.load_export_presets(dir.clone()).len(), 1);
        assert_eq!(fresh.export_preset("other").unwrap().name, "Other");
        run(&mut fresh, "export.preset.remove", json!({"name": "Other"})).unwrap();
        assert!(!dir.join("Other.json").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
