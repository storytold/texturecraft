//! Project templates ("Template" in the New project dialog): preset channel lists,
//! normal format, resolution and shader settings a new project starts from. Built-in templates
//! are read-only; the user's are saved from the current project and, on the desktop, mirrored
//! as JSON files in a library folder (see [`Session::load_templates`]).

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

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

/// How the New project list names the default template
pub const DEFAULT_LABEL: &str = "Default (OpenPBR)";
/// Appended to the other built-in templates in the list
pub const BUILTIN_SUFFIX: &str = " (starter_assets)";
/// Appended to the user's templates in the list
pub const USER_SUFFIX: &str = " (your_assets)";

/// The name the New project list shows for a template: "Default (OpenPBR)" for the default,
/// the other built-ins with " (starter_assets)", the user's with " (your_assets)".
pub fn template_label(name: &str, builtin: bool) -> String {
    let (name, suffix) = template_label_parts(name, builtin);
    format!("{name}{suffix}")
}

fn template_label_parts(name: &str, builtin: bool) -> (&str, &str) {
    match (builtin, name == DEFAULT_TEMPLATE) {
        (true, true) => (DEFAULT_LABEL, ""),
        (true, false) => (name, BUILTIN_SUFFIX),
        (false, _) => (name, USER_SUFFIX),
    }
}

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
    /// Folder for persistent desktop templates. Without it, pathless saves stay in memory.
    pub dir: Option<std::path::PathBuf>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "project.templates",
            "Project templates",
            [],
            None,
            "{} → [{name, label, builtin, resolution, normalFormat, channels: [ids], specGloss}]",
            always,
            list
        ),
        cmd!(
            "project.template.save",
            "Save project settings as template",
            [],
            None,
            "{name, path? (desktop)} → {name, path?}: save the active set's channels and resolution, normal format and shader settings; replace a user template of that name",
            always,
            save
        ),
    ]
}

fn same_template_key(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn matches_template_label(key: &str, name: &str, builtin: bool) -> bool {
    let (name, suffix) = template_label_parts(name, builtin);
    let name = name.trim_start();
    let suffix = if name.is_empty() { suffix.trim_start() } else { suffix };

    key.trim().split_at_checked(name.len()).is_some_and(|(prefix, ending)| prefix.eq_ignore_ascii_case(name) && ending.eq_ignore_ascii_case(suffix))
}

fn template_keys_overlap(name: &str, label: &str, other: &ProjectTemplate, builtin: bool) -> bool {
    same_template_key(name, &other.name)
        || same_template_key(label, &other.name)
        || matches_template_label(name, &other.name, builtin)
        || matches_template_label(label, &other.name, builtin)
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
    /// Resolve a canonical template name first, then an unambiguous New project display label.
    /// Both lookups ignore ASCII case and surrounding whitespace.
    pub fn template(&self, name: &str) -> Option<ProjectTemplate> {
        let builtins = builtin_templates();
        let templates = || builtins.iter().map(|template| (template, true)).chain(self.templates.list.iter().map(|template| (template, false)));
        if let Some((template, _)) = templates().find(|(template, _)| same_template_key(&template.name, name)) {
            return Some(template.clone());
        }

        let mut matches = templates().filter(|(template, builtin)| matches_template_label(name, &template.name, *builtin));
        let (template, _) = matches.next()?;
        matches.next().is_none().then(|| template.clone())
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
    pub fn load_templates(&mut self, dir: PathBuf) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).filter(|path| is_template_file(path)).collect())
            .unwrap_or_default();
        files.sort();

        let builtins = builtin_templates();
        let mut library = UserTemplates { list: Vec::new(), dir: Some(dir) };
        let mut template_files = std::collections::BTreeMap::new();
        for path in files.into_iter().take(MAX_USER_TEMPLATES) {
            let read = || -> std::result::Result<ProjectTemplate, String> {
                let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                if len > MAX_FILE_BYTES {
                    return Err(format!("larger than {MAX_FILE_BYTES} bytes"));
                }

                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                validated(serde_json::from_slice(&bytes).map_err(|e| e.to_string())?)
            };

            let template = match read() {
                Ok(template) => template,
                Err(error) => {
                    warnings.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };

            // Reserve both keys so a later file cannot shadow an accepted template
            let label = template_label(&template.name, false);
            if builtins.iter().any(|other| template_keys_overlap(&template.name, &label, other, true))
                || library.list.iter().any(|other| template_keys_overlap(&template.name, &label, other, false))
            {
                warnings.push(format!("{}: template \"{}\" conflicts with an existing name or display label", path.display(), template.name));
                continue;
            }

            template_files.insert(template.name.to_ascii_lowercase(), path);
            library.list.push(template);
        }

        self.templates = library;
        self.template_files = template_files;

        warnings
    }
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let entry = |t: &ProjectTemplate, builtin: bool| {
        json!({
            "name": t.name,
            "label": template_label(&t.name, builtin),
            "builtin": builtin,
            "resolution": t.resolution,
            "normalFormat": if t.normal_format == NormalFormat::DirectX { "directX" } else { "openGl" },
            "channels": t.channels.iter().map(|(c, _)| c.id()).collect::<Vec<_>>(),
            "specGloss": t.spec_gloss,
        })
    };
    Ok(json!(builtin_templates().iter().map(|t| entry(t, true)).chain(s.templates.list.iter().map(|t| entry(t, false))).collect::<Vec<_>>()))
}

fn save(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "project.template.save";
    let name = strp(params, "name").unwrap_or_default();
    let project = &session.doc.project;
    let set = project.texture_set(session.ts())?;
    let display = &project.display;
    let template = ProjectTemplate {
        name: name.to_string(),
        resolution: set.resolution,
        normal_format: project.normal_format,
        channels: set.channels.clone(),
        spec_gloss: display.spec_gloss,
        parallax: display.parallax,
        ior: display.ior,
        scatter_color: display.scatter_color,
        scatter_radius: display.scatter_radius,
    };

    let template = validated(template).map_err(|error| bad(CMD, error))?;
    let label = template_label(&template.name, false);
    if builtin_templates().iter().any(|other| template_keys_overlap(&template.name, &label, other, true)) {
        return Err(bad(CMD, format!("\"{}\" is a built-in template (read-only); choose another name", template.name)));
    }

    // Only a canonical name match replaces an existing user template
    let existing = session.templates.list.iter().position(|entry| same_template_key(&entry.name, &template.name));
    if let Some((_, other)) =
        session.templates.list.iter().enumerate().find(|(index, other)| Some(*index) != existing && template_keys_overlap(&template.name, &label, other, false))
    {
        return Err(bad(CMD, format!("template \"{}\" conflicts with the name or display label of \"{}\"", template.name, other.name)));
    }

    if existing.is_none() && session.templates.list.len() >= MAX_USER_TEMPLATES {
        return Err(bad(CMD, format!("at most {MAX_USER_TEMPLATES} user templates")));
    }

    let path = write(session, &template, strp(params, "path")).map_err(|error| bad(CMD, error))?;
    let reply = json!({ "name": template.name, "path": path });
    if let Some(index) = existing {
        session.templates.list[index] = template;
    } else {
        session.templates.list.push(template);
    }

    Ok(reply)
}

#[cfg(not(target_arch = "wasm32"))]
fn is_template_file(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

#[cfg(not(target_arch = "wasm32"))]
fn same_directory(parent: &Path, directory: &Path) -> bool {
    // Resolve aliases on the folder without following a link at the template file
    parent == directory
        || std::fs::canonicalize(parent.join(".")).is_ok_and(|parent| std::fs::canonicalize(directory.join(".")).is_ok_and(|directory| parent == directory))
}

/// Write the chosen file, or reuse the configured library's backing file on replacement.
#[cfg(not(target_arch = "wasm32"))]
fn write(session: &mut Session, template: &ProjectTemplate, chosen_path: Option<&str>) -> std::result::Result<Option<String>, String> {
    let directory = session.templates.dir.as_deref();
    let template_key = template.name.to_ascii_lowercase();
    let path = if let Some(path) = chosen_path {
        PathBuf::from(path)
    } else if let Some(directory) = directory {
        session
            .template_files
            .get(&template_key)
            .filter(|file| file.parent() == Some(directory))
            .cloned()
            .unwrap_or_else(|| directory.join(format!("{}.json", texturecraft_formats::export::sanitize_file_stem(&template.name))))
    } else {
        return Ok(None);
    };

    if let Some(directory) = directory
        && path.parent() == Some(directory)
    {
        std::fs::create_dir_all(directory).map_err(|e| format!("cannot create {}: {e}", directory.display()))?;
    }

    let json = serde_json::to_string_pretty(template).map_err(|e| e.to_string())?;
    texturecraft_formats::write_atomic(&path, json.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    // Only files the library can reload become backing files
    if let Some(directory) = directory
        && is_template_file(&path)
        && let Some(file_name) = path.file_name()
        && path.parent().is_some_and(|parent| same_directory(parent, directory))
    {
        let backing_file = directory.join(file_name);
        session.template_files.retain(|name, file| name == &template_key || *file != backing_file);
        session.template_files.insert(template_key, backing_file);
    }

    Ok(Some(path.display().to_string()))
}

#[cfg(target_arch = "wasm32")]
fn write(_: &mut Session, _: &ProjectTemplate, chosen_path: Option<&str>) -> std::result::Result<Option<String>, String> {
    if chosen_path.is_some() {
        return Err("saving to a path is only available on the desktop".into());
    }

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

        // The template supplies the resolution when the command omits it
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

        // The library persists across project switches
        execute(&mut fresh, "project.new", &json!({})).unwrap();
        assert!(fresh.template("My glow").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chosen_path_in_a_new_template_library_can_be_saved_and_reloaded() {
        let mut session = Session::new_project("Template", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path().join("templates");
        session.load_templates(library.clone());
        let destination = library.join("First.json");

        execute(&mut session, "project.template.save", &json!({"name": "First", "path": destination})).unwrap();

        assert!(destination.is_file());
        assert!(session.load_templates(library).is_empty());
        assert_eq!(session.template("First").unwrap().resolution, 128);
    }

    #[test]
    fn replacing_a_template_reuses_its_library_file() {
        let mut session = Session::new_project("Template", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path().join("templates");
        let backing_file = library.join("A.json");
        session.load_templates(library.clone());
        execute(&mut session, "project.template.save", &json!({"name": "Starter", "path": backing_file})).unwrap();

        execute(&mut session, "textureSet.resolution", &json!({"resolution": 256})).unwrap();
        let saved = execute(&mut session, "project.template.save", &json!({"name": "Starter"})).unwrap();
        assert_eq!(saved["path"], backing_file.display().to_string());
        assert!(!library.join("Starter.json").exists());
        assert!(session.load_templates(library.clone()).is_empty());
        assert_eq!(session.template("Starter").unwrap().resolution, 256);

        execute(&mut session, "project.new", &json!({"resolution": 512})).unwrap();
        let saved = execute(&mut session, "project.template.save", &json!({"name": " starter "})).unwrap();
        assert_eq!(saved["path"], backing_file.display().to_string());
        assert!(session.load_templates(library).is_empty());
        assert_eq!(session.template("Starter").unwrap().resolution, 512);
    }

    #[test]
    fn exporting_a_template_leaves_its_library_file_as_the_default() {
        let mut session = Session::new_project("Template", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path().join("templates");
        let backing_file = library.join("A.json");
        let exported_file = temporary.path().join("Starter.json");
        session.load_templates(library.clone());
        execute(&mut session, "project.template.save", &json!({"name": "Starter", "path": backing_file})).unwrap();
        execute(&mut session, "project.template.save", &json!({"name": "Starter", "path": exported_file})).unwrap();
        let exported_bytes = std::fs::read(&exported_file).unwrap();

        execute(&mut session, "textureSet.resolution", &json!({"resolution": 256})).unwrap();
        let saved = execute(&mut session, "project.template.save", &json!({"name": "Starter"})).unwrap();

        assert_eq!(saved["path"], backing_file.display().to_string());
        assert_eq!(std::fs::read(&exported_file).unwrap(), exported_bytes);
        assert!(session.load_templates(library).is_empty());
        assert_eq!(session.template("Starter").unwrap().resolution, 256);
    }

    #[test]
    fn a_save_error_leaves_the_existing_template_intact() {
        let mut session = Session::new_project("Template", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        execute(&mut session, "project.template.save", &json!({"name": "Starter"})).unwrap();
        execute(&mut session, "textureSet.resolution", &json!({"resolution": 256})).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let unavailable = dir.path().join("blocked");
        std::fs::write(&unavailable, "").unwrap();
        session.templates.dir = Some(unavailable);

        let result = execute(&mut session, "project.template.save", &json!({"name": "Starter"}));

        assert!(result.unwrap_err().to_string().contains("cannot create"));
        assert_eq!(session.template("Starter").unwrap().resolution, 128);
    }

    #[test]
    fn the_list_names_templates_like_the_reference() {
        assert_eq!(template_label(DEFAULT_TEMPLATE, true), "Default (OpenPBR)");
        assert_eq!(template_label("Unlit - Emissive", true), "Unlit - Emissive (starter_assets)");
        assert_eq!(template_label("Mine", false), "Mine (your_assets)");

        let mut s = Session::new_project("t", texturecraft_geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        execute(&mut s, "project.template.save", &json!({"name": "Mine"})).unwrap();
        let labels: Vec<String> =
            execute(&mut s, "project.templates", &json!({})).unwrap().as_array().unwrap().iter().map(|t| t["label"].as_str().unwrap().to_string()).collect();
        assert_eq!(labels.first().map(String::as_str), Some("Default (OpenPBR)"));
        assert_eq!(labels.last().map(String::as_str), Some("Mine (your_assets)"));
        assert!(labels.contains(&"Unlit - Emissive (starter_assets)".to_string()));

        // Display labels resolve here and the built-in label is reserved
        assert_eq!(s.template("default (openpbr)").unwrap().name, DEFAULT_TEMPLATE);
        assert_eq!(s.template("Mine (your_assets)").unwrap().name, "Mine");
        assert!(execute(&mut s, "project.template.save", &json!({"name": "Default (OpenPBR)"})).is_err());
        execute(&mut s, "project.new", &json!({"template": "Unlit - Emissive (starter_assets)"})).unwrap();
        assert_eq!(s.doc.project.texture_sets[0].channels, s.template("Unlit - Emissive").unwrap().channels);
    }

    fn assert_template_collision_is_rejected_on_save(existing_name: &str, conflicting_name: &str) {
        let mut session = Session::new_project("Template", texturecraft_geom::primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let library = temporary.path().join("templates");
        let existing_file = library.join("existing.json");
        let conflicting_file = library.join("conflicting.json");
        session.load_templates(library.clone());
        execute(&mut session, "project.template.save", &json!({"name": existing_name, "path": existing_file})).unwrap();
        let existing_bytes = std::fs::read(&existing_file).unwrap();
        execute(&mut session, "textureSet.resolution", &json!({"resolution": 256})).unwrap();

        let result = execute(&mut session, "project.template.save", &json!({"name": conflicting_name, "path": conflicting_file}));

        assert!(result.unwrap_err().to_string().contains("conflicts"));
        assert!(!conflicting_file.exists());
        assert_eq!(std::fs::read(&existing_file).unwrap(), existing_bytes);
        assert_eq!(session.templates.list.len(), 1);
        assert_eq!(session.template(existing_name).unwrap().resolution, 128);
        let label = template_label(existing_name, false);
        execute(&mut session, "project.new", &json!({"mesh": "plane", "template": label})).unwrap();
        assert_eq!(session.doc.project.texture_sets[0].resolution, 128);
    }

    #[test]
    fn template_collision_rejects_saving_a_name_that_matches_a_label() {
        assert_template_collision_is_rejected_on_save("Mine", "  mINE (YOUR_ASSETS)  ");
    }

    #[test]
    fn template_collision_rejects_saving_a_label_that_matches_a_name() {
        assert_template_collision_is_rejected_on_save("Mine (your_assets)", "  mINE  ");
    }

    #[test]
    fn template_collision_skips_conflicting_library_files() {
        let mut session = Session::new_project("Template", texturecraft_geom::primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let library = tempfile::tempdir().unwrap();
        for (file, name, resolution) in [
            ("01.json", " default (openpbr) ", 256),
            ("02.json", "Unlit - Emissive (STARTER_ASSETS)", 256),
            ("03.json", "Mine é", 128),
            ("04.json", " MINE é (YOUR_ASSETS) ", 256),
            ("05.json", "Other (your_assets)", 128),
            ("06.json", "OTHER", 256),
            ("07.json", " mine é ", 256),
        ] {
            let template = ProjectTemplate { name: name.into(), resolution, ..ProjectTemplate::default() };
            std::fs::write(library.path().join(file), serde_json::to_vec(&template).unwrap()).unwrap();
        }

        let warnings = session.load_templates(library.path().to_path_buf());

        assert_eq!(warnings.len(), 5, "{warnings:?}");
        for (warning, file) in warnings.iter().zip(["01.json", "02.json", "04.json", "06.json", "07.json"]) {
            assert!(warning.contains(file), "{warning}");
            assert!(library.path().join(file).is_file());
        }
        assert_eq!(session.templates.list.len(), 2);
        assert_eq!(session.template("Default (OpenPBR)").unwrap().name, DEFAULT_TEMPLATE);
        assert_eq!(session.template("Unlit - Emissive (starter_assets)").unwrap().name, "Unlit - Emissive");
        for (name, label) in [("Mine é", "  MINE é (YOUR_ASSETS)  "), ("Other (your_assets)", "Other (your_assets) (your_assets)")] {
            assert_eq!(session.template(name).unwrap().resolution, 128);
            assert_eq!(session.template(label).unwrap().resolution, 128);
        }
    }

    #[test]
    fn template_collision_leaves_canonical_lookup_available_for_an_ambiguous_label() {
        let mut session = Session::new_project("Template", texturecraft_geom::primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        execute(&mut session, "project.template.save", &json!({"name": "Mine"})).unwrap();
        session.templates.list.push(ProjectTemplate { name: "mINE".into(), resolution: 256, ..ProjectTemplate::default() });

        assert_eq!(session.template("Mine").unwrap().resolution, 128);
        assert!(session.template("Mine (your_assets)").is_none());
    }
}
