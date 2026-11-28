//! Project-owned GLSL surface shaders and their editable parameters.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// A shader is stored with the project; importing never keeps a host file dependency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomShader {
    pub name: String,
    pub source: Arc<str>,
    pub wgsl: Arc<str>,
    pub parameters: Vec<ShaderParameter>,
    #[serde(default)]
    pub textures: Vec<ShaderTexture>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

pub const SHADER_SUPPORT: &str = "Custom GLSL shaders run in the 3D material viewport. Library lighting uses TextureCraft approximations; path tracing is unavailable. Image inputs are resampled to at most 512 pixels per side without mipmaps.";

/// An image input to a surface shader; absent inputs use their declared default color.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShaderTexture {
    pub name: String,
    pub label: String,
    pub resource: Option<String>,
    pub default_color: [f32; 4],
}

pub const MAX_SHADER_TEXTURES: usize = 16;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShaderParameter {
    pub name: String,
    pub kind: String,
    pub label: String,
    pub group: String,
    pub description: String,
    pub widget: String,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub choices: Vec<(String, i32)>,
    pub default: Value,
    pub value: Value,
}

/// Maximum editable parameters, one aligned vector per parameter on the GPU.
pub const MAX_SHADER_PARAMETERS: usize = 128;
pub const MAX_CUSTOM_SHADERS: usize = 16;

impl CustomShader {
    pub fn uniform_values(&self) -> [[f32; 4]; MAX_SHADER_PARAMETERS] {
        let mut values = [[0.0; 4]; MAX_SHADER_PARAMETERS];
        for (slot, p) in values.iter_mut().zip(&self.parameters) {
            for (i, component) in slot.iter_mut().enumerate() {
                let value = p.value.as_array().and_then(|a| a.get(i)).unwrap_or(&p.value);
                *component = if p.kind == "int" || p.kind.starts_with("ivec") {
                    f32::from_bits(value.as_i64().unwrap_or(0) as i32 as u32)
                } else {
                    value.as_f64().map(|v| v as f32).unwrap_or_else(|| if value.as_bool() == Some(true) { 1.0 } else { 0.0 })
                };
            }
        }
        values
    }
}
