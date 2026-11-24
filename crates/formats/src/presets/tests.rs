//! The built-in templates against the reference app's template listing
//! (`fixtures/reference_templates.json`: names, file names, file types and channel packing).

use serde_json::Value;
use texturecraft_core::{ChannelFormat, Project, TextureSet};

use super::*;
use crate::export::{ExportOptions, list_outputs};

const LISTING: &str = include_str!("fixtures/reference_templates.json");
const STACKS: &str = include_str!("fixtures/reference_generator_stacks.json");

/// The stack the reference resolved its channel-following templates against
const REFERENCE_STACK: [Channel; 7] =
    [Channel::BaseColor, Channel::Height, Channel::Opacity, Channel::Emissive, Channel::Roughness, Channel::Metallic, Channel::Normal];

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_else(|| panic!("no {key} in {value}")).to_string()
}

/// Our source for a reference source (map type, map name, channel read)
fn our_source(kind: &str, name: &str, channel: &str) -> SourceRef {
    // The alpha of a reference map is where the mesh has UVs
    if channel == "A" {
        return virt(VirtualMap::UvCoverage, 0);
    }

    let source = match (kind, name) {
        ("documentMap", "baseColor" | "diffuse") => ch(Channel::BaseColor, 0),
        ("documentMap", "metallic") => ch(Channel::Metallic, 0),
        ("documentMap", "roughness") => ch(Channel::Roughness, 0),
        ("documentMap", "normal") => ch(Channel::Normal, 0),
        ("documentMap", "height") => ch(Channel::Height, 0),
        ("documentMap", "emissive") => ch(Channel::Emissive, 0),
        ("documentMap", "opacity") => ch(Channel::Opacity, 0),
        ("documentMap", "ambientOcclusion") => ch(Channel::AmbientOcclusion, 0),
        ("documentMap", "coatOpacity") => ch(Channel::CoatWeight, 0),
        ("documentMap", "sheenColor") => ch(Channel::SheenColor, 0),
        ("documentMap", "specularLevel") => ch(Channel::SpecularLevel, 0),
        ("documentMap", "specular") => ch(Channel::Specular, 0),
        ("documentMap", "glossiness") => ch(Channel::Glossiness, 0),
        ("documentMap", "anisotropyLevel") => ch(Channel::AnisotropyLevel, 0),
        ("documentMap", "anisotropyAngle") => ch(Channel::AnisotropyAngle, 0),
        ("documentMap", "scattering") => ch(Channel::ScatterWeight, 0),
        ("documentMap", "transmissive") => ch(Channel::Transmission, 0),
        // No such channels here: a map of them alone has no source and is left out
        ("documentMap", "user0" | "user1" | "user2" | "user3") | ("defaultMap", "black") => SourceRef::Constant(0.0),
        ("virtualMap", "Normal_DirectX") => virt(VirtualMap::NormalDirectX, 0),
        ("virtualMap", "Normal_OpenGL") => virt(VirtualMap::NormalOpenGl, 0),
        ("virtualMap", "AO_Mixed") => virt(VirtualMap::AoMixed, 0),
        ("virtualMap", "Glossiness") => virt(VirtualMap::Glossiness, 0),
        ("virtualMap", "Diffuse") => virt(VirtualMap::Diffuse, 0),
        ("virtualMap", "Specular") => virt(VirtualMap::SpecularColor, 0),
        ("virtualMap", "reflection") => virt(VirtualMap::Reflection, 0),
        ("virtualMap", "1/ior") => virt(VirtualMap::InverseIor, 0),
        ("virtualMap", "Glossiness2") => virt(VirtualMap::Glossiness2, 0),
        ("virtualMap", "Unity4Diff") => virt(VirtualMap::LegacyDiffuse, 0),
        ("meshMap", "ambient_occlusion") => baked(BakedMap::AmbientOcclusion, 0),
        ("meshMap", "bent_normals") => baked(BakedMap::BentNormal, 0),
        ("meshMap", "curvature") => baked(BakedMap::Curvature, 0),
        ("meshMap", "height") => baked(BakedMap::Height, 0),
        ("meshMap", "id") => baked(BakedMap::Id, 0),
        ("meshMap", "normal_base") => baked(BakedMap::Normal, 0),
        ("meshMap", "opacity") => baked(BakedMap::Opacity, 0),
        ("meshMap", "position") => baked(BakedMap::Position, 0),
        ("meshMap", "thickness") => baked(BakedMap::Thickness, 0),
        ("meshMap", "world_space_normals") => baked(BakedMap::WorldNormal, 0),
        other => panic!("no source for {other:?}"),
    };

    let component = match channel {
        "G" if source.components() == 3 => 1,
        "B" if source.components() == 3 => 2,
        _ => 0,
    };
    match source {
        SourceRef::Channel { channel, .. } => ch(channel, component),
        SourceRef::Virtual { map, .. } => virt(map, component),
        SourceRef::Baked { map, .. } => baked(map, component),
        constant => constant,
    }
}

/// The output map the reference lists, in our terms: paths are relative to the output folder, and
/// a red channel written with only an alpha is gray with alpha
fn our_map(reference: &Value) -> MapSpec {
    let channels = reference["channels"].as_array().unwrap_or_else(|| panic!("no channels in {reference}"));
    let destinations: Vec<String> = channels.iter().map(|c| c[0].as_str().unwrap_or_default().to_string()).collect();
    let gray_alpha = destinations.iter().all(|d| d == "R" || d == "A") && destinations.iter().any(|d| d == "R");
    let channels = channels
        .iter()
        .map(|c| {
            let field = |i: usize| c[i].as_str().unwrap_or_else(|| panic!("bad channel {c}"));
            let dest = match field(0) {
                "R" if gray_alpha => Dest::L,
                "R" => Dest::R,
                "G" => Dest::G,
                "B" => Dest::B,
                "A" => Dest::A,
                "L" => Dest::L,
                other => panic!("destination {other}"),
            };
            (dest, our_source(field(1), field(2), field(3)))
        })
        .collect();
    let format = FileFormat::from_extension(&text(reference, "format")).unwrap_or_else(|| panic!("format of {reference}"));
    let bit_depth = reference["bitDepth"].as_u64().unwrap_or_default() as u8;
    let dither = reference["dithering"].as_bool().unwrap_or_default();
    output(text(reference, "file").trim_start_matches('/'), format, bit_depth, dither, channels)
}

/// A project whose one texture set has `channels` and `baked`
fn stack_project(channels: &[Channel], baked: &[BakedMap]) -> Project {
    let mut project = Project::new("Reference", "builtin:sphere");
    let mut set = TextureSet::new("Set", 0, 64);
    set.channels = channels.iter().map(|c| (*c, ChannelFormat::default_for(*c))).collect();
    set.baked.extend(baked.iter().copied());
    project.texture_sets.push(set);
    project
}

/// The maps `preset` writes for the one texture set of `project`, as written: without the
/// channel conditions that chose them and without an alpha from a channel the set lacks
fn planned_maps(project: &Project, preset: &ExportPreset) -> Vec<MapSpec> {
    let set = &project.texture_sets[0];
    let written = |(dest, source): &(Dest, SourceRef)| !matches!((dest, source), (Dest::A, SourceRef::Channel { channel, .. }) if !set.has_channel(*channel));
    let planned = list_outputs(project, 0, "mesh", &ExportOptions::new(preset.clone())).unwrap();
    planned
        .iter()
        .map(|file| {
            let map = preset.maps[file.map].clone();
            MapSpec { channels: map.channels.iter().filter(|c| written(c)).cloned().collect(), when_channels: Vec::new(), unless_channels: Vec::new(), ..map }
        })
        .collect()
}

#[test]
fn built_in_templates_match_the_reference_listing() {
    let listing: Value = serde_json::from_str(LISTING).unwrap();
    let templates = listing["templates"].as_array().unwrap();

    // The reference writes the mixed AO without an AO channel; here it needs the baked AO
    let project = stack_project(&REFERENCE_STACK, &[BakedMap::AmbientOcclusion, BakedMap::Normal]);
    assert_eq!(templates.len(), 41);

    for template in templates {
        let name = text(template, "name");
        let Some(preset) = builtin_preset(&name) else {
            assert_eq!(name, "2D View", "only the rendered 2D view is missing");
            continue;
        };

        // Channel-following templates are compared as they resolve for the reference stack
        let ours: Vec<MapSpec> = if template["generator"].as_bool().unwrap_or_default() { planned_maps(&project, &preset) } else { preset.maps.clone() };
        let reference: Vec<MapSpec> = template["maps"].as_array().unwrap().iter().map(our_map).collect();

        assert_eq!(preset.name, name);
        assert_eq!(ours.len(), reference.len(), "{name}");
        for (index, (our, theirs)) in ours.iter().zip(&reference).enumerate() {
            assert_eq!(our, theirs, "{name}, output map {index}");
        }
    }

    // The reference order: its channel-following templates, then the static ones
    let reference_names: Vec<String> = templates.iter().map(|t| text(t, "name")).filter(|n| n != "2D View").collect();
    let names: Vec<String> = builtin_presets().into_iter().map(|p| p.name).take(reference_names.len()).collect();
    assert_eq!(names, reference_names);
}

#[test]
fn old_template_names_find_their_replacements() {
    for (old, current) in [
        ("Document channels + Normal + AO", DEFAULT_PRESET),
        ("Unity HDRP (Mask Map)", "Unity HD Render Pipeline (Metallic Standard)"),
        ("unity standard (metallic smoothness)", "Unity Universal Render Pipeline (Metallic Standard)"),
        ("Mesh maps", "Mesh Maps"),
    ] {
        assert_eq!(builtin_preset(old).map(|p| p.name).as_deref(), Some(current), "{old}");
    }
}

/// Whether the reference writes `map` on a set with `channels`: it lists maps of its user channels
/// (which TextureCraft does not have) and the mixed AO without an AO source, but writes neither
fn written_by_reference(map: &Value, channels: &[Channel]) -> bool {
    let sources: Vec<(String, String)> = map["channels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c[3] != "A")
        .map(|c| (c[1].as_str().unwrap_or_default().to_string(), c[2].as_str().unwrap_or_default().to_string()))
        .collect();
    let user_channel = |name: &str| name.starts_with("user");
    let mixed_ao_without_source = |name: &str| name == "AO_Mixed" && !channels.contains(&Channel::AmbientOcclusion);
    !sources.iter().all(|(_, name)| user_channel(name) || mixed_ao_without_source(name))
}

#[test]
fn channel_following_templates_match_the_reference_on_other_stacks() {
    let listing: Value = serde_json::from_str(STACKS).unwrap();

    for stack in listing["stacks"].as_array().unwrap() {
        let tag = text(stack, "stack");
        let channels: Vec<Channel> = stack["channels"].as_array().unwrap().iter().map(|c| Channel::parse(c.as_str().unwrap()).unwrap()).collect();
        let project = stack_project(&channels, &[]);
        for template in stack["templates"].as_array().unwrap() {
            let name = text(template, "name");
            let Some(preset) = builtin_preset(&name) else {
                assert_eq!(name, "2D View", "{tag}");
                continue;
            };

            let ours = planned_maps(&project, &preset);
            let reference: Vec<MapSpec> = template["maps"].as_array().unwrap().iter().filter(|map| written_by_reference(map, &channels)).map(our_map).collect();

            assert_eq!(ours.len(), reference.len(), "{tag} {name}: {:?}", ours.iter().map(|m| &m.file).collect::<Vec<_>>());
            for (index, (our, theirs)) in ours.iter().zip(&reference).enumerate() {
                assert_eq!(our, theirs, "{tag} {name}, output map {index}");
            }
        }
    }
}
