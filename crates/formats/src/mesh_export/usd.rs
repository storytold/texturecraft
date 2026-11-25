//! USD export of the working mesh: `.usda` (text), `.usdc` (Crate binary) or `.usdz` (a
//! package of a `.usdc` layer and its PNG textures). One `Mesh` prim (points, normals and
//! `primvars:st` per vertex, triangles), one `materialBind` GeomSubset per material when there
//! are several, and a UsdPreviewSurface material per material whose maps (base color,
//! roughness, metallic, normal, emissive, opacity, clear coat and its roughness) are
//! UsdUVTexture shaders. Meters, Y up.

use craft_usd::scene::{SceneError, UsdAttr, UsdLayer, UsdPrim, UsdValue, write_usda, write_usdc};
use texturecraft_core::{Channel, NormalFormat, TextureSet};
use texturecraft_geom::Mesh;

use texturecraft_texel::ColorSpace;

use super::{is_black, parts};
use crate::export::{
    Dest, ExportOptions, ExportPreset, ExportedFile, FileFormat, MapSpec, PlannedFile, SourceRef, VirtualMap, sanitize_file_stem, source_present,
};
use crate::presets::{ch, gray, rgb, virt};
use crate::{FormatError, Result};

/// The three USD encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsdFormat {
    Usda,
    Usdc,
    Usdz,
}

impl UsdFormat {
    /// From a file extension (`usda`, `usdc`, `usdz`; `usd` is the binary).
    pub fn from_ext(ext: &str) -> Option<UsdFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "usda" => Some(UsdFormat::Usda),
            "usdc" | "usd" => Some(UsdFormat::Usdc),
            "usdz" => Some(UsdFormat::Usdz),
            _ => None,
        }
    }
}

/// A USD export: the layer or package, and (usda / usdc) the textures it references by
/// relative file name, to be written next to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsdExport {
    pub bytes: Vec<u8>,
    pub textures: Vec<(String, Vec<u8>)>,
}

/// A UsdPreviewSurface input fed by a texture: (file stem, input, input type, texture output,
/// sRGB).
pub(crate) const SLOTS: [(&str, &str, &str, &str, bool); 8] = [
    ("BaseColor", "diffuseColor", "color3f", "rgb", true),
    ("Roughness", "roughness", "float", "r", false),
    ("Metallic", "metallic", "float", "r", false),
    ("Normal", "normal", "normal3f", "rgb", false),
    ("Emissive", "emissiveColor", "color3f", "rgb", true),
    ("Opacity", "opacity", "float", "r", false),
    ("Clearcoat", "clearcoat", "float", "r", false),
    ("ClearcoatRoughness", "clearcoatRoughness", "float", "r", false),
];

/// Texture export options for [`usd_bytes`]: one 8-bit PNG per UsdPreviewSurface input, named
/// by slot (maps the texture set has no data for are not produced).
pub fn usd_export_options() -> ExportOptions {
    let maps = SLOTS
        .iter()
        .map(|&(stem, ..)| {
            let channels = match stem {
                "BaseColor" => rgb(|k| ch(Channel::BaseColor, k)),
                "Roughness" => gray(ch(Channel::Roughness, 0)),
                "Metallic" => gray(ch(Channel::Metallic, 0)),
                "Normal" => rgb(|k| virt(VirtualMap::NormalOpenGl, k)),
                "Emissive" => rgb(|k| ch(Channel::Emissive, k)),
                "Clearcoat" => gray(ch(Channel::CoatWeight, 0)),
                "ClearcoatRoughness" => gray(ch(Channel::CoatRoughness, 0)),
                _ => gray(ch(Channel::Opacity, 0)),
            };
            MapSpec { file: stem.into(), format: FileFormat::Png, bit_depth: 8, channels, inverted: Vec::new() }
        })
        .collect();
    super::png8_options(ExportPreset { name: "USD preview surface".into(), maps }, NormalFormat::OpenGl)
}

/// Map a `craft-usd` scene error into [`FormatError`].
fn map_scene_err(e: SceneError) -> FormatError {
    match e {
        SceneError::Malformed(m) => FormatError::Invalid(m),
        SceneError::TooLarge(m) => FormatError::TooLarge(m),
        SceneError::Unsupported(m) => FormatError::Unsupported(m),
        SceneError::Invalid(m) => FormatError::Invalid(m),
    }
}

/// A valid, unique prim name for `s`.
fn identifier(s: &str, taken: &mut Vec<String>) -> String {
    let mut id: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if !id.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        id.insert(0, '_');
    }
    let base = id.clone();
    let mut k = 1;
    while taken.contains(&id) {
        id = format!("{base}_{k}");
        k += 1;
    }
    taken.push(id.clone());
    id
}

/// The export scene: `/<name>` with the `Mesh` and `Materials`. `material(m, path, id)` makes the
/// Material prim `id` at `path` for mesh material `m`.
fn scene(mesh: &Mesh, name: &str, material: impl Fn(usize, &str, &str) -> UsdPrim) -> Result<UsdLayer> {
    let vertex_count = mesh.positions.len();
    if vertex_count == 0 || mesh.indices.len() < 3 {
        return Err(FormatError::Invalid("the mesh is empty".into()));
    }

    let ints =
        |v: &[u32]| v.iter().map(|&i| i32::try_from(i).map_err(|_| FormatError::TooLarge("vertex index beyond 2^31".into()))).collect::<Result<Vec<i32>>>();
    let mut taken = Vec::new();
    let root_name = identifier(name, &mut taken);
    let mut root = UsdPrim::new(&root_name, "Xform");
    let mut geometry = UsdPrim::new(&identifier(&format!("{name}_mesh"), &mut taken), "Mesh");
    let material_names: Vec<String> = mesh.materials.iter().map(|m| identifier(m, &mut taken)).collect();
    let material_path = |m: usize| material_names.get(m).map(|id| format!("/{root_name}/Materials/{id}"));
    let (lo, hi) = mesh.bounds();

    // Faces in submesh order; subsets index them
    let mut indices: Vec<u32> = Vec::with_capacity(mesh.indices.len());
    let mut faces_of: Vec<Vec<i32>> = vec![Vec::new(); mesh.materials.len()];
    for part in parts(mesh, name) {
        let range = mesh.indices.get(part.start as usize..(part.start as usize).saturating_add(part.count as usize)).unwrap_or_default();
        for t in range.as_chunks::<3>().0 {
            if let Some(f) = faces_of.get_mut(part.material) {
                f.push(i32::try_from(indices.len() / 3).map_err(|_| FormatError::TooLarge("triangle count".into()))?);
            }
            indices.extend_from_slice(t);
        }
    }

    let vertex = |a: UsdAttr| a.with_interpolation("vertex");
    geometry.attrs = vec![
        UsdAttr::new("extent", "float3[]", Some(UsdValue::Vec3fs(vec![lo.to_array(), hi.to_array()]))),
        UsdAttr::new("faceVertexCounts", "int[]", Some(UsdValue::Ints(vec![3; indices.len() / 3]))),
        UsdAttr::new("faceVertexIndices", "int[]", Some(UsdValue::Ints(ints(&indices)?))),
        UsdAttr::new("points", "point3f[]", Some(UsdValue::Vec3fs(mesh.positions.iter().map(|p| p.to_array()).collect()))),
        UsdAttr::new("subdivisionScheme", "token", Some(UsdValue::Token("none".into()))).uniform(),
    ];
    if mesh.normals.len() == vertex_count {
        geometry.attrs.push(vertex(UsdAttr::new("normals", "normal3f[]", Some(UsdValue::Vec3fs(mesh.normals.iter().map(|v| v.to_array()).collect())))));
    }
    if mesh.uvs.len() == vertex_count {
        // USD's st origin is bottom-left
        let st = mesh.uvs.iter().map(|t| [t.x, 1.0 - t.y]).collect();
        geometry.attrs.push(vertex(UsdAttr::new("primvars:st", "texCoord2f[]", Some(UsdValue::Vec2fs(st)))));
    }

    let used: Vec<usize> = (0..mesh.materials.len()).filter(|m| faces_of.get(*m).is_some_and(|f| !f.is_empty())).collect();
    match used.as_slice() {
        [] => {}
        [m] => {
            geometry.binding_api = true;
            geometry.attrs.extend(material_path(*m).map(|p| UsdAttr::rel("material:binding", p)));
        }
        _ => {
            for &m in &used {
                let (Some(id), Some(path)) = (material_names.get(m), material_path(m)) else { continue };
                let mut subset = UsdPrim::new(id, "GeomSubset");
                subset.binding_api = true;
                subset.attrs = vec![
                    UsdAttr::new("elementType", "token", Some(UsdValue::Token("face".into()))).uniform(),
                    UsdAttr::new("familyName", "token", Some(UsdValue::Token("materialBind".into()))).uniform(),
                    UsdAttr::new("indices", "int[]", Some(UsdValue::Ints(faces_of.get(m).cloned().unwrap_or_default()))),
                    UsdAttr::rel("material:binding", path),
                ];
                geometry.children.push(subset);
            }
        }
    }

    root.children.push(geometry);
    let mut materials = UsdPrim::new("Materials", "Scope");
    for (m, id) in material_names.iter().enumerate() {
        materials.children.push(material(m, &format!("/{root_name}/Materials/{id}"), id));
    }
    if !materials.children.is_empty() {
        root.children.push(materials);
    }

    let mut layer = UsdLayer::new(root);
    layer.doc = Some("Exported by TextureCraft".into());
    Ok(layer)
}

/// A UsdPreviewSurface material at `path` whose slot inputs read `files` (slot, file name).
fn material(path: &str, id: &str, files: &[(usize, String)]) -> UsdPrim {
    let mut mat = UsdPrim::new(id, "Material");
    mat.attrs.push(UsdAttr::connected("outputs:surface", "token", format!("{path}/PBR.outputs:surface")));
    let mut pbr = UsdPrim::new("PBR", "Shader");
    pbr.attrs.push(UsdAttr::new("info:id", "token", Some(UsdValue::Token("UsdPreviewSurface".into()))).uniform());
    let mut textures = Vec::new();
    for (slot, file) in files {
        let Some(&(stem, input, input_type, output, srgb)) = SLOTS.get(*slot) else { continue };
        pbr.attrs.push(UsdAttr::connected(&format!("inputs:{input}"), input_type, format!("{path}/{stem}.outputs:{output}")));
        let mut tex = UsdPrim::new(stem, "Shader");
        let token = |s: &str| Some(UsdValue::Token(s.into()));
        tex.attrs = vec![
            UsdAttr::new("info:id", "token", token("UsdUVTexture")).uniform(),
            UsdAttr::new("inputs:file", "asset", Some(UsdValue::Asset(format!("./{file}")))),
            UsdAttr::connected("inputs:st", "float2", format!("{path}/st.outputs:result")),
            UsdAttr::new("inputs:sourceColorSpace", "token", token(if srgb { "sRGB" } else { "raw" })),
            UsdAttr::new("inputs:wrapS", "token", token("repeat")),
            UsdAttr::new("inputs:wrapT", "token", token("repeat")),
        ];
        if stem == "Normal" {
            // Unpack 0..1 texels to -1..1 vectors.
            tex.attrs.push(UsdAttr::new("inputs:scale", "float4", Some(UsdValue::Float4([2.0, 2.0, 2.0, 1.0]))));
            tex.attrs.push(UsdAttr::new("inputs:bias", "float4", Some(UsdValue::Float4([-1.0, -1.0, -1.0, 0.0]))));
        }
        tex.attrs.push(UsdAttr::new(&format!("outputs:{output}"), if output == "rgb" { "float3" } else { "float" }, None));
        textures.push(tex);
    }
    pbr.attrs.push(UsdAttr::new("inputs:useSpecularWorkflow", "int", Some(UsdValue::Int(0))));
    pbr.attrs.push(UsdAttr::new("outputs:surface", "token", None));
    mat.children.push(pbr);
    if !textures.is_empty() {
        let mut st = UsdPrim::new("st", "Shader");
        st.attrs = vec![
            UsdAttr::new("info:id", "token", Some(UsdValue::Token("UsdPrimvarReader_float2".into()))).uniform(),
            // A string, not a token: usdchecker's ARKit profile refuses a token here.
            UsdAttr::new("inputs:varname", "string", Some(UsdValue::Str("st".into()))),
            UsdAttr::new("outputs:result", "float2", None),
        ];
        mat.children.push(st);
        mat.children.extend(textures);
    }
    mat
}

/// The mesh as USD named `name`, with `textures[m]` (rendered with [`usd_export_options`]) on
/// material `m`. usda / usdc reference their textures as `<name>_<material>_<slot>.png` next
/// to the layer; usdz embeds them.
pub fn usd_bytes(mesh: &Mesh, name: &str, textures: &[Vec<ExportedFile>], format: UsdFormat) -> Result<UsdExport> {
    let stem = sanitize_file_stem(name);
    let (names, files) = material_maps(mesh, &stem, textures);
    let layer = scene(mesh, name, |m, path, id| material(path, id, names.get(m).map_or(&[][..], Vec::as_slice)))?;
    Ok(match format {
        UsdFormat::Usda => UsdExport { bytes: write_usda(&layer).into_bytes(), textures: files },
        UsdFormat::Usdc => UsdExport { bytes: write_usdc(&layer).map_err(map_scene_err)?, textures: files },
        UsdFormat::Usdz => UsdExport { bytes: usdz(&format!("{stem}.usdc"), &write_usdc(&layer).map_err(map_scene_err)?, &files)?, textures: Vec::new() },
    })
}

/// Files by name, with their bytes.
pub type NamedFiles = Vec<(String, Vec<u8>)>;

/// The maps of each material worth writing (`textures[m]`, rendered with
/// [`usd_export_options`]): per material its [`SLOTS`] indices and file names
/// (`<stem>_<material>_<slot>.png`), and the files with their bytes.
pub(crate) fn material_maps(mesh: &Mesh, stem: &str, textures: &[Vec<ExportedFile>]) -> (Vec<Vec<(usize, String)>>, NamedFiles) {
    let mut files = Vec::new();
    let mut names = Vec::with_capacity(mesh.materials.len());
    for (m, material) in super::unique_ids(&mesh.materials, sanitize_file_stem).iter().enumerate() {
        let mut slots = Vec::new();
        let maps = textures.get(m).map_or(&[][..], Vec::as_slice);
        // Without a coat its roughness means nothing.
        let no_coat = maps.iter().find(|f| f.name == "Clearcoat.png").is_none_or(|f| is_black(&f.bytes));
        for f in maps {
            let slot_stem = f.name.strip_suffix(".png").unwrap_or_default();
            let Some(slot) = SLOTS.iter().position(|s| s.0 == slot_stem) else { continue };
            let skip = match slot_stem {
                "Emissive" => is_black(&f.bytes),
                "Clearcoat" | "ClearcoatRoughness" => no_coat,
                _ => false,
            };
            if skip {
                continue;
            }
            let file = format!("{stem}_{material}_{slot_stem}.png");
            slots.push((slot, file.clone()));
            files.push((file, f.bytes.clone()));
        }
        names.push(slots);
    }
    (names, files)
}

/// A usdz package: the layer first, every entry stored uncompressed at a 64-byte aligned offset.
pub(crate) fn usdz(layer_name: &str, layer: &[u8], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut zip = craft_zip_usdz::Writer::new();
    let err = |e| FormatError::Invalid(format!("usdz package: {e}"));
    for (name, bytes) in std::iter::once((layer_name, layer)).chain(files.iter().map(|(n, b)| (n.as_str(), b.as_slice()))) {
        zip.add_stored_aligned(name, bytes, 64).map_err(err)?;
    }
    zip.finish().map_err(err)
}

/// A UsdPreviewSurface input that reads a texture file the export wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct UsdTextureBinding {
    /// The input, as an index into the table of UsdPreviewSurface inputs (`SLOTS`).
    slot: usize,
    /// The file name, `<UDIM>` for a UV-tiled set's tiles.
    file: String,
    /// The texture output the input reads: `r`, `g`, `b`, `a` or `rgb`.
    output: &'static str,
    /// The file holds sRGB-encoded colour.
    srgb: bool,
    /// A normal map: the green convention of its file.
    normal: Option<NormalFormat>,
}

/// Whether `source` is what UsdPreviewSurface input `slot` (an index into [`SLOTS`]) reads.
fn feeds_slot(slot: usize, source: &SourceRef) -> bool {
    let channel = match (slot, source) {
        (1, SourceRef::Virtual { map: VirtualMap::Roughness, .. }) => return true,
        (3, SourceRef::Virtual { map: VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX, .. }) => return true,
        (_, SourceRef::Channel { channel, .. }) => *channel,
        _ => return false,
    };
    let wanted = match slot {
        0 => Channel::BaseColor,
        1 => Channel::Roughness,
        2 => Channel::Metallic,
        3 => Channel::Normal,
        4 => Channel::Emissive,
        5 => Channel::Opacity,
        6 => Channel::CoatWeight,
        _ => Channel::CoatRoughness,
    };
    channel == wanted
}

fn component_of(source: &SourceRef) -> u8 {
    match source {
        SourceRef::Channel { component, .. } | SourceRef::Virtual { component, .. } | SourceRef::Baked { component, .. } => *component,
        SourceRef::Constant(_) => 0,
    }
}

/// The texture output of `map` that input `slot` reads, and the source it takes: a colour or
/// normal input needs the three components in R, G and B (the source is the green one's), a scalar
/// input reads the destination that holds it. None when the map does not hold the input, when `set` has no data for it (the
/// file then holds a default), when it is written inverted, or when `format` cannot keep it: JPEG
/// and HDR files drop alpha, a PGM file holds one gray channel (a colour map becomes its
/// luminance), which only a map written as L can bind.
fn output_for_slot<'m>(slot: usize, map: &'m MapSpec, set: &TextureSet, format: FileFormat) -> Option<(&'static str, &'m SourceRef)> {
    let holds = |dest: Dest, component: Option<u8>| {
        map.channels
            .iter()
            .find(|(d, source)| {
                *d == dest
                    && feeds_slot(slot, source)
                    && source_present(set, source)
                    && component.is_none_or(|k| component_of(source) == k)
                    && !map.inverted.contains(d)
            })
            .map(|(_, source)| source)
    };
    if SLOTS[slot].2 == "float" {
        return [(Dest::R, "r"), (Dest::L, "r"), (Dest::G, "g"), (Dest::B, "b"), (Dest::A, "a")]
            .into_iter()
            .filter(|(dest, _)| match format {
                FileFormat::Pgm => *dest == Dest::L,
                FileFormat::Jpeg | FileFormat::Hdr => *dest != Dest::A,
                _ => true,
            })
            .find_map(|(dest, output)| holds(dest, None).map(|source| (output, source)));
    }

    if format == FileFormat::Pgm {
        return None;
    }

    holds(Dest::R, Some(0))?;
    let green = holds(Dest::G, Some(1))?;
    holds(Dest::B, Some(2))?;

    // The green source is returned: only the green channel differs between the OpenGL and DirectX
    // normal conventions
    Some(("rgb", green))
}

/// The UsdPreviewSurface inputs the exported `files` of texture set `set` (planned with `opts`)
/// can feed, by what each map holds: base colour, roughness, metallic, normal, emissive colour,
/// opacity, clear coat and clear coat roughness. Maps with none of them (ambient occlusion,
/// height, specular/glossiness, baked maps...) bind nothing, and neither do channels the set has
/// no data for. `project_normal` is the project's normal convention, used by normal maps that
/// follow it.
pub fn usd_texture_bindings(set: &TextureSet, opts: &ExportOptions, project_normal: NormalFormat, files: &[PlannedFile]) -> Vec<UsdTextureBinding> {
    let mut out: Vec<UsdTextureBinding> = Vec::new();
    let mut seen_maps = Vec::new();
    for file in files {
        // The first tile stands for all the tiles of a map
        if seen_maps.contains(&file.map) {
            continue;
        }

        seen_maps.push(file.map);
        let Some(map) = opts.preset.maps.get(file.map) else { continue };

        // A tile whose name has no `<UDIM>` token cannot stand for the others
        let name = match (&file.udim_name, file.tile) {
            (Some(udim), _) => udim.clone(),
            (None, None) => file.name.clone(),
            (None, Some(_)) => continue,
        };
        let srgb = opts.space_for_map(map, file.format, file.bit_depth) == ColorSpace::Srgb;
        for slot in 0..SLOTS.len() {
            if out.iter().any(|binding| binding.slot == slot) {
                continue;
            }

            let Some((output, source)) = output_for_slot(slot, map, set, file.format) else { continue };
            let normal = (slot == 3).then(|| {
                opts.normal_format.unwrap_or(match source {
                    SourceRef::Virtual { map: VirtualMap::NormalOpenGl, .. } => NormalFormat::OpenGl,
                    SourceRef::Virtual { map: VirtualMap::NormalDirectX, .. } => NormalFormat::DirectX,
                    _ => project_normal,
                })
            });
            out.push(UsdTextureBinding { slot, file: name.clone(), output, srgb, normal });
        }
    }
    out
}

/// The mesh as a layer named `name` whose material `m` reads the textures of `bindings[m]`, by
/// relative file name (`./<file>`).
fn textured_layer(mesh: &Mesh, name: &str, bindings: &[Vec<UsdTextureBinding>]) -> Result<UsdLayer> {
    scene(mesh, name, |m, path, id| textured_material(path, id, bindings.get(m).map_or(&[][..], Vec::as_slice)))
}

/// The mesh as a binary USD (Crate) file named `name` whose material `m` reads the textures of
/// `bindings[m]`, by relative file name (`./<file>`): the textures are not part of the result.
pub fn usd_layer_with_textures(mesh: &Mesh, name: &str, bindings: &[Vec<UsdTextureBinding>]) -> Result<Vec<u8>> {
    write_usdc(&textured_layer(mesh, name, bindings)?).map_err(map_scene_err)
}

/// A UsdPreviewSurface material at `path` whose inputs read `bindings`. One texture shader reads
/// each file (once per colour space and normal convention), so inputs packed into one file read
/// different outputs of it.
fn textured_material(path: &str, id: &str, bindings: &[UsdTextureBinding]) -> UsdPrim {
    let mut mat = UsdPrim::new(id, "Material");
    mat.attrs.push(UsdAttr::connected("outputs:surface", "token", format!("{path}/PBR.outputs:surface")));
    let mut pbr = UsdPrim::new("PBR", "Shader");
    pbr.attrs.push(UsdAttr::new("info:id", "token", Some(UsdValue::Token("UsdPreviewSurface".into()))).uniform());

    // (shader name, a binding that has its file, colour space and normal convention, outputs read)
    let mut taken = vec!["PBR".to_string(), "st".to_string()];
    let mut shaders: Vec<(String, &UsdTextureBinding, Vec<&'static str>)> = Vec::new();
    for binding in bindings {
        let Some(&(_, input, input_type, ..)) = SLOTS.get(binding.slot) else { continue };
        let at = match shaders.iter().position(|(_, key, _)| (&key.file, key.srgb, key.normal) == (&binding.file, binding.srgb, binding.normal)) {
            Some(at) => at,
            None => {
                let stem = binding.file.rsplit_once('.').map_or(binding.file.as_str(), |(stem, _)| stem);
                shaders.push((identifier(stem, &mut taken), binding, Vec::new()));
                shaders.len() - 1
            }
        };
        let (shader, _, outputs) = &mut shaders[at];
        if !outputs.contains(&binding.output) {
            outputs.push(binding.output);
        }

        pbr.attrs.push(UsdAttr::connected(&format!("inputs:{input}"), input_type, format!("{path}/{shader}.outputs:{}", binding.output)));
    }

    let mut textures = Vec::new();
    for (shader, binding, outputs) in shaders {
        let mut tex = UsdPrim::new(&shader, "Shader");
        let token = |s: &str| Some(UsdValue::Token(s.into()));
        tex.attrs = vec![
            UsdAttr::new("info:id", "token", token("UsdUVTexture")).uniform(),
            UsdAttr::new("inputs:file", "asset", Some(UsdValue::Asset(format!("./{}", binding.file)))),
            UsdAttr::connected("inputs:st", "float2", format!("{path}/st.outputs:result")),
            UsdAttr::new("inputs:sourceColorSpace", "token", token(if binding.srgb { "sRGB" } else { "raw" })),
            UsdAttr::new("inputs:wrapS", "token", token("repeat")),
            UsdAttr::new("inputs:wrapT", "token", token("repeat")),
        ];

        // Unpack 0..1 texels to -1..1 vectors; a DirectX file also flips green to the OpenGL convention
        if let Some(format) = binding.normal {
            let (scale, bias) = match format {
                NormalFormat::OpenGl => ([2.0, 2.0, 2.0, 1.0], [-1.0, -1.0, -1.0, 0.0]),
                NormalFormat::DirectX => ([2.0, -2.0, 2.0, 1.0], [-1.0, 1.0, -1.0, 0.0]),
            };
            tex.attrs.push(UsdAttr::new("inputs:scale", "float4", Some(UsdValue::Float4(scale))));
            tex.attrs.push(UsdAttr::new("inputs:bias", "float4", Some(UsdValue::Float4(bias))));
        }
        for output in outputs {
            tex.attrs.push(UsdAttr::new(&format!("outputs:{output}"), if output == "rgb" { "float3" } else { "float" }, None));
        }
        textures.push(tex);
    }
    pbr.attrs.push(UsdAttr::new("inputs:useSpecularWorkflow", "int", Some(UsdValue::Int(0))));
    pbr.attrs.push(UsdAttr::new("outputs:surface", "token", None));
    mat.children.push(pbr);
    if !textures.is_empty() {
        let mut st = UsdPrim::new("st", "Shader");
        st.attrs = vec![
            UsdAttr::new("info:id", "token", Some(UsdValue::Token("UsdPrimvarReader_float2".into()))).uniform(),
            // A string, not a token: usdchecker's ARKit profile refuses a token here.
            UsdAttr::new("inputs:varname", "string", Some(UsdValue::Str("st".into()))),
            UsdAttr::new("outputs:result", "float2", None),
        ];
        mat.children.push(st);
        mat.children.extend(textures);
    }
    mat
}

#[cfg(test)]
mod aligned_package_tests {
    use super::*;
    use std::io::{Cursor, Read, Write};

    #[test]
    fn shared_usdz_keeps_layer_first_and_exact_archive_bytes() {
        let files = vec![("textures/color.png".to_owned(), vec![0, 1, 255]), ("empty".to_owned(), Vec::new())];
        let bytes = usdz("scene.usdc", b"PXR-USDC", &files).unwrap();
        let mut previous = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).with_alignment(64);
        for (name, data) in std::iter::once(("scene.usdc", b"PXR-USDC".as_slice())).chain(files.iter().map(|(n, b)| (n.as_str(), b.as_slice()))) {
            previous.start_file(name, opts).unwrap();
            previous.write_all(data).unwrap();
        }
        assert_eq!(bytes, previous.finish().unwrap().into_inner());
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 3);
        for (i, (name, data)) in
            std::iter::once(("scene.usdc", b"PXR-USDC".as_slice())).chain(files.iter().map(|(n, b)| (n.as_str(), b.as_slice()))).enumerate()
        {
            let mut entry = archive.by_index(i).unwrap();
            assert_eq!(entry.name().unwrap(), name);
            assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
            assert_eq!(entry.data_start().unwrap() % 64, 0);
            let mut actual = Vec::new();
            entry.read_to_end(&mut actual).unwrap();
            assert_eq!(actual, data);
        }
    }

    #[test]
    fn shared_usdz_rejects_duplicate_layer_name() {
        let err = usdz("scene.usdc", b"PXR-USDC", &[("scene.usdc".to_owned(), Vec::new())]).unwrap_err();
        assert!(matches!(err, FormatError::Invalid(ref why) if why.starts_with("usdz package:")));
    }
}

#[cfg(test)]
mod texture_binding_tests {
    use super::*;
    use texturecraft_core::ChannelFormat;

    const BASE_COLOR: usize = 0;
    const ROUGHNESS: usize = 1;
    const METALLIC: usize = 2;
    const NORMAL: usize = 3;
    const OPACITY: usize = 5;

    fn set_with(extra: &[Channel]) -> TextureSet {
        let mut set = TextureSet::new("Body", 0, 128);
        set.channels.extend(extra.iter().map(|c| (*c, ChannelFormat::default_for(*c))));
        set
    }

    fn map(file: &str, format: FileFormat, channels: Vec<(Dest, SourceRef)>) -> MapSpec {
        MapSpec { file: file.into(), format, bit_depth: 8, channels, inverted: Vec::new() }
    }

    fn options(maps: Vec<MapSpec>) -> ExportOptions {
        ExportOptions::new(ExportPreset { name: "test".into(), maps })
    }

    fn planned(opts: &ExportOptions, index: usize) -> PlannedFile {
        let map = &opts.preset.maps[index];
        PlannedFile {
            name: format!("{}.{}", map.file, map.format.extension()),
            map: index,
            format: map.format,
            bit_depth: map.bit_depth,
            dither: false,
            tile: None,
            udim_name: None,
            warnings: Vec::new(),
        }
    }

    fn bind(set: &TextureSet, opts: &ExportOptions, normal: NormalFormat) -> Vec<UsdTextureBinding> {
        let files: Vec<PlannedFile> = (0..opts.preset.maps.len()).map(|i| planned(opts, i)).collect();
        usd_texture_bindings(set, opts, normal, &files)
    }

    fn slot(bindings: &[UsdTextureBinding], slot: usize) -> Option<&UsdTextureBinding> {
        bindings.iter().find(|b| b.slot == slot)
    }

    fn rgba() -> Vec<(Dest, SourceRef)> {
        vec![
            (Dest::R, ch(Channel::BaseColor, 0)),
            (Dest::G, ch(Channel::BaseColor, 1)),
            (Dest::B, ch(Channel::BaseColor, 2)),
            (Dest::A, ch(Channel::Opacity, 0)),
        ]
    }

    #[test]
    fn colour_and_opacity_of_one_file_are_read_by_one_texture_shader() {
        let opts = options(vec![map("albedo", FileFormat::Png, rgba())]);
        let bindings = bind(&set_with(&[Channel::Opacity]), &opts, NormalFormat::OpenGl);
        assert_eq!(slot(&bindings, BASE_COLOR).map(|b| b.output), Some("rgb"));
        assert_eq!(slot(&bindings, OPACITY).map(|b| (b.output, b.file.as_str())), Some(("a", "albedo.png")));

        // PBR, the st reader and a single texture shader
        assert_eq!(textured_material("/m", "M", &bindings).children.len(), 3);

        // A set without opacity has no alpha in the file
        let bindings = bind(&set_with(&[]), &opts, NormalFormat::OpenGl);
        assert!(slot(&bindings, OPACITY).is_none());
        assert!(slot(&bindings, BASE_COLOR).is_some());
    }

    #[test]
    fn the_layer_reads_each_file_with_its_output_and_colour_space() {
        let opts = options(vec![
            map("albedo", FileFormat::Png, rgba()),
            map("orm", FileFormat::Png, vec![(Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
        ]);
        let bindings = vec![bind(&set_with(&[Channel::Opacity]), &opts, NormalFormat::OpenGl), Vec::new()];
        let mesh = texturecraft_geom::primitives::material_ball();
        assert_eq!(mesh.materials.len(), 2);

        let text = write_usda(&textured_layer(&mesh, "ball", &bindings).unwrap());
        for expected in [
            "asset inputs:file = @./albedo.png@",
            "asset inputs:file = @./orm.png@",
            "color3f inputs:diffuseColor.connect = </ball/Materials/",
            "float inputs:roughness.connect = </ball/Materials/",
            "float inputs:metallic.connect = </ball/Materials/",
            "float inputs:opacity.connect = </ball/Materials/",
        ] {
            assert!(text.contains(expected), "{expected} is missing: {text}");
        }
        assert!(text.contains("/orm.outputs:g>") && text.contains("/orm.outputs:b>") && text.contains("/albedo.outputs:a>"), "{text}");
        assert!(text.contains("token inputs:sourceColorSpace = \"sRGB\"") && text.contains("token inputs:sourceColorSpace = \"raw\""), "{text}");
        assert_eq!(text.matches("\"UsdUVTexture\"").count(), 2, "one texture shader per file: {text}");

        // The binary file holds the same layer
        let bytes = usd_layer_with_textures(&mesh, "ball", &bindings).unwrap();
        assert!(bytes.starts_with(b"PXR-USDC"));
    }

    #[test]
    fn an_inverted_destination_is_not_bound() {
        let mut inverted = map("rough", FileFormat::Png, vec![(Dest::L, ch(Channel::Roughness, 0))]);
        inverted.inverted = vec![Dest::L];
        let plain = map("rough", FileFormat::Png, vec![(Dest::L, ch(Channel::Roughness, 0))]);
        let set = set_with(&[]);
        assert!(slot(&bind(&set, &options(vec![inverted]), NormalFormat::OpenGl), ROUGHNESS).is_none());
        assert!(slot(&bind(&set, &options(vec![plain]), NormalFormat::OpenGl), ROUGHNESS).is_some());
    }

    #[test]
    fn the_tiles_of_a_uv_tiled_set_are_read_through_the_udim_name() {
        let opts = options(vec![map("color.$udim", FileFormat::Png, rgba())]);
        let set = set_with(&[]);
        let mut tile = planned(&opts, 0);
        tile.name = "color.1001.png".into();
        tile.tile = Some(1001);
        let mut second = tile.clone();
        second.name = "color.1002.png".into();
        second.tile = Some(1002);

        // Tiles without a `<UDIM>` name cannot stand for the others
        assert!(usd_texture_bindings(&set, &opts, NormalFormat::OpenGl, &[tile.clone(), second.clone()]).is_empty());

        tile.udim_name = Some("color.<UDIM>.png".into());
        second.udim_name = tile.udim_name.clone();
        let bindings = usd_texture_bindings(&set, &opts, NormalFormat::OpenGl, &[tile, second]);
        assert_eq!(slot(&bindings, BASE_COLOR).map(|b| b.file.as_str()), Some("color.<UDIM>.png"));
        assert_eq!(bindings.len(), 1, "the tiles of one map make one binding");
    }

    #[test]
    fn only_colour_maps_in_integer_srgb_files_are_tagged_srgb() {
        let colour = || vec![(Dest::R, ch(Channel::BaseColor, 0)), (Dest::G, ch(Channel::BaseColor, 1)), (Dest::B, ch(Channel::BaseColor, 2))];
        let set = set_with(&[]);
        let tag = |format: FileFormat, depth: u8, space: Option<ColorSpace>| {
            let mut spec = map("c", format, colour());
            spec.bit_depth = depth;
            let mut opts = options(vec![spec]);
            opts.color_space = space;
            let file = planned(&opts, 0);
            let expected = opts.space_for_map(&opts.preset.maps[0], file.format, file.bit_depth) == ColorSpace::Srgb;
            let bindings = usd_texture_bindings(&set, &opts, NormalFormat::OpenGl, &[PlannedFile { bit_depth: depth, ..file }]);
            let srgb = slot(&bindings, BASE_COLOR).unwrap().srgb;
            assert_eq!(srgb, expected);
            srgb
        };
        assert!(tag(FileFormat::Png, 8, None));
        assert!(tag(FileFormat::Png, 16, None));
        assert!(!tag(FileFormat::Exr, 16, None), "float files hold linear colour");
        assert!(!tag(FileFormat::Png, 8, Some(ColorSpace::AcesCg)), "UsdPreviewSurface knows only sRGB and raw");

        // Data maps are raw
        let data = options(vec![map("m", FileFormat::Png, vec![(Dest::L, ch(Channel::Metallic, 0))])]);
        assert!(!slot(&bind(&set, &data, NormalFormat::OpenGl), METALLIC).unwrap().srgb);
    }

    #[test]
    fn roughness_from_glossiness_is_bound_only_for_a_set_with_glossiness() {
        let from_glossiness = options(vec![map("r", FileFormat::Png, vec![(Dest::L, virt(VirtualMap::Roughness, 0))])]);
        assert!(slot(&bind(&set_with(&[Channel::Glossiness]), &from_glossiness, NormalFormat::OpenGl), ROUGHNESS).is_some());
        assert!(slot(&bind(&set_with(&[]), &from_glossiness, NormalFormat::OpenGl), ROUGHNESS).is_none());

        // Glossiness itself is the opposite of roughness
        let glossiness = options(vec![map("g", FileFormat::Png, vec![(Dest::L, virt(VirtualMap::Glossiness, 0))])]);
        assert!(bind(&set_with(&[]), &glossiness, NormalFormat::OpenGl).is_empty());
    }

    #[test]
    fn a_file_that_cannot_hold_alpha_or_colour_is_not_bound_for_them() {
        let set = set_with(&[Channel::Opacity]);
        for format in [FileFormat::Jpeg, FileFormat::Hdr] {
            let bindings = bind(&set, &options(vec![map("albedo", format, rgba())]), NormalFormat::OpenGl);
            assert!(slot(&bindings, BASE_COLOR).is_some(), "{format:?} keeps the colour");
            assert!(slot(&bindings, OPACITY).is_none(), "{format:?} drops alpha");
        }
        assert!(slot(&bind(&set, &options(vec![map("albedo", FileFormat::Png, rgba())]), NormalFormat::OpenGl), OPACITY).is_some());

        // PGM is one gray channel: colour becomes luminance, and only a map written as L is bound
        let pgm = options(vec![
            map("albedo", FileFormat::Pgm, rgba()),
            map("orm", FileFormat::Pgm, vec![(Dest::G, ch(Channel::Roughness, 0)), (Dest::B, ch(Channel::Metallic, 0))]),
            map("metal", FileFormat::Pgm, vec![(Dest::L, ch(Channel::Metallic, 0))]),
        ]);
        let bindings = bind(&set, &pgm, NormalFormat::OpenGl);
        assert!(slot(&bindings, BASE_COLOR).is_none());
        assert!(slot(&bindings, ROUGHNESS).is_none());
        assert_eq!(slot(&bindings, METALLIC).map(|b| (b.output, b.file.as_str())), Some(("r", "metal.pgm")));
    }

    #[test]
    fn a_normal_map_unpacks_with_the_convention_of_its_file() {
        let set = set_with(&[]);
        let normal = |source: fn(u8) -> SourceRef| {
            options(vec![map("n", FileFormat::Png, (0..3).map(|k| ([Dest::R, Dest::G, Dest::B][k as usize], source(k))).collect())])
        };
        let convention = |opts: &ExportOptions| slot(&bind(&set, opts, NormalFormat::DirectX), NORMAL).unwrap().normal;
        assert_eq!(convention(&normal(|k| ch(Channel::Normal, k))), Some(NormalFormat::DirectX), "follows the project");
        assert_eq!(convention(&normal(|k| virt(VirtualMap::NormalOpenGl, k))), Some(NormalFormat::OpenGl));

        let mut forced = normal(|k| virt(VirtualMap::NormalOpenGl, k));
        forced.normal_format = Some(NormalFormat::DirectX);
        assert_eq!(convention(&forced), Some(NormalFormat::DirectX), "the export's normal format wins");
    }

    #[test]
    fn a_normal_map_takes_its_convention_from_the_green_channel() {
        let set = set_with(&[]);
        // Red and blue are the same in both conventions; green decides
        let mixed = |green: VirtualMap| {
            options(vec![map(
                "n",
                FileFormat::Png,
                vec![(Dest::R, virt(VirtualMap::NormalOpenGl, 0)), (Dest::G, virt(green, 1)), (Dest::B, virt(VirtualMap::NormalOpenGl, 2))],
            )])
        };
        let convention = |opts: &ExportOptions| slot(&bind(&set, opts, NormalFormat::OpenGl), NORMAL).unwrap().normal;
        assert_eq!(convention(&mixed(VirtualMap::NormalDirectX)), Some(NormalFormat::DirectX));
        assert_eq!(convention(&mixed(VirtualMap::NormalOpenGl)), Some(NormalFormat::OpenGl));
    }
}
