//! Import, select and edit the project's GLSL shaders through the command registry.

use crate::commands::{CommandSpec, always, bad, strp};
use crate::{Result, Session, cmd};
use serde_json::{Value, json};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("shader.import", "Import shader", [], None, "{name, source} | {path} — annotated GLSL surface shader; validates before importing", always, import),
        cmd!("shader.list", "List shaders", [], None, "{}", always, |s, _| Ok(list(s))),
        cmd!("shader.select", "Select shader", [], None, "{name} — open-pbr or an imported shader", always, select),
        cmd!(
            "shader.set",
            "Set shader parameters",
            [],
            None,
            "{parameters?: {uniform: value}, textures?: {input: imageResource | null}, instanceName?, reset?: false}",
            always,
            set
        ),
    ]
}

pub fn list(s: &Session) -> Value {
    let d = &s.doc.project.display;
    json!({"selected": d.custom_shader.as_deref().unwrap_or("open-pbr"), "instanceName": d.shader_instance_name, "support": texturecraft_core::custom_shader::SHADER_SUPPORT, "shaders": d.custom_shaders.iter().map(|shader| json!({"name":shader.name,"parameters":shader.parameters,"textures":shader.textures,"warnings":shader.warnings})).collect::<Vec<_>>()})
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(source) = strp(p, "source") {
        let name = strp(p, "name").ok_or_else(|| bad("shader.import", "name is required with source"))?;
        return import_source(s, name, source);
    }
    let path = strp(p, "path").ok_or_else(|| bad("shader.import", "source and name, or path, are required"))?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let bytes = crate::resource_cmds::read_resource_file(
            "shader.import",
            std::path::Path::new(path),
            texturecraft_formats::custom_shader::MAX_SHADER_BYTES as u64,
        )?;
        let source = std::str::from_utf8(&bytes).map_err(|_| bad("shader.import", "shader must be UTF-8"))?;
        let name = std::path::Path::new(path).file_stem().and_then(|v| v.to_str()).ok_or_else(|| bad("shader.import", "shader filename is invalid"))?;
        import_source(s, name, source)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err(bad("shader.import", "use name and source in the browser"))
    }
}

pub fn import_source(s: &mut Session, name: &str, source: &str) -> Result<Value> {
    let name = name.trim();
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) || name == "open-pbr" {
        return Err(bad("shader.import", "shader name must be 1–128 bytes and must not be open-pbr"));
    }
    let shader = texturecraft_formats::custom_shader::import(name, source).map_err(|e| bad("shader.import", e.to_string()))?;
    let warnings = shader.warnings.clone();
    s.edit("Import shader", |s| {
        let d = &mut s.doc.project.display;
        if d.custom_shaders.len() >= texturecraft_core::custom_shader::MAX_CUSTOM_SHADERS {
            return Err(bad("shader.import", "project has too many custom shaders"));
        }
        if d.custom_shaders.iter().any(|v| v.name == name) {
            return Err(bad("shader.import", "a shader with that name already exists"));
        }
        d.custom_shaders.push(shader);
        Ok(json!({"name":name,"warnings":warnings,"support":texturecraft_core::custom_shader::SHADER_SUPPORT}))
    })
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let name = strp(p, "name").ok_or_else(|| bad("shader.select", "name is required"))?;
    if name != "open-pbr" && !s.doc.project.display.custom_shaders.iter().any(|shader| shader.name == name) {
        return Err(bad("shader.select", "unknown shader"));
    }
    s.edit_if_changed("Select shader", false, |s| {
        let selected = (name != "open-pbr").then(|| name.to_string());
        let changed = s.doc.project.display.custom_shader != selected;
        s.doc.project.display.custom_shader = selected;
        Ok((list(s), changed))
    })
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("reset").is_some_and(|value| !value.is_boolean()) {
        return Err(bad("shader.set", "reset must be boolean"));
    }
    s.edit_if_changed("Set shader parameters", false, |s| {
        let before = s.doc.project.display.clone();
        let d = &mut s.doc.project.display;
        if let Some(name) = p.get("instanceName") {
            let name = name
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control))
                .ok_or_else(|| bad("shader.set", "instanceName must be 1–128 bytes"))?;
            d.shader_instance_name = name.into();
        }
        if p.get("parameters").is_some() || p.get("reset").is_some() || p.get("textures").is_some() {
            let shader = d
                .custom_shaders
                .iter_mut()
                .find(|v| Some(&v.name) == d.custom_shader.as_ref())
                .ok_or_else(|| bad("shader.set", "select an imported shader first"))?;
            if p.get("reset").and_then(Value::as_bool) == Some(true) {
                for parameter in &mut shader.parameters {
                    parameter.value = parameter.default.clone();
                }
                for texture in &mut shader.textures {
                    texture.resource = None;
                }
            }
            if let Some(parameters) = p.get("parameters") {
                let parameters = parameters.as_object().ok_or_else(|| bad("shader.set", "parameters must be an object"))?;
                for (name, value) in parameters {
                    let parameter =
                        shader.parameters.iter_mut().find(|p| &p.name == name).ok_or_else(|| bad("shader.set", format!("unknown parameter `{name}`")))?;
                    parameter.value = texturecraft_formats::custom_shader::validate_value(parameter, value).map_err(|e| bad("shader.set", e.to_string()))?;
                }
            }
            if let Some(textures) = p.get("textures") {
                let textures =
                    textures.as_object().ok_or_else(|| bad("shader.set", "textures must be an object of input names to image resource names or null"))?;
                for (name, value) in textures {
                    let texture = shader.textures.iter_mut().find(|t| &t.name == name).ok_or_else(|| bad("shader.set", "unknown shader texture input"))?;
                    texture.resource = if value.is_null() {
                        None
                    } else {
                        let resource = value.as_str().filter(|name| s.images.contains_key(*name)).ok_or_else(|| bad("shader.set", "unknown image resource"))?;
                        Some(resource.into())
                    };
                }
            }
        }
        let changed = before != *d;
        Ok((list(s), changed))
    })
}
