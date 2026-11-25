//! Export presets: the built-in ones (read-only) and the user's, which agents and the export
//! dialog's Output templates tab list, read, save and remove. On the desktop the user presets
//! are mirrored as `<name>.json` files in a library folder (see [`Session::load_export_presets`]).

use serde_json::{Value, json};
use texturecraft_formats::export::{MAX_PRESET_NAME, sanitize_file_stem};
use texturecraft_formats::{ExportPreset, builtin_preset, builtin_presets, preset_from_json};

use crate::commands::{CommandSpec, always, bad, boolp, strp};
use crate::{Result, Session, cmd};

/// Most user presets kept (and read from the library folder).
const MAX_USER_PRESETS: usize = 512;

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
        builtin_preset(name).is_some()
            || self.list.iter().enumerate().any(|(i, p)| Some(i) != skip && (same_name(&p.name, name) || file_key(&p.name) == file_key(name)))
    }

    /// `name`, or `name 2`, `name 3`… (shortened to fit) when taken.
    fn unique_name(&self, name: &str, skip: Option<usize>) -> String {
        (1..=MAX_USER_PRESETS + 1)
            .map(|k| {
                let suffix = if k == 1 { String::new() } else { format!(" {k}") };
                let base: String = name.chars().take(MAX_PRESET_NAME.saturating_sub(suffix.len())).collect();
                format!("{}{suffix}", base.trim_end())
            })
            .find(|n| !self.taken(n, skip))
            .unwrap_or_else(|| name.to_string())
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
        builtin_preset(name).or_else(|| self.export_presets.find(name).and_then(|i| self.export_presets.list.get(i).cloned()))
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
            match texturecraft_formats::load_preset(&path).map_err(|e| e.to_string()) {
                Ok(p) if lib.taken(&p.name, None) => warnings.push(format!("{}: preset \"{}\" exists already", path.display(), p.name)),
                Ok(p) => lib.list.push(p),
                Err(e) => warnings.push(format!("{}: {e}", path.display())),
            }
        }
        self.export_presets = lib;
        warnings
    }
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let entry = |p: &ExportPreset, builtin: bool| json!({"name": p.name, "maps": p.maps.len(), "builtin": builtin});
    let builtins = builtin_presets();
    Ok(json!(builtins.iter().map(|p| entry(p, true)).chain(s.export_presets.list.iter().map(|p| entry(p, false))).collect::<Vec<_>>()))
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

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "export.preset.save";
    let mut preset = preset_param(p.get("preset").ok_or_else(|| bad(CMD, "`preset` is required"))?, CMD)?;
    let lib = &mut s.export_presets;
    let renamed = match strp(p, "rename") {
        Some(old) => Some(lib.find(old).ok_or_else(|| bad(CMD, format!("no user export preset `{old}` to rename")))?),
        None => None,
    };
    let target = renamed.or_else(|| lib.find(&preset.name));
    if boolp(p, "unique") == Some(true) {
        preset.name = lib.unique_name(&preset.name, renamed);
    } else if lib.taken(&preset.name, target) {
        return Err(bad(CMD, format!("an export preset named \"{}\" exists already (built-in presets are read-only)", preset.name)));
    }
    let target = renamed.or_else(|| lib.find(&preset.name));
    if target.is_none() && lib.list.len() >= MAX_USER_PRESETS {
        return Err(bad(CMD, format!("at most {MAX_USER_PRESETS} user export presets")));
    }
    let previous = target.and_then(|i| lib.list.get(i)).map(|old| old.name.clone());
    lib.write(&preset, previous.as_deref()).map_err(|e| bad(CMD, e))?;
    let name = preset.name.clone();
    match target.and_then(|i| lib.list.get_mut(i)) {
        Some(slot) => *slot = preset,
        None => lib.list.push(preset),
    }
    Ok(json!({ "name": name }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "export.preset.remove";
    let name = name_param(p, CMD)?;
    if builtin_preset(name).is_some() {
        return Err(bad(CMD, format!("\"{name}\" is a built-in preset and can't be removed")));
    }
    let i = s.export_presets.find(name).ok_or_else(|| bad(CMD, format!("no user export preset `{name}`")))?;
    let removed = s.export_presets.list.remove(i);
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
        // Saving the same name replaces; a clash with a built-in or with `unique` renames.
        let mut edited = preset("Mine");
        edited["maps"][0]["bit_depth"] = json!(16);
        run(&mut s, "export.preset.save", json!({"preset": edited})).unwrap();
        assert_eq!(s.export_presets.list.len(), 1);
        assert_eq!(s.export_preset("Mine").unwrap().maps[0].bit_depth, 16);
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset("Separate maps")})).is_err());
        assert_eq!(run(&mut s, "export.preset.save", json!({"preset": preset("Separate maps"), "unique": true})).unwrap()["name"], "Separate maps 2");
        assert_eq!(run(&mut s, "export.preset.save", json!({"preset": preset("mine"), "unique": true})).unwrap()["name"], "mine 2");
        // Renaming onto another preset fails; onto a free name keeps the position.
        assert!(run(&mut s, "export.preset.save", json!({"preset": preset("Mine"), "rename": "mine 2"})).is_err());
        run(&mut s, "export.preset.save", json!({"preset": preset("Renamed"), "rename": "Mine"})).unwrap();
        let names: Vec<&str> = s.export_presets.list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Renamed", "Separate maps 2", "mine 2"]);
        let listed = run(&mut s, "export.presets", json!({})).unwrap();
        assert!(listed.as_array().unwrap().iter().any(|p| p["name"] == "Renamed" && p["builtin"] == false));
        assert!(run(&mut s, "export.preset.remove", json!({"name": "Separate maps"})).is_err(), "built-ins are read-only");
        run(&mut s, "export.preset.remove", json!({"name": "renamed"})).unwrap();
        assert!(run(&mut s, "export.preset.get", json!({"name": "Renamed"})).is_err());
        // Presets survive opening another project.
        run(&mut s, "project.new", json!({"resolution": 128})).unwrap();
        assert_eq!(s.export_presets.list.len(), 2);
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

        // By name and as an inline object: the same files
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
    fn the_library_folder_mirrors_saves_renames_and_removals() {
        let dir = std::env::temp_dir().join(format!("tc_presets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.json"), "{").unwrap();
        let mut s = session();
        assert_eq!(s.load_export_presets(dir.clone()).len(), 1, "the broken file is reported");
        run(&mut s, "export.preset.save", json!({"preset": preset("My Preset")})).unwrap();
        assert!(dir.join("My_Preset.json").is_file());
        // Names sharing a file stem clash instead of overwriting each other's file.
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
