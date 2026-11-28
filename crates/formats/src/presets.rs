//! Built-in export presets and user preset JSON. Preset names are ours; their contents follow
//! each engine's public material conventions.

use texturecraft_core::{BakedMap, Channel, ChannelKind};

use crate::export::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef, VirtualMap};
use crate::{FormatError, Result};

/// Largest preset JSON accepted.
const MAX_PRESET_JSON: usize = 1 << 20;

pub(crate) fn ch(channel: Channel, component: u8) -> SourceRef {
    SourceRef::Channel { channel, component }
}

pub(crate) fn virt(map: VirtualMap, component: u8) -> SourceRef {
    SourceRef::Virtual { map, component }
}

/// `$mesh_$textureSet_<suffix>` PNG.
fn map(suffix: &str, bit_depth: u8, channels: Vec<(Dest, SourceRef)>) -> MapSpec {
    MapSpec { file: format!("$mesh_$textureSet_{suffix}"), format: FileFormat::Png, bit_depth, channels, inverted: Vec::new() }
}

pub(crate) fn rgb(f: impl Fn(u8) -> SourceRef) -> Vec<(Dest, SourceRef)> {
    vec![(Dest::R, f(0)), (Dest::G, f(1)), (Dest::B, f(2))]
}

pub(crate) fn gray(s: SourceRef) -> Vec<(Dest, SourceRef)> {
    vec![(Dest::L, s)]
}

/// Base color RGB with opacity in alpha (alpha is dropped when the set has no opacity).
fn base_with_alpha() -> Vec<(Dest, SourceRef)> {
    let mut c = rgb(|k| ch(Channel::BaseColor, k));
    c.push((Dest::A, ch(Channel::Opacity, 0)));
    c
}

fn normal(map_kind: VirtualMap) -> Vec<(Dest, SourceRef)> {
    rgb(|k| virt(map_kind, k))
}

/// One file per channel: RGB for colors and normals, grayscale (16-bit for height) otherwise.
fn channel_map(c: Channel) -> MapSpec {
    let (channels, depth) = match c.kind() {
        ChannelKind::Color | ChannelKind::NormalMap => (rgb(|k| ch(c, k)), 8),
        ChannelKind::Signed => (gray(ch(c, 0)), 16),
        ChannelKind::Scalar => (gray(ch(c, 0)), 8),
    };
    map("$channel", depth, channels)
}

fn orm() -> Vec<(Dest, SourceRef)> {
    vec![(Dest::R, virt(VirtualMap::AoMixed, 0)), (Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Metallic, 0))]
}

/// File stems of the glTF metallic-roughness maps (also the GLB export's texture slots).
pub(crate) const GLTF_BASE_COLOR: &str = "BaseColor";
pub(crate) const GLTF_ORM: &str = "OcclusionRoughnessMetallic";
pub(crate) const GLTF_NORMAL: &str = "Normal";
pub(crate) const GLTF_EMISSIVE: &str = "Emissive";
/// KHR_materials_clearcoat: clear coat weight (R) and roughness (G).
pub(crate) const GLTF_CLEARCOAT: &str = "Clearcoat";
/// KHR_materials_sheen: sheen color × weight (RGB) and roughness (A).
pub(crate) const GLTF_SHEEN: &str = "Sheen";

/// glTF metallic-roughness maps (8-bit PNG), plus the clear coat and sheen extension maps where
/// the texture set has those channels; `file` turns a stem above into a file template.
pub(crate) fn gltf_pbr_maps(file: impl Fn(&str) -> String) -> Vec<MapSpec> {
    let spec = |stem: &str, channels| MapSpec { file: file(stem), format: FileFormat::Png, bit_depth: 8, channels, inverted: Vec::new() };
    let mut sheen = rgb(|k| virt(VirtualMap::SheenTint, k));
    sheen.push((Dest::A, ch(Channel::SheenRoughness, 0)));
    vec![
        spec(GLTF_BASE_COLOR, base_with_alpha()),
        spec(GLTF_ORM, orm()),
        spec(GLTF_NORMAL, normal(VirtualMap::NormalOpenGl)),
        spec(GLTF_EMISSIVE, rgb(|k| ch(Channel::Emissive, k))),
        spec(GLTF_CLEARCOAT, vec![(Dest::R, ch(Channel::CoatWeight, 0)), (Dest::G, ch(Channel::CoatRoughness, 0))]),
        spec(GLTF_SHEEN, sheen),
    ]
}

/// Every built-in preset, in menu order.
pub fn builtin_presets() -> Vec<ExportPreset> {
    let preset = |name: &str, maps: Vec<MapSpec>| ExportPreset { name: name.into(), maps };
    vec![
        preset(DEFAULT_PRESET, {
            let mut maps: Vec<MapSpec> =
                Channel::ALL.into_iter().filter(|c| c.is_shaded() && !matches!(c, Channel::Normal | Channel::AmbientOcclusion)).map(channel_map).collect();
            maps.push(map("normal", 8, rgb(|k| ch(Channel::Normal, k))));
            maps.push(map("ambientOcclusion", 8, gray(virt(VirtualMap::AoMixed, 0))));

            // Later channels come last, so the maps automation addresses by index keep their place
            maps.extend(Channel::ALL.into_iter().filter(|c| !c.is_shaded()).map(channel_map));
            maps
        }),
        // Spec: https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#metallic-roughness-material
        preset("glTF PBR Metal Roughness", gltf_pbr_maps(|s| format!("$mesh_$textureSet_{s}"))),
        // Spec: https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#metallic-roughness-material
        // Spec: https://dev.epicgames.com/documentation/en-us/unreal-engine/physically-based-materials-in-unreal-engine
        preset(
            "Unreal Engine (Packed)",
            vec![map("BaseColor", 8, base_with_alpha()), map("OcclusionRoughnessMetallic", 8, orm()), map("Normal", 8, normal(VirtualMap::NormalDirectX))],
        ),
        // Spec: https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Mask-Map-and-Detail-Map.html
        preset(
            "Unity HDRP (Mask Map)",
            vec![
                map("BaseMap", 8, base_with_alpha()),
                map(
                    "MaskMap",
                    8,
                    vec![
                        (Dest::R, ch(Channel::Metallic, 0)),
                        (Dest::G, virt(VirtualMap::AoMixed, 0)),
                        // The preset has no detail mask
                        (Dest::B, SourceRef::Constant(0.0)),
                        (Dest::A, virt(VirtualMap::Glossiness, 0)),
                    ],
                ),
                map("Normal", 8, normal(VirtualMap::NormalOpenGl)),
            ],
        ),
        // Spec: https://docs.unity3d.com/Manual/StandardShaderMaterialParameterMetallic.html (smoothness in the metallic map's alpha)
        preset(
            "Unity Standard (Metallic Smoothness)",
            vec![
                map("Albedo", 8, base_with_alpha()),
                map(
                    "MetallicSmoothness",
                    8,
                    vec![
                        (Dest::R, ch(Channel::Metallic, 0)),
                        (Dest::G, SourceRef::Constant(0.0)),
                        (Dest::B, SourceRef::Constant(0.0)),
                        (Dest::A, virt(VirtualMap::Glossiness, 0)),
                    ],
                ),
                map("Normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("Occlusion", 8, gray(virt(VirtualMap::AoMixed, 0))),
            ],
        ),
        // Spec: https://docs.blender.org/manual/en/latest/render/shader_nodes/shader/principled.html
        preset(
            "Blender (Principled BSDF)",
            vec![
                map("BaseColor", 8, base_with_alpha()),
                map("Metallic", 8, gray(ch(Channel::Metallic, 0))),
                map("Roughness", 8, gray(ch(Channel::Roughness, 0))),
                map("Normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("Height", 16, gray(ch(Channel::Height, 0))),
                map("Emission", 8, rgb(|k| ch(Channel::Emissive, k))),
                map("Specular", 8, gray(ch(Channel::SpecularLevel, 0))),
            ],
        ),
        // Spec: https://docs.godotengine.org/en/stable/tutorials/3d/standard_material_3d.html (ORM material: R occlusion, G roughness, B metallic)
        preset(
            "Godot 4 (ORM)",
            vec![
                map("albedo", 8, base_with_alpha()),
                map("orm", 8, orm()),
                map("normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("emission", 8, rgb(|k| ch(Channel::Emissive, k))),
            ],
        ),
        // Spec: https://academysoftwarefoundation.github.io/OpenPBR/
        // One map per OpenPBR input present in the texture set
        preset("OpenPBR (separate maps)", {
            let gray8 = |name: &str, c: Channel| map(name, 8, gray(ch(c, 0)));
            let rgb8 = |name: &str, c: Channel| map(name, 8, rgb(|k| ch(c, k)));
            vec![
                rgb8("base_color", Channel::BaseColor),
                gray8("base_metalness", Channel::Metallic),
                gray8("specular_roughness", Channel::Roughness),
                gray8("specular_roughness_anisotropy", Channel::AnisotropyLevel),
                gray8("coat_weight", Channel::CoatWeight),
                gray8("coat_roughness", Channel::CoatRoughness),
                gray8("fuzz_weight", Channel::SheenWeight),
                rgb8("fuzz_color", Channel::SheenColor),
                gray8("fuzz_roughness", Channel::SheenRoughness),
                gray8("transmission_weight", Channel::Transmission),
                gray8("subsurface_weight", Channel::ScatterWeight),
                rgb8("emission_color", Channel::Emissive),
                gray8("geometry_opacity", Channel::Opacity),
                map("geometry_normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("height", 16, gray(ch(Channel::Height, 0))),
            ]
        }),
        // Spec: https://docs.unity3d.com/Packages/com.unity.render-pipelines.universal@17.0/manual/lit-shader.html (smoothness in the metallic map's alpha)
        preset(
            "Unity URP Lit (Metallic Smoothness)",
            vec![
                map("BaseMap", 8, base_with_alpha()),
                map(
                    "MetallicMap",
                    8,
                    vec![
                        (Dest::R, ch(Channel::Metallic, 0)),
                        (Dest::G, SourceRef::Constant(0.0)),
                        (Dest::B, SourceRef::Constant(0.0)),
                        (Dest::A, virt(VirtualMap::Glossiness, 0)),
                    ],
                ),
                map("NormalMap", 8, normal(VirtualMap::NormalOpenGl)),
                map("OcclusionMap", 8, gray(virt(VirtualMap::AoMixed, 0))),
                map("EmissionMap", 8, rgb(|k| ch(Channel::Emissive, k))),
            ],
        ),
        // Spec: https://openusd.org/release/spec_usdpreviewsurface.html (one texture per input, named like it)
        preset(
            "USD Preview Surface (separate maps)",
            vec![
                map("diffuseColor", 8, rgb(|k| ch(Channel::BaseColor, k))),
                map("emissiveColor", 8, rgb(|k| ch(Channel::Emissive, k))),
                map("metallic", 8, gray(ch(Channel::Metallic, 0))),
                map("roughness", 8, gray(ch(Channel::Roughness, 0))),
                map("normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("occlusion", 8, gray(virt(VirtualMap::AoMixed, 0))),
                map("opacity", 8, gray(ch(Channel::Opacity, 0))),
                map("clearcoat", 8, gray(ch(Channel::CoatWeight, 0))),
                map("clearcoatRoughness", 8, gray(ch(Channel::CoatRoughness, 0))),
            ],
        ),
        // Spec: https://dev.epicgames.com/documentation/en-us/unreal-engine/subsurface-profile-shading-model-in-unreal-engine (the Opacity input masks the scattering, so the base color has no alpha)
        preset(
            "Unreal Engine Subsurface Profile (Packed)",
            vec![
                map("BaseColor", 8, rgb(|k| ch(Channel::BaseColor, k))),
                map("OcclusionRoughnessMetallic", 8, orm()),
                map("Normal", 8, normal(VirtualMap::NormalDirectX)),
                map("SubsurfaceMask", 8, gray(ch(Channel::ScatterWeight, 0))),
            ],
        ),
        // Spec: https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Archived/KHR_materials_pbrSpecularGlossiness/README.md (diffuse with alpha, specular RGB with glossiness in A)
        preset(
            "glTF Specular Glossiness (archived extension)",
            vec![
                map("Diffuse", 8, base_with_alpha()),
                map("SpecularGlossiness", 8, {
                    let mut specular_glossiness = rgb(|k| ch(Channel::Specular, k));
                    specular_glossiness.push((Dest::A, ch(Channel::Glossiness, 0)));
                    specular_glossiness
                }),
                map("Normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("Occlusion", 8, gray(virt(VirtualMap::AoMixed, 0))),
                map("Emissive", 8, rgb(|k| ch(Channel::Emissive, k))),
            ],
        ),
        // Spec: https://help.maxon.net/r3d/maya/en-us/Content/html/Standard_Material.html (the Standard Material inputs)
        // Spec: https://help.maxon.net/r3d/maya/en-us/Content/html/Bump+Map.html (normal maps in OpenGL format)
        preset("Redshift Standard Material (separate maps)", {
            let gray8 = |name: &str, c: Channel| map(name, 8, gray(ch(c, 0)));
            let rgb8 = |name: &str, c: Channel| map(name, 8, rgb(|k| ch(c, k)));
            vec![
                rgb8("BaseColor", Channel::BaseColor),
                gray8("Metalness", Channel::Metallic),
                gray8("ReflectionRoughness", Channel::Roughness),
                map("Normal", 8, normal(VirtualMap::NormalOpenGl)),
                rgb8("EmissionColor", Channel::Emissive),
                gray8("Opacity", Channel::Opacity),
                gray8("SubsurfaceWeight", Channel::ScatterWeight),
                gray8("CoatWeight", Channel::CoatWeight),
                gray8("CoatRoughness", Channel::CoatRoughness),
                gray8("SheenWeight", Channel::SheenWeight),
                rgb8("SheenColor", Channel::SheenColor),
                gray8("SheenRoughness", Channel::SheenRoughness),
            ]
        }),
        // Spec: https://create.roblox.com/docs/art/modeling/surface-appearance (ColorMap alpha is opacity)
        // Spec: https://create.roblox.com/docs/art/modeling/texture-specifications (OpenGL normals only; grayscale metalness and roughness)
        // Spec: https://create.roblox.com/docs/art/modeling/reimport (the file name suffix it assigns to each map)
        preset(
            "Roblox SurfaceAppearance (separate maps)",
            vec![
                map("Color", 8, base_with_alpha()),
                map("Normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("Metalness", 8, gray(ch(Channel::Metallic, 0))),
                map("Roughness", 8, gray(ch(Channel::Roughness, 0))),
            ],
        ),
        // Spec: https://help.sketchfab.com/en/articles/16152630-pbr-maps (sRGB base color, linear grayscale metalness; no alpha, the docs don't say opacity reads it)
        // Spec: https://help.sketchfab.com/en/articles/16152634-materials-pbr (roughness, AO and emission channels)
        // Spec: https://help.sketchfab.com/en/articles/16152621-normal-map-bump-map (DirectX normals by default)
        preset(
            "Sketchfab Metalness (separate maps)",
            vec![
                map("BaseColor", 8, rgb(|k| ch(Channel::BaseColor, k))),
                map("Metalness", 8, gray(ch(Channel::Metallic, 0))),
                map("Roughness", 8, gray(ch(Channel::Roughness, 0))),
                map("Normal", 8, normal(VirtualMap::NormalDirectX)),
                map("AmbientOcclusion", 8, gray(virt(VirtualMap::AoMixed, 0))),
                map("Emission", 8, rgb(|k| ch(Channel::Emissive, k))),
            ],
        ),
        preset("Separate maps", Channel::ALL.into_iter().map(channel_map).collect()),
        preset(
            "Mesh maps",
            BakedMap::ALL
                .into_iter()
                .map(|m| {
                    let src = |component| SourceRef::Baked { map: m, component };
                    map("$channel", 16, if m.components() == 3 { rgb(src) } else { gray(src(0)) })
                })
                .collect(),
        ),
    ]
}

/// The default export template.
pub const DEFAULT_PRESET: &str = "Document channels + Normal + AO (No Alpha)";

/// The default template's name before it took the reference's "(No Alpha)" (saved settings and
/// scripts still use it).
const OLD_DEFAULT_PRESET: &str = "Document channels + Normal + AO";

/// Names introduced with the export update, which could previously belong to saved user presets.
pub const NEW_BUILTIN_PRESET_NAMES: [&str; 5] = [
    "Unreal Engine Subsurface Profile (Packed)",
    "glTF Specular Glossiness (archived extension)",
    "Redshift Standard Material (separate maps)",
    "Roblox SurfaceAppearance (separate maps)",
    "Sketchfab Metalness (separate maps)",
];

/// The immutable built-in templates shared by menu and command lookups.
pub fn builtin_preset_catalog() -> &'static [ExportPreset] {
    static CATALOG: std::sync::LazyLock<Vec<ExportPreset>> = std::sync::LazyLock::new(builtin_presets);
    &CATALOG
}

/// A built-in preset by name (case-insensitive).
pub fn builtin_preset(name: &str) -> Option<ExportPreset> {
    let name = name.trim();
    let name = if name.eq_ignore_ascii_case(OLD_DEFAULT_PRESET) { DEFAULT_PRESET } else { name };

    builtin_preset_catalog().iter().find(|preset| preset.name.eq_ignore_ascii_case(name)).cloned()
}

/// Parse and validate a user preset.
pub fn preset_from_json(json: &str) -> Result<ExportPreset> {
    if json.len() > MAX_PRESET_JSON {
        return Err(FormatError::TooLarge("export preset JSON over 1 MiB".into()));
    }
    let p: ExportPreset = serde_json::from_str(json).map_err(|e| crate::malformed("export preset", e))?;
    p.validate()?;
    Ok(p)
}

pub fn preset_to_json(preset: &ExportPreset) -> Result<String> {
    serde_json::to_string_pretty(preset).map_err(|e| FormatError::Invalid(e.to_string()))
}

/// Read a user preset file (size-capped, validated).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_preset(path: &std::path::Path) -> Result<ExportPreset> {
    let bytes = crate::read_capped(path, MAX_PRESET_JSON as u64)?;
    preset_from_json(&String::from_utf8_lossy(&bytes))
}
