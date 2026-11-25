//! `effect.set` keys of Baked Lighting Stylized past Baked Lighting's sun and ambient

use serde_json::Value;
use texturecraft_core::{DirectionalLight, LightingOutput, StylizedLighting};

use crate::Result;
use crate::commands::{bad, f32p};
use crate::filter_lib::{choice, set_color, two_way};
use crate::layer_cmds::FULL_TURN_DEGREES;

/// The keys that give a Baked Lighting saved before them its stylized settings
pub(crate) const KEYS: [&str; 17] = [
    "sunColor",
    "skyColor",
    "horizonColor",
    "groundColor",
    "light1",
    "light2",
    "diffuseCavity",
    "specularAo",
    "specularCavity",
    "cavitySmoothness",
    "edgesIntensity",
    "edgesSmoothness",
    "input",
    "output",
    "dielectricReflectance",
    "normalDetails",
    "heightToNormal",
];

/// The highest Dielectric Reflectance, Cavity Smoothness and Height to Normal Intensity
pub const MAX_DIELECTRIC_REFLECTANCE: f32 = 0.16;
pub const MAX_CAVITY_SMOOTHNESS: f32 = 4.0;
pub const MAX_HEIGHT_TO_NORMAL: f32 = 3.0;

/// The highest intensity of Light 1 and Light 2
pub const MAX_LIGHT_INTENSITY: f32 = 2.0;

/// Whether `p` sets any stylized setting
pub(crate) fn sets_any(p: &Value) -> bool {
    KEYS.iter().any(|key| p.get(*key).is_some())
}

/// Set the stylized settings `p` names on `lighting`, clamped; a bad color or choice is refused
pub(crate) fn apply(p: &Value, lighting: &mut StylizedLighting, cmd: &str) -> Result<()> {
    let set = |slot: &mut f32, key: &str, high: f32| {
        if let Some(value) = f32p(p, key) {
            *slot = value.clamp(0.0, high);
        }
    };

    for (key, slot) in [
        ("sunColor", &mut lighting.sun_color),
        ("skyColor", &mut lighting.sky_color),
        ("horizonColor", &mut lighting.horizon_color),
        ("groundColor", &mut lighting.ground_color),
    ] {
        set_color(p, key, slot, cmd)?;
    }

    for (key, light) in ["light1", "light2"].into_iter().zip(lighting.lights.iter_mut()) {
        if let Some(params) = p.get(key) {
            set_light(params, light, key, cmd)?;
        }
    }

    set(&mut lighting.diffuse_cavity, "diffuseCavity", 1.0);
    set(&mut lighting.specular_ao, "specularAo", 1.0);
    set(&mut lighting.specular_cavity, "specularCavity", 1.0);
    set(&mut lighting.cavity_smoothness, "cavitySmoothness", MAX_CAVITY_SMOOTHNESS);
    set(&mut lighting.edges_intensity, "edgesIntensity", 1.0);
    set(&mut lighting.edges_smoothness, "edgesSmoothness", 1.0);
    set(&mut lighting.dielectric_reflectance, "dielectricReflectance", MAX_DIELECTRIC_REFLECTANCE);
    set(&mut lighting.height_to_normal, "heightToNormal", MAX_HEIGHT_TO_NORMAL);

    lighting.specular_glossiness = two_way(p, "input", "metalRoughness", "specularGlossiness", cmd)?.unwrap_or(lighting.specular_glossiness);
    lighting.output = choice(p, "output", &LightingOutput::ALL, LightingOutput::id, cmd)?.unwrap_or(lighting.output);
    lighting.mesh_only = two_way(p, "normalDetails", "meshHeightNormal", "mesh", cmd)?.unwrap_or(lighting.mesh_only);
    Ok(())
}

/// Light 1 or Light 2 from `{azimuth, elevation, intensity, color}` (any of them)
fn set_light(p: &Value, light: &mut DirectionalLight, key: &str, cmd: &str) -> Result<()> {
    if !p.is_object() {
        return Err(bad(cmd, format!("`{key}` must be an object {{azimuth, elevation, intensity, color}}")));
    }

    if let Some(degrees) = f32p(p, "azimuth") {
        light.azimuth = degrees.rem_euclid(FULL_TURN_DEGREES);
    }
    if let Some(degrees) = f32p(p, "elevation") {
        light.elevation = degrees.rem_euclid(FULL_TURN_DEGREES);
    }
    if let Some(intensity) = f32p(p, "intensity") {
        light.intensity = intensity.clamp(0.0, MAX_LIGHT_INTENSITY);
    }
    set_color(p, "color", &mut light.color, cmd)
}
