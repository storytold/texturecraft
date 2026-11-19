//! Project templates ("Template" in the New project dialog): preset channel lists,
//! normal format, resolution and shader settings a new project starts from. Built-in templates
//! are read-only; the user's are saved from the current project and, on the desktop, mirrored
//! as `<name>.json` files in a library folder (see [`Session::load_templates`]).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use texturecraft_core::{Channel, ChannelBlend, ChannelFormat, DisplaySettings, NormalFormat};

use crate::commands::{CommandSpec, always, bad, strp};
use crate::session::check_resolution;
use crate::{Result, Session, cmd};

/// Most user templates kept (and read from the library folder).
const MAX_USER_TEMPLATES: usize = 256;
/// Longest template name.
const MAX_NAME: usize = 128;
/// Largest template file read.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// What a new project takes from a template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectTemplate {
    pub name: String,
    /// Texture set resolution (power of two, 128..=8192).
    pub resolution: u32,
    pub normal_format: NormalFormat,
    /// Channels of every texture set, with their formats.
    pub channels: Vec<(Channel, ChannelFormat)>,
    /// Shader: PBR specular/glossiness instead of metallic/roughness.
    pub spec_gloss: bool,
    pub parallax: bool,
    pub ior: f32,
    pub scatter_color: [f32; 3],
    pub scatter_radius: f32,
}

impl Default for ProjectTemplate {
    fn default() -> Self {
        let d = DisplaySettings::default();
        ProjectTemplate {
            name: String::new(),
            resolution: 2048,
            normal_format: NormalFormat::OpenGl,
            channels: with_formats(&Channel::DEFAULT_SET),
            spec_gloss: d.spec_gloss,
            parallax: d.parallax,
            ior: d.ior,
            scatter_color: d.scatter_color,
            scatter_radius: d.scatter_radius,
        }
    }
}

/// `channels` with their default formats, in channel order (as texture sets keep them).
fn with_formats(channels: &[Channel]) -> Vec<(Channel, ChannelFormat)> {
    let mut out: Vec<_> = channels.iter().map(|c| (*c, ChannelFormat::default_for(*c))).collect();
    out.sort_by_key(|(c, _)| c.index());
    out
}

/// The template the New project dialog starts with.
pub const DEFAULT_TEMPLATE: &str = "PBR - Metallic Roughness";

/// The built-in templates.
pub fn builtin_templates() -> Vec<ProjectTemplate> {
    use Channel::*;
    let t = |name: &str, channels: &[Channel]| ProjectTemplate { name: name.into(), channels: with_formats(channels), ..ProjectTemplate::default() };
    vec![
        t(DEFAULT_TEMPLATE, &Channel::DEFAULT_SET),
        ProjectTemplate { spec_gloss: true, ..t("PBR - Specular Glossiness", &[BaseColor, Specular, Glossiness, Normal, Height]) },
        t("PBR - Metallic Roughness Alpha blend", &[BaseColor, Metallic, Roughness, Normal, Height, Opacity]),
        t("Unlit - Emissive", &[BaseColor, Emissive, Opacity]),
    ]
}

/// The user's project templates.
#[derive(Clone, Debug, Default)]
pub struct UserTemplates {
    pub list: Vec<ProjectTemplate>,
    /// Folder mirroring `list` as `<name>.json` files (desktop); None keeps them in memory.
    pub dir: Option<std::path::PathBuf>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("project.templates", "Project templates", [], None, "{} → [{name, builtin, resolution, normalFormat, channels: [ids], specGloss}]", always, list),
        cmd!(
            "project.template.save",
            "Save project settings as template",
            [],
            None,
            "{name} → {name, path?}: the current project's channels (active texture set), resolution, normal format and shader settings; replaces a user template of that name",
            always,
            save
        ),
    ]
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Checks a template from untrusted input: name, resolution, channels (base color first in
/// the list order of the channel enum, no duplicates).
fn validated(mut t: ProjectTemplate) -> std::result::Result<ProjectTemplate, String> {
    t.name = t.name.trim().to_string();
    if t.name.is_empty() || t.name.chars().count() > MAX_NAME {
        return Err(format!("a template name must be 1 to {MAX_NAME} characters"));
    }
    check_resolution(t.resolution).map_err(|e| e.to_string())?;
    if !t.channels.iter().any(|(c, _)| *c == Channel::BaseColor) {
        t.channels.push((Channel::BaseColor, ChannelFormat::default_for(Channel::BaseColor)));
    }
    t.channels.sort_by_key(|(c, _)| c.index());
    t.channels.dedup_by_key(|(c, _)| *c);
    let finite = t.ior.is_finite() && t.scatter_radius.is_finite() && t.scatter_color.iter().all(|v| v.is_finite());
    if !finite {
        return Err("template shader values must be finite".into());
    }
    Ok(t)
}

impl Session {
    /// A built-in or user project template by name (case-insensitive).
    pub fn template(&self, name: &str) -> Option<ProjectTemplate> {
        builtin_templates().into_iter().chain(self.templates.list.iter().cloned()).find(|t| same(&t.name, name))
    }

    /// Start this (new, unpainted) project from template `t`: its channels on every texture set
    /// and layer, normal format and shader settings. The resolution is the caller's choice.
    pub fn apply_template(&mut self, t: &ProjectTemplate) {
        let p = &mut self.doc.project;
        p.normal_format = t.normal_format;
        let d = &mut p.display;
        (d.spec_gloss, d.parallax, d.ior, d.scatter_color, d.scatter_radius) = (t.spec_gloss, t.parallax, t.ior, t.scatter_color, t.scatter_radius);
        for ts in 0..p.texture_sets.len() {
            if let Some(set) = p.texture_sets.get_mut(ts) {
                set.channels = t.channels.clone();
            }
            let wanted: Vec<Channel> = t.channels.iter().map(|(c, _)| *c).collect();
            for (id, _) in p.walk(ts) {
                if let Ok(l) = p.layer_mut(id) {
                    let mode = if l.is_group() { texturecraft_core::BlendMode::Passthrough } else { texturecraft_core::BlendMode::Normal };
                    l.channels.retain(|c, _| wanted.contains(c));
                    for c in &wanted {
                        l.channels.entry(*c).or_insert(ChannelBlend { mode, opacity: 1.0 });
                    }
                }
            }
        }
        self.touched();
        self.modified = false;
    }

    /// Use `dir` as the user template library: read its `*.json` templates (replacing the list)
    /// and mirror later saves there. Returns a warning per file that was skipped.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_templates(&mut self, dir: std::path::PathBuf) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))).collect())
            .unwrap_or_default();
        files.sort();
        let mut lib = UserTemplates { list: Vec::new(), dir: Some(dir) };
        for path in files.into_iter().take(MAX_USER_TEMPLATES) {
            let read = || -> std::result::Result<ProjectTemplate, String> {
                let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                if len > MAX_FILE_BYTES {
                    return Err(format!("larger than {MAX_FILE_BYTES} bytes"));
                }
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                validated(serde_json::from_slice(&bytes).map_err(|e| e.to_string())?)
            };
            match read() {
                Ok(t) if builtin_templates().iter().chain(&lib.list).any(|o| same(&o.name, &t.name)) => {
                    warnings.push(format!("{}: template \"{}\" exists already", path.display(), t.name))
                }
                Ok(t) => lib.list.push(t),
                Err(e) => warnings.push(format!("{}: {e}", path.display())),
            }
        }
        self.templates = lib;
        warnings
    }
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let entry = |t: &ProjectTemplate, builtin: bool| {
        json!({
            "name": t.name,
            "builtin": builtin,
            "resolution": t.resolution,
            "normalFormat": if t.normal_format == NormalFormat::DirectX { "directX" } else { "openGl" },
            "channels": t.channels.iter().map(|(c, _)| c.id()).collect::<Vec<_>>(),
            "specGloss": t.spec_gloss,
        })
    };
    Ok(json!(builtin_templates().iter().map(|t| entry(t, true)).chain(s.templates.list.iter().map(|t| entry(t, false))).collect::<Vec<_>>()))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "project.template.save";
    let name = strp(p, "name").unwrap_or_default();
    if builtin_templates().iter().any(|t| same(&t.name, name)) {
        return Err(bad(CMD, format!("\"{}\" is a built-in template (read-only); choose another name", name.trim())));
    }
    let project = &s.doc.project;
    let set = project.texture_set(s.ts())?;
    let d = &project.display;
    let t = ProjectTemplate {
        name: name.to_string(),
        resolution: set.resolution,
        normal_format: project.normal_format,
        channels: set.channels.clone(),
        spec_gloss: d.spec_gloss,
        parallax: d.parallax,
        ior: d.ior,
        scatter_color: d.scatter_color,
        scatter_radius: d.scatter_radius,
    };
    let t = validated(t).map_err(|e| bad(CMD, e))?;
    let lib = &mut s.templates;
    match lib.list.iter().position(|o| same(&o.name, &t.name)) {
        Some(i) => lib.list[i] = t.clone(),
        None if lib.list.len() >= MAX_USER_TEMPLATES => return Err(bad(CMD, format!("at most {MAX_USER_TEMPLATES} user templates"))),
        None => lib.list.push(t.clone()),
    }
    let path = write(lib, &t).map_err(|e| bad(CMD, e))?;
    Ok(json!({ "name": t.name, "path": path }))
}

/// Mirror template `t` to the library folder; returns the file written.
#[cfg(not(target_arch = "wasm32"))]
fn write(lib: &UserTemplates, t: &ProjectTemplate) -> std::result::Result<Option<String>, String> {
    let Some(dir) = &lib.dir else { return Ok(None) };
    let stem = texturecraft_formats::export::sanitize_file_stem(&t.name);
    // Distinct names can sanitize alike: reuse the file of this template, else take a free `_2`, `_3`... name.
    let candidate = |n: usize| if n == 1 { dir.join(format!("{stem}.json")) } else { dir.join(format!("{stem}_{n}.json")) };
    let mine = |path: &std::path::Path| {
        let ok = std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
        let owner = if ok { std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<ProjectTemplate>(&b).ok()) } else { None };
        owner.is_some_and(|o| same(&o.name, &t.name))
    };
    let path = (1..=MAX_USER_TEMPLATES + 1)
        .map(candidate)
        .find(|p| !p.exists() || mine(p))
        .ok_or_else(|| format!("too many templates share the file name `{stem}`"))?;
    let json = serde_json::to_string_pretty(t).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    texturecraft_formats::write_atomic(&path, json.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

#[cfg(target_arch = "wasm32")]
fn write(_: &UserTemplates, _: &ProjectTemplate) -> std::result::Result<Option<String>, String> {
    Ok(None)
}

/// The template named by param `template` (None when absent).
pub(crate) fn template_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<ProjectTemplate>> {
    match strp(p, "template") {
        Some(name) => s.template(name).map(Some).ok_or_else(|| bad(cmd, format!("unknown project template `{name}`"))),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute;

    #[test]
    fn new_project_from_templates() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let names: Vec<String> =
            execute(&mut s, "project.templates", &json!({})).unwrap().as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names.len(), 4);
        for name in &names {
            execute(&mut s, "project.new", &json!({"template": name, "resolution": 128})).unwrap();
            let want = s.template(name).unwrap();
            for set in &s.doc.project.texture_sets {
                assert_eq!(set.channels, want.channels, "{name}");
                assert_eq!(set.resolution, 128);
            }
            for l in s.doc.project.layers.values() {
                assert_eq!(l.channels.keys().copied().collect::<Vec<_>>(), want.channels.iter().map(|(c, _)| *c).collect::<Vec<_>>());
            }
            assert_eq!(s.doc.project.display.spec_gloss, want.spec_gloss);
            assert!(!s.modified && !s.can_undo());
        }
        // The template's resolution applies when none is given.
        execute(&mut s, "project.new", &json!({"template": "pbr - specular glossiness"})).unwrap();
        assert_eq!(s.doc.project.texture_sets[0].resolution, 2048);
        assert!(s.doc.project.display.spec_gloss);
        assert!(execute(&mut s, "project.new", &json!({"template": "nope"})).is_err());
    }

    #[test]
    fn save_current_settings_as_template_and_reload() {
        let dir = std::env::temp_dir().join(format!("tc_templates_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
        assert!(s.load_templates(dir.clone()).is_empty());
        execute(&mut s, "textureSet.channel.add", &json!({"channel": "emissive"})).unwrap();
        execute(&mut s, "textureSet.settings", &json!({"normalFormat": "directX"})).unwrap();
        assert!(execute(&mut s, "project.template.save", &json!({"name": DEFAULT_TEMPLATE})).is_err(), "built-ins are read-only");
        assert!(execute(&mut s, "project.template.save", &json!({"name": "  "})).is_err());
        let out = execute(&mut s, "project.template.save", &json!({"name": "My glow"})).unwrap();
        assert!(std::path::Path::new(out["path"].as_str().unwrap()).is_file());
        std::fs::write(dir.join("broken.json"), b"{ not json").unwrap();
        let mut fresh = Session::new_project("f", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        assert_eq!(fresh.load_templates(dir.clone()).len(), 1, "the broken file is reported");
        execute(&mut fresh, "project.new", &json!({"template": "my glow"})).unwrap();
        let set = &fresh.doc.project.texture_sets[0];
        assert!(set.has_channel(Channel::Emissive) && set.resolution == 256);
        assert_eq!(fresh.doc.project.normal_format, NormalFormat::DirectX);
        // Templates survive opening another project.
        execute(&mut fresh, "project.new", &json!({})).unwrap();
        assert!(fresh.template("My glow").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn colliding_template_files() {
        let dir = std::env::temp_dir().join(format!("tc_templates_collide_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
        assert!(s.load_templates(dir.clone()).is_empty());
        execute(&mut s, "project.template.save", &json!({"name": "My glow"})).unwrap();
        execute(&mut s, "textureSet.channel.add", &json!({"channel": "emissive"})).unwrap();
        execute(&mut s, "project.template.save", &json!({"name": "My_glow"})).unwrap();
        // Saving again replaces in place and keeps the file.
        execute(&mut s, "project.template.save", &json!({"name": "my_glow"})).unwrap();
        let mut fresh = Session::new_project("f", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        assert!(fresh.load_templates(dir.clone()).is_empty());
        assert_eq!(fresh.templates.list.len(), 2, "templates after restart");
        let glow = fresh.template("My glow").expect("My glow survives");
        let under = fresh.template("My_glow").expect("My_glow survives");
        assert!(!glow.channels.iter().any(|(c, _)| *c == Channel::Emissive), "My glow keeps its original channels");
        assert!(under.channels.iter().any(|(c, _)| *c == Channel::Emissive), "My_glow keeps the emissive channel");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "re-saving replaces the file in place");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
