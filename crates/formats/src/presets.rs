//! Built-in export presets and user preset JSON. The reference app's built-in output templates
//! come first, with its names, file names and channel packing; a few of our own follow.

use texturecraft_core::{BakedMap, Channel, ChannelKind};

use crate::export::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef, VirtualMap};
use crate::{FormatError, Result};

mod reference_templates;
#[cfg(test)]
mod tests;

/// Largest preset JSON accepted.
const MAX_PRESET_JSON: usize = 1 << 20;

pub(crate) fn ch(channel: Channel, component: u8) -> SourceRef {
    SourceRef::Channel { channel, component }
}

pub(crate) fn virt(map: VirtualMap, component: u8) -> SourceRef {
    SourceRef::Virtual { map, component }
}

fn baked(map: BakedMap, component: u8) -> SourceRef {
    SourceRef::Baked { map, component }
}

/// One output map of a template.
fn output(file: &str, format: FileFormat, bit_depth: u8, dither: bool, channels: Vec<(Dest, SourceRef)>) -> MapSpec {
    MapSpec { file: file.into(), format, bit_depth, channels, inverted: Vec::new(), dither }
}

/// `$mesh_$textureSet_<suffix>` PNG.
fn map(suffix: &str, bit_depth: u8, channels: Vec<(Dest, SourceRef)>) -> MapSpec {
    output(&format!("$mesh_$textureSet_{suffix}"), FileFormat::Png, bit_depth, false, channels)
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
    let spec = |stem: &str, channels| output(&file(stem), FileFormat::Png, 8, false, channels);
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

/// The reference's first channels of the document channel templates, in its order (its order of
/// the channels it did not list is not known; ours follow in [`Channel::ALL`] order)
const DOCUMENT_CHANNELS_FIRST: [Channel; 7] =
    [Channel::BaseColor, Channel::Height, Channel::Roughness, Channel::Metallic, Channel::Normal, Channel::Opacity, Channel::Emissive];

/// "Document channels + Normal + AO": one map per texture set channel (named after its label),
/// the DirectX normal and the mixed AO; `alpha` adds the UV coverage as each map's alpha.
fn document_channel_maps(alpha: bool) -> Vec<MapSpec> {
    let with_alpha = |mut channels: Vec<(Dest, SourceRef)>| {
        if alpha {
            channels.push((Dest::A, virt(VirtualMap::UvCoverage, 0)));
        }
        channels
    };

    // The mixed AO map stands for the AO channel
    let rest = Channel::ALL.into_iter().filter(|c| !DOCUMENT_CHANNELS_FIRST.contains(c) && *c != Channel::AmbientOcclusion);
    let mut maps: Vec<MapSpec> = DOCUMENT_CHANNELS_FIRST
        .into_iter()
        .chain(rest)
        .map(|c| {
            let (channels, bit_depth) = match c.kind() {
                ChannelKind::Color => (rgb(|k| ch(c, k)), 8),
                ChannelKind::NormalMap => (rgb(|k| ch(c, k)), 16),
                ChannelKind::Signed => (gray(ch(c, 0)), 16),
                ChannelKind::Scalar => (gray(ch(c, 0)), 8),
            };
            output(&format!("$textureSet_{}", c.label().replace(' ', "_")), FileFormat::Png, bit_depth, false, with_alpha(channels))
        })
        .collect();
    maps.push(output("$textureSet_Normal_DirectX", FileFormat::Png, 16, false, with_alpha(normal(VirtualMap::NormalDirectX))));
    maps.push(output("$textureSet_Mixed_AO", FileFormat::Png, 8, false, with_alpha(gray(virt(VirtualMap::AoMixed, 0)))));
    maps
}

/// The glTF templates' maps (the reference's displacement variant writes the same ones), plus our
/// clear coat and sheen extension maps where the texture set has those channels.
fn gltf_template_maps() -> Vec<MapSpec> {
    let png = |file: &str, dither: bool, channels| output(file, FileFormat::Png, 8, dither, channels);
    let mut sheen = rgb(|k| virt(VirtualMap::SheenTint, k));
    sheen.push((Dest::A, ch(Channel::SheenRoughness, 0)));
    vec![
        png("$textureSet_baseColor", false, base_with_alpha()),
        png("$textureSet_occlusionRoughnessMetallic", false, orm()),
        png("$textureSet_normal", true, normal(VirtualMap::NormalOpenGl)),
        png("$textureSet_emissive", false, rgb(|k| ch(Channel::Emissive, k))),
        png("$textureSet_clearcoat", false, vec![(Dest::R, ch(Channel::CoatWeight, 0)), (Dest::G, ch(Channel::CoatRoughness, 0))]),
        png("$textureSet_sheen", false, sheen),
    ]
}

/// The Sketchfab template's maps (the reference writes them into a folder of its upload; here
/// they go beside the others).
fn sketchfab_maps() -> Vec<MapSpec> {
    let jpeg = |file: &str, channels| output(file, FileFormat::Jpeg, 8, false, channels);
    vec![
        jpeg("$textureSet_albedo", rgb(|k| ch(Channel::BaseColor, k))),
        jpeg("$textureSet_opacity", gray(ch(Channel::Opacity, 0))),
        jpeg("$textureSet_emissive", rgb(|k| ch(Channel::Emissive, k))),
        jpeg("$textureSet_roughness", gray(ch(Channel::Roughness, 0))),
        jpeg("$textureSet_metallic", gray(ch(Channel::Metallic, 0))),
        output("$textureSet_normal", FileFormat::Png, 8, true, normal(VirtualMap::NormalOpenGl)),
    ]
}

fn usdz_maps() -> Vec<MapSpec> {
    let jpeg = |file: &str, channels| output(file, FileFormat::Jpeg, 8, false, channels);
    vec![
        output("$textureSet_baseColor", FileFormat::Png, 8, false, base_with_alpha()),
        jpeg("$textureSet_roughness", gray(ch(Channel::Roughness, 0))),
        jpeg("$textureSet_metallic", gray(ch(Channel::Metallic, 0))),
        output("$textureSet_normal", FileFormat::Png, 8, true, normal(VirtualMap::NormalOpenGl)),
        jpeg("$textureSet_emissive", rgb(|k| ch(Channel::Emissive, k))),
    ]
}

/// Every built-in preset, in menu order: the reference's templates that follow the texture set's
/// channels (without "2D View", which needs a rendered 2D view), its static templates, then ours.
pub fn builtin_presets() -> Vec<ExportPreset> {
    let preset = |name: &str, maps: Vec<MapSpec>| ExportPreset { name: name.into(), maps };
    let mut presets = vec![
        preset(DEFAULT_PRESET, document_channel_maps(false)),
        preset("Document channels + Normal + AO (With Alpha)", document_channel_maps(true)),
        preset("glTF PBR Metal Roughness", gltf_template_maps()),
        preset("glTF PBR Metal Roughness + Displacement texture (experimental)", gltf_template_maps()),
        preset("Sketchfab", sketchfab_maps()),
        preset("USDz (Apple AR)", usdz_maps()),
    ];
    presets.extend(reference_templates::static_templates());
    presets.extend([
        preset(
            "Godot 4 (ORM)",
            vec![
                map("albedo", 8, base_with_alpha()),
                map("orm", 8, orm()),
                map("normal", 8, normal(VirtualMap::NormalOpenGl)),
                map("emission", 8, rgb(|k| ch(Channel::Emissive, k))),
            ],
        ),
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
        preset("Separate maps", Channel::ALL.into_iter().map(channel_map).collect()),
    ]);
    presets
}

/// The default export template.
pub const DEFAULT_PRESET: &str = "Document channels + Normal + AO (No Alpha)";

/// Built-in templates that were renamed or replaced by a reference template: (old name, current
/// name). Saved settings and scripts still use the old names; the old "Mesh maps" finds "Mesh Maps"
/// because names compare ignoring case.
const RENAMED_PRESETS: [(&str, &str); 3] = [
    ("Document channels + Normal + AO", DEFAULT_PRESET),
    ("Unity HDRP (Mask Map)", "Unity HD Render Pipeline (Metallic Standard)"),
    ("Unity Standard (Metallic Smoothness)", "Unity Universal Render Pipeline (Metallic Standard)"),
];

/// A built-in preset by name (case-insensitive).
pub fn builtin_preset(name: &str) -> Option<ExportPreset> {
    let name = name.trim();
    let name = RENAMED_PRESETS.iter().find(|(old, _)| old.eq_ignore_ascii_case(name)).map_or(name, |(_, current)| *current);
    builtin_presets().into_iter().find(|p| p.name.eq_ignore_ascii_case(name))
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
