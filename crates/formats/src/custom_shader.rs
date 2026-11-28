//! TextureCraft policy for annotated GLSL surface shaders.
//!
//! Format: published Painter Shader API, shader reference and custom/engine parameters,
//! https://adobedocs.github.io/painter-shader-api/api/ (accessed 2026-11-28).
//! Naga handles the GLSL language; this adapter maps the documented surface interface to
//! TextureCraft's packed channels. No external includes or file reads are performed.

use crate::{FormatError, Result, malformed};
use serde_json::Value;
use texturecraft_core::custom_shader::{CustomShader, MAX_SHADER_PARAMETERS, MAX_SHADER_TEXTURES, ShaderParameter, ShaderTexture};

pub const MAX_SHADER_BYTES: usize = 1 << 20;

fn error(message: impl std::fmt::Display) -> FormatError {
    malformed("GLSL shader", message)
}

/// Parse annotations and compile before any project state changes.
pub fn import(name: &str, source: &str) -> Result<CustomShader> {
    crate::guard("GLSL shader", || import_inner(name, source))
}

fn import_inner(name: &str, source: &str) -> Result<CustomShader> {
    if source.len() > MAX_SHADER_BYTES {
        return Err(FormatError::TooLarge("GLSL shader exceeds 1 MiB".into()));
    }
    let mut body = String::new();
    let mut annotation = String::new();
    let mut parameters = Vec::new();
    let mut textures = Vec::new();
    let mut uniforms = std::collections::HashSet::new();
    let mut warnings = Vec::new();
    for (line_index, line) in source.lines().enumerate() {
        let line = line.trim();
        if let Some(comment) = line.strip_prefix("//:") {
            if comment.trim().starts_with("param ") {
                annotation.clear();
            }
            if comment.trim().starts_with("state ") {
                let warning = "Shader state annotations are unsupported; the viewport uses its own render state.".to_string();
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
                continue;
            }
            if comment.trim().starts_with("metadata ") {
                continue;
            }
            annotation.push_str(comment.trim());
            annotation.push(' ');
            continue;
        }
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(library) = line.strip_prefix("import ") {
            let library = library.trim().trim_end_matches(';');
            if ![
                "lib-defines.glsl",
                "lib-sparse.glsl",
                "lib-sampler.glsl",
                "lib-normal.glsl",
                "lib-vectors.glsl",
                "lib-utils.glsl",
                "lib-pbr.glsl",
                "lib-env.glsl",
                "lib-emissive.glsl",
                "lib-alpha-test.glsl",
            ]
            .contains(&library)
            {
                return Err(error(format!("unsupported import `{library}` at line {}", line_index + 1)));
            }
            annotation.clear();
            continue;
        }
        if let Some(declaration) = line.strip_prefix("uniform ").or_else(|| line.strip_prefix("uniform_specialization ")) {
            let tokens: Vec<_> = declaration.trim_end_matches(';').split_whitespace().collect();
            if tokens.len() != 2 || !identifier(tokens[1]) {
                return Err(error(format!("invalid uniform at line {}", line_index + 1)));
            }
            let (kind, uniform) = (tokens[0], tokens[1]);
            if !uniforms.insert(uniform.to_string()) {
                return Err(error("duplicate shader uniform"));
            }
            if let Some(meta) = annotation.trim().strip_prefix("param custom") {
                let meta: Value =
                    serde_json::from_str(meta.trim()).map_err(|_| error(format!("invalid custom parameter annotation at line {}", line_index + 1)))?;
                let default = meta.get("default").ok_or_else(|| error("a custom parameter needs a default"))?.clone();
                if kind == "sampler2D" {
                    if textures.len() >= MAX_SHADER_TEXTURES {
                        return Err(error("too many shader texture inputs"));
                    }
                    if !default.is_string() {
                        return Err(error("sampler default must be a resource name"));
                    }
                    let color = meta.get("default_color").cloned().unwrap_or_else(|| serde_json::json!([0.0, 0.0, 0.0, 1.0]));
                    let values = color.as_array().filter(|a| a.len() == 4).ok_or_else(|| error("default_color needs four components"))?;
                    let mut default_color = [0.0; 4];
                    for (slot, value) in default_color.iter_mut().zip(values) {
                        *slot = value.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or_else(|| error("invalid default_color"))? as f32;
                    }
                    let label = meta.get("label").and_then(Value::as_str).unwrap_or(uniform);
                    body.push_str(&format!("#define {uniform} TCInputTexture({})\n", textures.len()));
                    textures.push(ShaderTexture { name: uniform.into(), label: label.into(), resource: None, default_color });
                    annotation.clear();
                    continue;
                }
                if parameters.len() >= MAX_SHADER_PARAMETERS {
                    return Err(error("too many custom parameters"));
                }
                let text = |key: &str| meta.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                let mut p = ShaderParameter {
                    name: uniform.into(),
                    kind: kind.into(),
                    label: text("label"),
                    group: text("group"),
                    description: text("description"),
                    widget: text("widget"),
                    min: meta.get("min").and_then(Value::as_f64).map(|v| v as f32),
                    max: meta.get("max").and_then(Value::as_f64).map(|v| v as f32),
                    choices: Vec::new(),
                    default: default.clone(),
                    value: default,
                };
                if p.label.is_empty() {
                    p.label = uniform.into();
                }
                if p.min.is_some_and(|v| !v.is_finite())
                    || p.max.is_some_and(|v| !v.is_finite())
                    || matches!((p.min, p.max), (Some(min), Some(max)) if min > max)
                {
                    return Err(error("invalid shader parameter range"));
                }
                if let Some(choices) = meta.get("values").and_then(Value::as_object) {
                    for (label, v) in choices {
                        p.choices
                            .push((label.clone(), v.as_i64().and_then(|v| i32::try_from(v).ok()).ok_or_else(|| error("combobox values must be integers"))?));
                    }
                }
                p.value = validate_value(&p, &p.value)?;
                p.default = p.value.clone();
                let swizzle = match components(kind)? {
                    1 => "x",
                    2 => "xy",
                    3 => "xyz",
                    _ => "xyzw",
                };
                let value = format!("tc_params.values[{}].{swizzle}", parameters.len());
                let value = match kind {
                    "bool" => format!("({value} != 0.0)"),
                    "int" | "ivec2" | "ivec3" | "ivec4" => format!("floatBitsToInt({value})"),
                    _ => format!("{kind}({value})"),
                };
                body.push_str(&format!("#define {uniform} ({value})\n"));
                parameters.push(p);
            } else if let Some(tag) = annotation.trim().strip_prefix("param auto ") {
                if tag.trim().starts_with("channel_user") && kind == "SamplerSparse" {
                    let index = tag
                        .trim()
                        .strip_prefix("channel_user")
                        .and_then(|v| v.parse::<u8>().ok())
                        .filter(|v| *v < 8)
                        .ok_or_else(|| error("invalid user texture channel"))?;
                    if textures.len() >= MAX_SHADER_TEXTURES {
                        return Err(error("too many shader texture inputs"));
                    }
                    body.push_str(&format!("#define {uniform} SamplerSparse({})\n", 16 + textures.len()));
                    textures.push(ShaderTexture {
                        name: uniform.into(),
                        label: format!("User channel {index}"),
                        resource: None,
                        default_color: [0.0, 0.0, 0.0, 1.0],
                    });
                } else {
                    body.push_str(&format!("#define {uniform} ({})\n", auto_value(kind, tag.trim())?));
                }
            } else {
                return Err(error(format!("uniform `{uniform}` needs a supported parameter annotation")));
            }
            annotation.clear();
            continue;
        }
        if line.starts_with("#version") {
            continue;
        }
        if line.starts_with("#include") {
            return Err(error("file includes are unsupported; use the documented library imports"));
        }
        annotation.clear();
        body.push_str(line);
        body.push('\n');
    }
    let glsl = format!(
        "{}\n{}\n{}\n#define sampler2D TCInputTexture\n{body}\n{}",
        include_str!("custom_shader/interface.glsl"),
        include_str!("custom_shader/surface.glsl"),
        include_str!("custom_shader/library_parameters.glsl"),
        include_str!("custom_shader/entry.glsl")
    );
    let module = naga::front::glsl::Frontend::default()
        .parse(&naga::front::glsl::Options::from(naga::ShaderStage::Fragment), &glsl)
        .map_err(|errors| error(format!("GLSL compilation failed: {errors}")))?;
    for (_, global) in module.global_variables.iter() {
        if let Some(binding) = &global.binding {
            let expected = match global.name.as_deref() {
                Some("g") => Some((0, 0)),
                Some("tc_params") => Some((0, 5)),
                Some("tc_base") => Some((1, 0)),
                Some("tc_orm") => Some((1, 1)),
                Some("tc_normal") => Some((1, 2)),
                Some("tc_height") => Some((1, 3)),
                Some("tc_emissive") => Some((1, 4)),
                Some("tc_maps") => Some((1, 8)),
                Some("tc_sampler") => Some((1, 9)),
                Some("tc_shadow_map") => Some((0, 3)),
                Some("tc_shadow_sampler") => Some((0, 4)),
                Some("tc_inputs") => Some((0, 6)),
                Some("tc_environment") => Some((0, 1)),
                Some("tc_env_sampler") => Some((0, 2)),
                _ => None,
            };
            if expected != Some((binding.group, binding.binding)) {
                return Err(error("shader declares an unsupported GPU resource; use parameter annotations"));
            }
        }
    }
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).map_err(error)?;
    let wgsl = naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty()).map_err(error)?;
    Ok(CustomShader { name: name.into(), source: source.into(), wgsl: wgsl.into(), parameters, textures, warnings })
}

fn identifier(name: &str) -> bool {
    !name.is_empty() && !name.starts_with("tc_") && name.chars().enumerate().all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

fn components(kind: &str) -> Result<usize> {
    match kind {
        "bool" | "int" | "float" => Ok(1),
        "vec2" | "ivec2" => Ok(2),
        "vec3" | "ivec3" => Ok(3),
        "vec4" | "ivec4" => Ok(4),
        _ => Err(error(format!("unsupported custom parameter type `{kind}`"))),
    }
}

pub fn validate_value(p: &ShaderParameter, value: &Value) -> Result<Value> {
    let count = components(&p.kind)?;
    if p.kind == "bool" {
        return value.as_bool().map(Value::Bool).ok_or_else(|| error(format!("{} must be boolean", p.name)));
    }
    let check = |v: &Value| -> Result<Value> {
        let number = v.as_f64().filter(|n| n.is_finite() && (*n as f32).is_finite()).ok_or_else(|| error(format!("{} must contain finite numbers", p.name)))?;
        if p.min.is_some_and(|min| number < min as f64) || p.max.is_some_and(|max| number > max as f64) {
            return Err(error(format!("{} is outside its range", p.name)));
        }
        if p.kind == "int" || p.kind.starts_with("ivec") {
            if number.fract() != 0.0 || number < i32::MIN as f64 || number > i32::MAX as f64 {
                return Err(error(format!("{} must contain integers", p.name)));
            }
            if !p.choices.is_empty() && !p.choices.iter().any(|(_, v)| *v == number as i32) {
                return Err(error("unknown shader combobox value"));
            }
        }
        if p.kind == "int" || p.kind.starts_with("ivec") { Ok(serde_json::json!(number as i32)) } else { Ok(v.clone()) }
    };
    if count == 1 {
        return check(value);
    }
    let values = match value.as_array() {
        Some(a) if a.len() == count => a.clone(),
        Some(_) => return Err(error("wrong shader vector length")),
        None => vec![value.clone(); count],
    };
    values.iter().map(check).collect::<Result<Vec<_>>>().map(Value::Array)
}

fn auto_value(kind: &str, tag: &str) -> Result<String> {
    let channel = match tag {
        "channel_basecolor" | "channel_diffuse" => Some(0),
        "channel_roughness" => Some(1),
        "channel_metallic" => Some(2),
        "channel_ambientocclusion" => Some(3),
        "channel_normal" => Some(4),
        "channel_height" | "channel_displacement" => Some(5),
        "channel_emissive" => Some(6),
        "channel_opacity" => Some(7),
        "channel_specularlevel" => Some(8),
        _ => None,
    };
    let channel = channel.or(match tag {
        "texture_curvature" => Some(9),
        "texture_thickness" => Some(10),
        "texture_ambientocclusion" => Some(11),
        _ => None,
    });
    if let Some(channel) = channel {
        return if kind == "SamplerSparse" { Ok(format!("SamplerSparse({channel})")) } else { Err(error("channel bindings require SamplerSparse")) };
    }
    let expression = match tag {
        "world_eye_position" => "g.eye.xyz",
        "world_camera_direction" => "g.fwd.xyz",
        "main_light" => "g.light",
        "environment_max_lod" => "g.env.y",
        "environment_exposure" => "1.0",
        "is_2d_view" => "(g.bg.z > 0.5)",
        "facing" => "0",
        "mvp_matrix" => "g.view_proj",
        "camera_vp_matrix_inverse" => "g.inv_view_proj",
        "camera_view_matrix" => "tc_camera_view()",
        "camera_view_matrix_it" => "transpose(inverse(tc_camera_view()))",
        "texture_environment" if kind == "sampler2D" => "TCInputTexture(-1)",
        _ => return Err(error(format!("unsupported automatic parameter `{tag}`"))),
    };
    Ok(expression.into())
}
