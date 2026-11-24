//! Mesh export of the working mesh: Wavefront OBJ (positions, UVs, normals, one group per
//! submesh; with an MTL material library and its maps, or alone), binary glTF (one primitive per submesh, a metallic-roughness material per
//! material with its textures embedded) and USD (usda / usdc / usdz, see [`usd`]).

pub(crate) mod usd;

use std::collections::BTreeSet;
use std::fmt::Write as _;

use craft_image_codecs::{DecodeOptions, Format, Limits, SampleType, decode_as_with, png_probe};
use texturecraft_core::{Channel, NormalFormat};
use texturecraft_geom::{Mesh, SubMesh};

use crate::export::sanitize_file_stem;
use crate::presets::{GLTF_BASE_COLOR, GLTF_CLEARCOAT, GLTF_EMISSIVE, GLTF_NORMAL, GLTF_ORM, GLTF_SHEEN, gltf_pbr_maps};
use crate::{ExportOptions, ExportPreset, ExportedFile, FileFormat, FormatError, Result};

pub use usd::{NamedFiles, UsdExport, UsdFormat, UsdTextureBinding, usd_bytes, usd_export_options, usd_layer_with_textures, usd_texture_bindings};

/// The mesh as OBJ text (UV v flipped back to OBJ's bottom-up convention).
/// Invalid mesh data or shared writer limits return an error, never an empty successful export.
pub fn obj_bytes(mesh: &Mesh, name: &str) -> Result<Vec<u8>> {
    let scene = to_craft_scene(mesh, &token(name), mesh.materials.iter().map(|m| craft_mesh::Material::plain(token(m), [1.0; 4])).collect(), Vec::new());
    craft_mesh::obj::write(&scene, None, &craft_mesh::Limits::default()).map(String::into_bytes).map_err(|e| FormatError::Invalid(e.to_string()))
}

/// The mesh as OBJ with a material library: the OBJ (naming `<stem>.mtl`, `stem` being the
/// OBJ file's own, so exports beside each other keep their files apart) and the files to write
/// next to it, the MTL first, then each material's maps (`textures[m]`, rendered with
/// [`usd_export_options`], named like USD's). Roughness, metallic, normal and coat maps use the
/// MTL PBR extension's keys. Writer failures return an error without companion files.
pub fn obj_with_mtl(mesh: &Mesh, name: &str, stem: &str, textures: &[Vec<ExportedFile>]) -> Result<(Vec<u8>, NamedFiles)> {
    let clean = sanitize_file_stem(stem);
    // OBJ names that sanitize alike (`ball 1`, `ball_1`) must not overwrite each other's files:
    // a changed name gets a hash of the original (FNV-1a).
    let stem = if clean == stem {
        clean
    } else {
        let hash = stem.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
        format!("{clean}_{hash:08x}")
    };
    let scene = to_craft_scene(mesh, &token(name), mesh.materials.iter().map(|m| craft_mesh::Material::plain(token(m), [1.0; 4])).collect(), Vec::new());
    let (maps, mut files) = usd::material_maps(mesh, &stem, textures);
    let mut mtl = format!("# {}, exported by TextureCraft\n", token(name));
    for (material, maps) in craft_mesh::obj::material_names(&scene).iter().zip(&maps) {
        let _ = writeln!(mtl, "\nnewmtl {material}\nKd 1 1 1");
        for (slot, file) in maps {
            let keys: &[&str] = match usd::SLOTS.get(*slot).map_or("", |s| s.0) {
                "BaseColor" => &["map_Kd"],
                "Roughness" => &["map_Pr"],
                "Metallic" => &["map_Pm"],
                // `norm` is the PBR extension's key; many tools read a normal map from `map_Bump`.
                "Normal" => &["norm", "map_Bump"],
                // The emissive map scales `Ke`.
                "Emissive" => &["Ke 1 1 1\nmap_Ke"],
                "Opacity" => &["map_d"],
                "Clearcoat" => &["map_Pc"],
                "ClearcoatRoughness" => &["map_Pcr"],
                _ => &[],
            };
            for key in keys {
                let _ = writeln!(mtl, "{key} {file}");
            }
        }
    }
    files.insert(0, (format!("{stem}.mtl"), mtl.into_bytes()));
    let obj = craft_mesh::obj::write(&scene, Some(&format!("{stem}.mtl")), &craft_mesh::Limits::default()).map_err(|e| FormatError::Invalid(e.to_string()))?;
    Ok((obj.into_bytes(), files))
}

fn to_craft_scene(mesh: &Mesh, name: &str, materials: Vec<craft_mesh::Material>, images: Vec<craft_mesh::Image>) -> craft_mesh::Scene {
    let n = mesh.positions.len();
    let has_uv = mesh.uvs.len() == n;
    let has_normal = mesh.normals.len() == n;
    let mut parts_out = Vec::new();
    for sm in parts(mesh, name) {
        let tris = mesh.indices.get(sm.start as usize..(sm.start as usize).saturating_add(sm.count as usize)).unwrap_or_default();
        let tris = tris.get(..tris.len() / 3 * 3).unwrap_or_default();
        if tris.is_empty() {
            continue;
        }
        let corners = tris
            .iter()
            .copied()
            .map(|i| craft_mesh::Corner { position: i, uv: if has_uv { i } else { craft_mesh::NONE }, normal: if has_normal { i } else { craft_mesh::NONE } })
            .collect();
        parts_out.push(craft_mesh::Part {
            name: token(&sm.name),
            material: (sm.material < materials.len()).then_some(sm.material),
            corners,
            ..craft_mesh::Part::default()
        });
    }
    let cm = craft_mesh::Mesh {
        name: name.into(),
        positions: mesh.positions.iter().map(|p| p.to_array().map(f64::from)).collect(),
        normals: if has_normal { mesh.normals.iter().map(|v| v.to_array().map(f64::from)).collect() } else { Vec::new() },
        uv_sets: if has_uv { vec![mesh.uvs.iter().map(|t| t.to_array()).collect()] } else { Vec::new() },
        parts: parts_out,
        ..craft_mesh::Mesh::default()
    };
    let mut scene = craft_mesh::Scene::from_meshes(vec![cm], materials);
    scene.images = images;
    scene
}

/// `id` of each material name, made unique with `_2`, `_3`… where two would be the same, also
/// ignoring case (the ids name files, and file systems may ignore case).
pub(crate) fn unique_ids(materials: &[String], id: impl Fn(&str) -> String) -> Vec<String> {
    let mut seen = BTreeSet::new();
    materials
        .iter()
        .map(|m| {
            let base = id(m);
            let mut unique = base.clone();
            for n in 2.. {
                if seen.insert(unique.to_lowercase()) {
                    break;
                }
                unique = format!("{base}_{n}");
            }
            unique
        })
        .collect()
}

/// The submeshes, or one covering the whole mesh when it has none.
fn parts(mesh: &Mesh, name: &str) -> Vec<SubMesh> {
    if mesh.submeshes.is_empty() {
        vec![SubMesh { name: name.into(), material: 0, start: 0, count: mesh.indices.len() as u32 }]
    } else {
        mesh.submeshes.clone()
    }
}

/// Textures of one glTF material (PNG bytes), from the files of [`glb_export_options`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlbTextures {
    pub base_color: Option<Vec<u8>>,
    /// Occlusion (R), roughness (G), metallic (B).
    pub orm: Option<Vec<u8>>,
    /// OpenGL convention (+Y up the image), as glTF expects.
    pub normal: Option<Vec<u8>>,
    pub emissive: Option<Vec<u8>>,
    /// KHR_materials_clearcoat: weight (R), roughness (G).
    pub clearcoat: Option<Vec<u8>>,
    /// KHR_materials_sheen: color × weight (RGB), roughness (A, when the set has it).
    pub sheen: Option<Vec<u8>>,
}

impl GlbTextures {
    /// Sort rendered files into slots; an all-black emissive map is dropped.
    pub fn from_files(files: Vec<ExportedFile>) -> GlbTextures {
        let mut t = GlbTextures::default();
        for f in files {
            let slot = match f.name.strip_suffix(".png").unwrap_or_default() {
                GLTF_BASE_COLOR => &mut t.base_color,
                GLTF_ORM => &mut t.orm,
                GLTF_NORMAL => &mut t.normal,
                GLTF_EMISSIVE if !is_black(&f.bytes) => &mut t.emissive,
                GLTF_CLEARCOAT => &mut t.clearcoat,
                GLTF_SHEEN => &mut t.sheen,
                _ => continue,
            };
            *slot = Some(f.bytes);
        }
        t
    }
}

/// Texture export options for [`GlbTextures`]: the glTF maps as 8-bit PNG named by slot.
pub fn glb_export_options() -> ExportOptions {
    png8_options(ExportPreset { name: "glTF binary".into(), maps: gltf_pbr_maps(str::to_string) }, NormalFormat::OpenGl)
}

/// `preset` as 8-bit PNGs with normals in `normal` convention.
fn png8_options(preset: ExportPreset, normal: NormalFormat) -> ExportOptions {
    let mut o = ExportOptions::new(preset);
    o.format_override = Some(FileFormat::Png);
    o.bit_depth_override = Some(8);
    o.normal_format = Some(normal);
    o
}

fn is_black(png: &[u8]) -> bool {
    // Keep the previous PNG decoder's 512 MiB allocation ceiling.
    let options = DecodeOptions { limits: Limits { max_alloc: 512 << 20, ..Limits::none() }, keep_orientation: true };
    let Ok(image) = decode_as_with(Format::Png, png, &options) else { return true };
    let sample = image.sample_type();
    let layout = image.layout();
    let pixel_bytes = layout.channels() * sample.bytes();
    let color_bytes = layout.color_channels() * sample.bytes();
    let mut colors = image.data().chunks_exact(pixel_bytes).map(|pixel| &pixel[..color_bytes]);
    match sample {
        SampleType::U8 => colors.all(|color| color.iter().all(|&value| value == 0)),
        // The prior RGB8 conversion rounds 16-bit values through 128 to zero.
        SampleType::U16 => colors.all(|color| color.as_chunks::<2>().0.iter().all(|&value| u16::from_ne_bytes(value) <= 128)),
        _ => true,
    }
}

/// PNG color type 4 (gray + alpha) or 6 (RGBA), from the IHDR chunk.
fn png_has_alpha(png: &[u8]) -> bool {
    png_probe::pixel_model(png).is_ok_and(|model| matches!(model.color_type, 4 | 6))
}

/// The mesh as binary glTF (GLB) named `name`, with `textures[i]` on material `i` (materials
/// without an entry get a plain one). Vertex data is written as is: the mesh already follows
/// glTF's conventions.
pub fn glb_bytes(mesh: &Mesh, name: &str, textures: &[GlbTextures]) -> Result<Vec<u8>> {
    if mesh.positions.is_empty() || mesh.indices.len() < 3 {
        return Err(FormatError::Invalid("the mesh is empty".into()));
    }
    let none = GlbTextures::default();
    let mut images = Vec::new();
    let mut push_img = |png: &[u8]| {
        let image = images.len();
        images.push(craft_mesh::Image { mime_type: Some("image/png".into()), data: Some(png.to_vec()), ..craft_mesh::Image::default() });
        craft_mesh::TextureRef { image, ..craft_mesh::TextureRef::default() }
    };
    let materials: Vec<craft_mesh::Material> = mesh
        .materials
        .iter()
        .enumerate()
        .map(|(i, mat_name)| {
            let t = textures.get(i).unwrap_or(&none);
            let mut m = craft_mesh::Material { name: mat_name.clone(), metallic: 0.0, roughness: 1.0, ..craft_mesh::Material::default() };
            if let Some(png) = &t.base_color {
                m.base_color_texture = Some(push_img(png));
                if png_has_alpha(png) {
                    m.alpha_mode = craft_mesh::AlphaMode::Blend;
                }
            }
            if let Some(png) = &t.orm {
                let tex = push_img(png);
                m.metallic_roughness_texture = Some(tex);
                m.occlusion_texture = Some(tex);
                m.metallic = 1.0;
            }
            if let Some(png) = &t.normal {
                m.normal_texture = Some(push_img(png));
            }
            if let Some(png) = &t.emissive {
                m.emissive_texture = Some(push_img(png));
                m.emissive = [1.0; 3];
            }
            if let Some(png) = &t.clearcoat {
                let tex = push_img(png);
                m.clearcoat = 1.0;
                m.clearcoat_roughness = 1.0;
                m.clearcoat_texture = Some(tex);
                m.clearcoat_roughness_texture = Some(tex);
            }
            if let Some(png) = &t.sheen {
                let tex = push_img(png);
                m.sheen_color = [1.0; 3];
                m.sheen_color_texture = Some(tex);
                if png_has_alpha(png) {
                    m.sheen_roughness = 1.0;
                    m.sheen_roughness_texture = Some(tex);
                } else {
                    m.sheen_roughness = Channel::SheenRoughness.default_value()[0];
                }
            }
            m
        })
        .collect();
    let scene = to_craft_scene(mesh, name, materials, images);
    let options = craft_mesh::gltf::GlbWriteOptions {
        mag_filter: Some(craft_mesh::gltf::MagFilter::Linear),
        min_filter: Some(craft_mesh::gltf::MinFilter::LinearMipmapLinear),
    };
    craft_mesh::gltf::write_glb_with_options(&scene, &craft_mesh::Limits::default(), &options).map_err(|e| FormatError::Invalid(e.to_string()))
}

/// OBJ names end at whitespace.
fn token(name: &str) -> String {
    let t: String = name.trim().chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect();
    if t.is_empty() { "unnamed".into() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_writers_report_invalid_corner_indices() {
        let mut mesh = texturecraft_geom::primitives::cube(1.0);
        mesh.indices[0] = u32::MAX;
        for error in [obj_bytes(&mesh, "invalid").unwrap_err(), obj_with_mtl(&mesh, "invalid", "m", &[]).unwrap_err()] {
            assert!(matches!(error, FormatError::Invalid(_)));
            assert!(error.to_string().contains("corner index is out of range"), "{error}");
        }
    }

    #[test]
    fn glb_writer_rejects_empty_mesh_and_invalid_corner_indices() {
        let mut empty = texturecraft_geom::primitives::cube(1.0);
        empty.positions.clear();
        assert!(matches!(glb_bytes(&empty, "empty", &[]).unwrap_err(), FormatError::Invalid(_)));

        let mut invalid = texturecraft_geom::primitives::cube(1.0);
        invalid.indices[0] = u32::MAX;
        let error = glb_bytes(&invalid, "invalid", &[]).unwrap_err();
        assert!(matches!(error, FormatError::Invalid(_)));
        assert!(error.to_string().contains("corner index is out of range"), "{error}");
    }

    #[test]
    fn obj_with_mtl_links_all_material_texture_slots() {
        let mesh = texturecraft_geom::primitives::cube(1.0);
        let mut png = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(&mut png, &[200u8, 200, 200], 1, 1, image::ColorType::Rgb8, image::ImageFormat::Png).unwrap();
        let png_bytes = png.into_inner();
        let file = |name: &str| ExportedFile { name: name.into(), bytes: png_bytes.clone(), width: 1, height: 1, warnings: Vec::new() };
        let textures = vec![vec![
            file("BaseColor.png"),
            file("Roughness.png"),
            file("Metallic.png"),
            file("Normal.png"),
            file("Emissive.png"),
            file("Opacity.png"),
            file("Clearcoat.png"),
            file("ClearcoatRoughness.png"),
        ]];
        let (obj, files) = obj_with_mtl(&mesh, "project", "cube", &textures).unwrap();
        let obj = String::from_utf8(obj).unwrap();
        assert!(obj.contains("mtllib cube.mtl\n"));
        assert!(obj.contains("usemtl Cube\n"));
        let mtl = String::from_utf8(files[0].1.clone()).unwrap();
        assert_eq!(files[0].0, "cube.mtl");
        for (key, slot) in [
            ("map_Kd", "BaseColor"),
            ("map_Pr", "Roughness"),
            ("map_Pm", "Metallic"),
            ("norm", "Normal"),
            ("map_Bump", "Normal"),
            ("map_Ke", "Emissive"),
            ("map_d", "Opacity"),
            ("map_Pc", "Clearcoat"),
            ("map_Pcr", "ClearcoatRoughness"),
        ] {
            let expected = format!("{key} cube_Cube_{slot}.png");
            assert!(mtl.lines().any(|line| line == expected), "missing `{expected}` in:\n{mtl}");
        }
        assert!(mtl.contains("Ke 1 1 1\nmap_Ke cube_Cube_Emissive.png"));
        assert_eq!(files.len(), 9);
    }

    #[test]
    fn name_with_newline_stays_in_the_header_comment() {
        let mesh = texturecraft_geom::primitives::cube(1.0);
        let name = "a\nv 9 9 9\r\nmtllib x";
        let obj = String::from_utf8(obj_bytes(&mesh, name).unwrap()).unwrap();
        assert!(obj.lines().all(|l| !l.starts_with("v 9 9 9") && !l.starts_with("mtllib x")));
        let (obj, files) = obj_with_mtl(&mesh, name, "m", &[]).unwrap();
        let obj = String::from_utf8(obj).unwrap();
        assert!(obj.lines().all(|l| !l.starts_with("v 9 9 9") && !l.starts_with("mtllib x")));
        let mtl = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(mtl.lines().all(|l| !l.starts_with("v 9 9 9") && !l.starts_with("mtllib x")));
    }

    #[test]
    fn materials_whose_names_collide_even_by_case_stay_apart() {
        let mut mesh = texturecraft_geom::primitives::cube(1.0);
        mesh.materials = vec!["Mat A".into(), "mat_a".into()];
        mesh.submeshes = vec![
            SubMesh { name: "a".into(), material: 0, start: 0, count: 18 },
            SubMesh { name: "b".into(), material: 1, start: 18, count: mesh.indices.len() as u32 - 18 },
        ];
        let png = |name: &str| ExportedFile { name: name.into(), bytes: vec![1, 2, 3], width: 1, height: 1, warnings: Vec::new() };
        let (obj, files) = obj_with_mtl(&mesh, "project", "m", &[vec![png("BaseColor.png")], vec![png("BaseColor.png")]]).unwrap();
        let obj = String::from_utf8(obj).unwrap();
        assert!(obj.contains("mtllib m.mtl\n") && obj.contains("usemtl Mat_A\n") && obj.contains("usemtl mat_a_2\n"));
        let names: Vec<_> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["m.mtl", "m_Mat_A_BaseColor.png", "m_mat_a_2_BaseColor.png"]);
        let mtl = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(
            mtl.contains("newmtl Mat_A\nKd 1 1 1\nmap_Kd m_Mat_A_BaseColor.png") && mtl.contains("newmtl mat_a_2\nKd 1 1 1\nmap_Kd m_mat_a_2_BaseColor.png"),
            "{mtl}"
        );
    }

    #[test]
    fn obj_and_mtl_share_material_ids_after_control_character_normalization() {
        let mut mesh = texturecraft_geom::primitives::cube(1.0);
        mesh.materials = vec!["Mat\u{7}A".into(), "Mat_A".into()];
        mesh.submeshes = vec![
            SubMesh { name: "a".into(), material: 0, start: 0, count: 18 },
            SubMesh { name: "b".into(), material: 1, start: 18, count: mesh.indices.len() as u32 - 18 },
        ];
        let png = || ExportedFile { name: "BaseColor.png".into(), bytes: vec![1], width: 1, height: 1, warnings: Vec::new() };
        let (obj, files) = obj_with_mtl(&mesh, "project", "m", &[vec![png()], vec![png()]]).unwrap();
        let obj = String::from_utf8(obj).unwrap();
        let mtl = String::from_utf8(files[0].1.clone()).unwrap();
        let used: Vec<_> = obj.lines().filter_map(|line| line.strip_prefix("usemtl ")).collect();
        let declared: Vec<_> = mtl.lines().filter_map(|line| line.strip_prefix("newmtl ")).collect();
        assert_eq!(used, ["Mat_A", "Mat_A_2"]);
        assert_eq!(declared, used, "each OBJ part must resolve to its own MTL material");
        let maps: Vec<_> = mtl.lines().filter_map(|line| line.strip_prefix("map_Kd ")).collect();
        assert_eq!(maps, [files[1].0.as_str(), files[2].0.as_str()], "material index and texture mapping stay aligned");
    }

    #[test]
    fn obj_names_that_sanitize_alike_keep_their_companions_apart() {
        let mesh = texturecraft_geom::primitives::cube(1.0);
        let png = || vec![ExportedFile { name: "BaseColor.png".into(), bytes: vec![1], width: 1, height: 1, warnings: Vec::new() }];
        let names = |stem: &str| obj_with_mtl(&mesh, "p", stem, &[png()]).unwrap().1.into_iter().map(|f| f.0).collect::<Vec<_>>();
        let (spaced, plain) = (names("ball 1"), names("ball_1"));
        assert_eq!(plain, ["ball_1.mtl", "ball_1_Cube_BaseColor.png"], "a clean name is kept");
        assert!(spaced.iter().all(|n| n.starts_with("ball_1_") && !plain.contains(n)), "{spaced:?}");
        assert_eq!(spaced, names("ball 1"), "the same name always maps alike");
    }

    #[test]
    fn obj_round_trips_through_the_importer() {
        let mesh = texturecraft_geom::primitives::rounded_box(texturecraft_geom::glam::Vec3::splat(1.0), 0.1);
        let bytes = obj_bytes(&mesh, "My box").unwrap();
        let back = crate::import_mesh_bytes("box.obj", &bytes).unwrap().mesh;
        assert_eq!(back.triangle_count(), mesh.triangle_count());
        let (a, b) = (mesh.bounds(), back.bounds());
        assert!((a.0 - b.0).length() < 1e-4 && (a.1 - b.1).length() < 1e-4);
        let uv_sum = |m: &Mesh| m.indices.iter().map(|&i| m.uvs[i as usize].y as f64).sum::<f64>();
        assert!((uv_sum(&mesh) - uv_sum(&back)).abs() < 1e-3 * mesh.indices.len() as f64);
        assert!(String::from_utf8(bytes).unwrap().contains("o My_box"));
    }

    #[test]
    fn glb_texture_export_keeps_filtering() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(&mut png, &[127, 63, 31], 1, 1, image::ColorType::Rgb8, image::ImageFormat::Png).unwrap();
        let textures = GlbTextures { base_color: Some(png.into_inner()), ..GlbTextures::default() };
        let mesh = texturecraft_geom::primitives::cube(1.0);
        let glb = glb_bytes(&mesh, "Textured cube", &[textures]).unwrap();

        let document = gltf::Gltf::from_slice(&glb).unwrap();
        let material = document.materials().next().unwrap();
        let sampler = material.pbr_metallic_roughness().base_color_texture().unwrap().texture().sampler();
        let expected = (Some(gltf::texture::MagFilter::Linear), Some(gltf::texture::MinFilter::LinearMipmapLinear));
        assert_eq!((sampler.mag_filter(), sampler.min_filter()), expected);
    }

    #[test]
    fn mesh_png_maps_keep_only_nonblack_color() {
        let png = |color, samples: &[u8]| {
            let mut out = std::io::Cursor::new(Vec::new());
            image::write_buffer_with_format(&mut out, samples, 1, 1, color, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let mesh = texturecraft_geom::primitives::cube(1.0);
        for (bytes, keep) in [
            (png(image::ColorType::Rgba8, &[0, 0, 0, 255]), false),
            (png(image::ColorType::Rgba8, &[0, 1, 0, 0]), true),
            (png(image::ColorType::L16, &128u16.to_ne_bytes()), false),
            (png(image::ColorType::L16, &129u16.to_ne_bytes()), true),
            (b"PNG without image data".to_vec(), false),
        ] {
            let file = |name: &str, bytes: &[u8]| ExportedFile { name: name.into(), bytes: bytes.to_vec(), width: 1, height: 1, warnings: Vec::new() };
            let emissive = file("Emissive.png", &bytes);
            let textures = GlbTextures::from_files(vec![emissive.clone()]);
            assert_eq!(textures.emissive.as_deref(), keep.then_some(bytes.as_slice()));
            let glb = glb_bytes(&mesh, "PNG maps", &[textures]).unwrap();
            let glb = gltf::Gltf::from_slice(&glb).unwrap();
            assert_eq!(glb.materials().next().unwrap().emissive_texture().is_some(), keep);

            let roughness = png(image::ColorType::L8, &[64]);
            let usd_maps = vec![emissive, file("Clearcoat.png", &bytes), file("ClearcoatRoughness.png", &roughness)];
            let usd = usd_bytes(&mesh, "PNG maps", &[usd_maps], UsdFormat::Usda).unwrap();
            if keep {
                assert_eq!(usd.textures.len(), 3);
                for (suffix, expected) in [("_Emissive.png", &bytes), ("_Clearcoat.png", &bytes), ("_ClearcoatRoughness.png", &roughness)] {
                    let (_, written) = usd.textures.iter().find(|(name, _)| name.ends_with(suffix)).unwrap();
                    assert_eq!(written, expected);
                }
            } else {
                assert!(usd.textures.is_empty());
            }
        }
    }

    #[test]
    fn mesh_png_alpha_requires_a_png_with_stored_alpha() {
        let encode = |color: png::ColorType, samples: &[u8], color_key: bool| {
            let mut output = Vec::new();
            let mut encoder = png::Encoder::new(&mut output, 1, 1);
            encoder.set_color(color);
            if color_key {
                encoder.set_trns(vec![0; 6]);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(samples).unwrap();
            writer.finish().unwrap();
            output
        };
        let mut impostor = vec![0; 26];
        impostor[25] = 6;
        let mesh = texturecraft_geom::primitives::cube(1.0);
        for (bytes, expected) in [
            (encode(png::ColorType::Rgba, &[0, 1, 2, 120], false), gltf::material::AlphaMode::Blend),
            (encode(png::ColorType::Rgb, &[0, 0, 0], true), gltf::material::AlphaMode::Opaque),
            (impostor, gltf::material::AlphaMode::Opaque),
        ] {
            let textures = GlbTextures { base_color: Some(bytes), ..GlbTextures::default() };
            let glb = glb_bytes(&mesh, "PNG alpha", &[textures]).unwrap();
            let glb = gltf::Gltf::from_slice(&glb).unwrap();
            assert_eq!(glb.materials().next().unwrap().alpha_mode(), expected);
        }
    }

    #[test]
    fn glb_writes_clearcoat_and_sheen_extensions() {
        use serde_json::{Value, json};
        const KHR_CLEARCOAT: &str = "KHR_materials_clearcoat";
        const KHR_SHEEN: &str = "KHR_materials_sheen";
        let png = |color: image::ColorType| {
            let mut out = std::io::Cursor::new(Vec::new());
            image::write_buffer_with_format(&mut out, &vec![200u8; 4 * color.channel_count() as usize], 2, 2, color, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let file = |name: &str, bytes: Vec<u8>| ExportedFile { name: name.into(), bytes, width: 2, height: 2, warnings: vec![] };
        let mesh = texturecraft_geom::primitives::sphere(1.0, 8, 4);
        let json_of = |t: GlbTextures| {
            let glb = glb_bytes(&mesh, "Ball", &[t]).unwrap();
            let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
            serde_json::from_slice::<Value>(&glb[20..20 + len]).unwrap()
        };
        let t = GlbTextures::from_files(vec![file("Clearcoat.png", png(image::ColorType::Rgb8)), file("Sheen.png", png(image::ColorType::Rgb8))]);
        let doc = json_of(t);
        assert_eq!(doc["extensionsUsed"], json!([KHR_CLEARCOAT, KHR_SHEEN]));
        let ext = &doc["materials"][0]["extensions"];
        let coat = &ext[KHR_CLEARCOAT];
        assert_eq!(coat["clearcoatTexture"], coat["clearcoatRoughnessTexture"], "one map: weight in R, roughness in G");
        let sheen = &ext[KHR_SHEEN];
        assert!(sheen["sheenColorTexture"].is_object() && sheen["sheenRoughnessTexture"].is_null(), "no alpha: no roughness texture");
        assert_eq!(sheen["sheenRoughnessFactor"], json!(0.5));
        let doc = json_of(GlbTextures::from_files(vec![file("Sheen.png", png(image::ColorType::Rgba8))]));
        assert_eq!(doc["extensionsUsed"], json!([KHR_SHEEN]));
        assert_eq!(doc["materials"][0]["extensions"][KHR_SHEEN]["sheenRoughnessTexture"], doc["materials"][0]["extensions"][KHR_SHEEN]["sheenColorTexture"]);
        let untextured = json_of(GlbTextures::default());
        for key in ["extensionsUsed", "textures", "samplers"] {
            assert!(untextured.get(key).is_none(), "unexpected {key} on an untextured export");
        }
    }
}
