//! Export shader parameters: the project's shader, its settings and the texture sets using it, as JSON

use serde_json::{Value, json};
use texturecraft_core::Project;
use texturecraft_formats::ExportedFile;

use crate::Result;
use crate::commands::bad;

/// The project's one shader, its parameters and the names of `sets` (they all use it)
pub fn shader_parameters(project: &Project, sets: &[usize]) -> Value {
    let display = &project.display;
    let model = if display.spec_gloss { "specularGlossiness" } else { "metallicRoughness" };
    let names: Vec<&str> = sets.iter().filter_map(|ts| project.texture_sets.get(*ts)).map(|set| set.name.as_str()).collect();

    if let Some(shader) = display.custom_shaders.iter().find(|shader| Some(&shader.name) == display.custom_shader.as_ref()) {
        let parameters: serde_json::Map<String, Value> = shader.parameters.iter().map(|p| (p.name.clone(), p.value.clone())).collect();
        return json!({"shaders":[{"model":"customGlsl","name":shader.name,"instanceName":display.shader_instance_name,"parameters":parameters,"textures":shader.textures,"textureSets":names}]});
    }

    json!({
        "shaders": [{
            "model": model,
            "parameters": {
                "aoIntensity": display.ao_intensity,
                "ior": display.ior,
                "scatterColor": display.scatter_color,
                "scatterRadius": display.scatter_radius,
                "displacement": display.displacement,
                "parallax": display.parallax,
            },
            "textureSets": names,
        }]
    })
}

/// The JSON file's name for project `name`
pub fn shader_parameters_file(name: &str) -> String {
    format!("{}_shaders.json", texturecraft_formats::export::sanitize_file_stem(name))
}

/// Refuse the file name `name` when one of the export's texture paths (`/`-separated, relative to the export folder) is that file or lies in a folder of that name, ignoring case
pub fn check_shader_parameters_name<'a>(name: &str, texture_paths: impl IntoIterator<Item = &'a str>) -> std::result::Result<(), String> {
    if texture_paths.into_iter().any(|path| path.split('/').next().is_some_and(|root| root.eq_ignore_ascii_case(name))) {
        return Err(format!("\"{name}\" is already used by a texture file or folder"));
    }

    Ok(())
}

/// Build the download or headless sidecar after checking its name against every texture path.
pub fn shader_parameters_sidecar(project: &Project, sets: &[usize], textures: &[ExportedFile]) -> std::result::Result<ExportedFile, String> {
    let name = shader_parameters_file(&project.name);
    check_shader_parameters_name(&name, textures.iter().map(|texture| texture.name.as_str()))?;

    let bytes = serde_json::to_vec_pretty(&shader_parameters(project, sets)).map_err(|error| error.to_string())?;
    Ok(ExportedFile { name, bytes, width: 0, height: 0, warnings: Vec::new() })
}

/// The texture sets `shaderParameters` asks to list: `true` takes the command's own `sets`, a list gives them all; None when it asks for no file
pub fn shader_parameters_param(project: &Project, params: &Value, sets: &[usize], cmd: &str) -> Result<Option<Vec<usize>>> {
    match params.get("shaderParameters") {
        None | Some(Value::Null | Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(sets.to_vec())),
        Some(Value::Array(indexes)) => indexes
            .iter()
            .map(|index| {
                index
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .filter(|index| *index < project.texture_sets.len())
                    .ok_or_else(|| bad(cmd, "`shaderParameters` lists texture set indexes"))
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err(bad(cmd, "`shaderParameters` must be true, false or a list of texture set indexes")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_json_names_the_shader_its_settings_and_its_texture_sets() {
        let mut session = crate::sample_session(128, 32).unwrap();
        session.doc.project.display.ior = 1.33;

        let value = shader_parameters(&session.doc.project, &[1]);

        let shader = &value["shaders"][0];

        assert_eq!(shader["model"], "metallicRoughness");
        assert!((shader["parameters"]["ior"].as_f64().unwrap() - 1.33).abs() < 1e-6);
        assert_eq!(shader["textureSets"], json!([session.doc.project.texture_sets[1].name]));
        assert!(shader_parameters_file("Material Ball").ends_with("_shaders.json"));
    }
}
