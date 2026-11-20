use std::io::Cursor;

use image::{DynamicImage, GenericImageView, ImageFormat};
use texturecraft_core::{ChannelFormat, Project, TextureSet};
use texturecraft_texel::{to_u16, to_u16_signed};

use super::*;
use crate::presets::{builtin_preset, builtin_presets, preset_from_json, preset_to_json};

const RES: u32 = 64;

struct Fx {
    project: Project,
    comp: Composite,
    baked: BakedMaps,
    coverage: Vec<bool>,
}

impl Fx {
    fn new(channels: &[Channel]) -> Fx {
        let mut project = Project::new("My Project", "builtin:sphere");
        let mut ts = TextureSet::new("Body Set", 0, RES);
        ts.channels = channels.iter().map(|c| (*c, ChannelFormat::default_for(*c))).collect();
        project.texture_sets.push(ts);
        let mut fx = Fx { project, comp: Composite::new(channels, RES), baked: BakedMaps::default(), coverage: vec![true; (RES * RES) as usize] };
        for y in 0..RES {
            for x in 0..RES {
                fx.set(Channel::BaseColor, x, y, [x as f32 / 63.0, y as f32 / 63.0, 0.2]);
                fx.set(Channel::Roughness, x, y, [0.25; 3]);
                fx.set(Channel::Metallic, x, y, [0.75; 3]);
                fx.set(Channel::Normal, x, y, [0.5, 0.5, 1.0]);
                fx.set(Channel::Height, x, y, [0.0; 3]);
            }
        }
        fx
    }

    fn set(&mut self, c: Channel, x: u32, y: u32, v: [f32; 3]) {
        let Some(img) = self.comp.channels.get_mut(&c) else { return };
        let vals: Vec<u16> = v.iter().take(c.components()).map(|v| if c.kind() == ChannelKind::Signed { to_u16_signed(*v) } else { to_u16(*v) }).collect();
        img.set(x, y, &vals);
    }

    fn set_all(&mut self, c: Channel, f: impl Fn(u32, u32) -> [f32; 3]) {
        for y in 0..RES {
            for x in 0..RES {
                self.set(c, x, y, f(x, y));
            }
        }
    }

    fn input(&self) -> ExportInput<'_> {
        ExportInput { project: &self.project, ts: 0, composite: &self.comp, baked: &self.baked, coverage: &self.coverage, mesh_name: "sphere" }
    }

    fn render(&self, opts: &ExportOptions) -> Vec<ExportedFile> {
        render_maps(&self.input(), opts).unwrap()
    }

    fn render_preset(&self, name: &str) -> Vec<ExportedFile> {
        self.render(&ExportOptions::new(builtin_preset(name).unwrap()))
    }
}

fn find<'a>(files: &'a [ExportedFile], suffix: &str) -> &'a ExportedFile {
    files.iter().find(|f| f.name.contains(suffix)).unwrap_or_else(|| panic!("no {suffix} in {:?}", files.iter().map(|f| &f.name).collect::<Vec<_>>()))
}

fn decode(f: &ExportedFile) -> DynamicImage {
    let ext = f.name.rsplit('.').next().unwrap();
    image::load_from_memory_with_format(&f.bytes, ImageFormat::from_extension(ext).unwrap()).unwrap()
}

fn default_channels() -> Vec<Channel> {
    Channel::DEFAULT_SET.to_vec()
}

#[test]
fn gltf_png_matches_the_composite() {
    let fx = Fx::new(&default_channels());
    let files = fx.render_preset("glTF PBR Metal Roughness");
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["sphere_Body_Set_BaseColor.png", "sphere_Body_Set_OcclusionRoughnessMetallic.png", "sphere_Body_Set_Normal.png"]);
    let base = decode(&files[0]);
    assert_eq!(base.color(), image::ColorType::Rgb8, "no opacity channel → no alpha");
    for (x, y) in [(0, 0), (10, 40), (63, 63), (32, 7)] {
        let v = fx.comp.value(Channel::BaseColor, x, y);
        let px = base.get_pixel(x, y);
        for k in 0..3 {
            let want = linear_to_srgb(v[k]) * 255.0;
            assert!((px[k] as f32 - want).abs() <= 1.0, "base ({x},{y})[{k}] {} vs {want}", px[k]);
        }
    }
    let orm = decode(&files[1]).get_pixel(5, 5);
    assert_eq!(orm[0], 255, "no AO anywhere → white");
    assert!((orm[1] as i32 - 64).abs() <= 1 && (orm[2] as i32 - 191).abs() <= 1, "{orm:?}");
    let n = decode(&files[2]).get_pixel(5, 5);
    assert!((n[0] as i32 - 128).abs() <= 1 && (n[1] as i32 - 128).abs() <= 1 && n[2] == 255, "{n:?}");
    // Emissive appears only when the set has the channel.
    let mut channels = default_channels();
    channels.push(Channel::Emissive);
    let fx = Fx::new(&channels);
    assert!(fx.render_preset("glTF PBR Metal Roughness").iter().any(|f| f.name.ends_with("_Emissive.png")));
}

#[test]
fn exr_values_are_linear() {
    let fx = Fx::new(&default_channels());
    for (depth, tol) in [(32u8, 1e-4f32), (16, 2e-3)] {
        let mut opts = ExportOptions::new(builtin_preset("glTF PBR Metal Roughness").unwrap());
        opts.format_override = Some(FileFormat::Exr);
        opts.bit_depth_override = Some(depth);
        let files = fx.render(&opts);
        let base = find(&files, "_BaseColor.exr");
        let img = decode(base).to_rgb32f();
        for (x, y) in [(0, 0), (10, 40), (63, 63)] {
            let v = fx.comp.value(Channel::BaseColor, x, y);
            let px = img.get_pixel(x, y);
            for k in 0..3 {
                assert!((px[k] - v[k]).abs() <= tol.max(v[k] * tol), "depth {depth} ({x},{y})[{k}]: {} vs {}", px[k], v[k]);
            }
        }
    }
    // A single-channel map is written as a luminance (Y) EXR.
    let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
    opts.format_override = Some(FileFormat::Exr);
    let files = fx.render(&opts);
    let rough = find(&files, "_roughness.exr");
    use exr::prelude::{ReadChannels, ReadLayers};
    let img = exr::prelude::read()
        .no_deep_data()
        .largest_resolution_level()
        .all_channels()
        .first_valid_layer()
        .all_attributes()
        .from_buffered(std::io::Cursor::new(&rough.bytes))
        .unwrap();
    let ch = &img.layer_data.channel_data.list;
    assert_eq!(ch.len(), 1);
    assert_eq!(ch[0].name.to_string(), "Y");
    assert!((ch[0].sample_data.value_by_flat_index(100).to_f32() - 0.25).abs() < 1e-3);
}

#[test]
fn exr_exports_luminance_and_alpha_at_both_depths() {
    use exr::prelude::{FlatSamples, ReadChannels, ReadLayers};

    let mut fx = Fx::new(&[Channel::Roughness]);
    fx.set_all(Channel::Roughness, |_, _| [1.0; 3]);
    let mut preset = builtin_preset("Separate maps").unwrap();
    preset.maps = vec![MapSpec {
        file: "luminance_alpha".into(),
        format: FileFormat::Exr,
        bit_depth: 16,
        channels: vec![(Dest::L, SourceRef::Channel { channel: Channel::Roughness, component: 0 }), (Dest::A, SourceRef::Constant(0.75))],
        inverted: Vec::new(),
    }];
    let mut opts = ExportOptions::new(preset);

    for depth in [16, 32] {
        opts.bit_depth_override = Some(depth);
        let files = fx.render(&opts);
        assert_eq!(files.len(), 1);
        let image = exr::prelude::read()
            .no_deep_data()
            .largest_resolution_level()
            .all_channels()
            .first_valid_layer()
            .all_attributes()
            .from_buffered(Cursor::new(&files[0].bytes))
            .unwrap();
        let layer = image.layer_data;
        assert_eq!((layer.size.0, layer.size.1), (RES as usize, RES as usize));
        assert_eq!(layer.channel_data.list.len(), 2);

        for (name, value) in [("Y", 1.0), ("A", 0.75)] {
            let channel = layer.channel_data.list.iter().find(|channel| channel.name.eq(name)).unwrap_or_else(|| panic!("missing EXR channel {name}"));
            let samples = &channel.sample_data;
            assert!(matches!((depth, samples), (16, FlatSamples::F16(_)) | (32, FlatSamples::F32(_))), "{depth}-bit {name} sample type");
            assert_eq!(samples.len(), (RES * RES) as usize);
            assert!(samples.values_as_f32().all(|sample| sample == value), "{depth}-bit {name} samples");
        }
    }
}

#[test]
fn directx_flips_green() {
    let mut fx = Fx::new(&default_channels());
    let n = Vec3::new(0.3, 0.4, 0.0);
    let n = Vec3::new(n.x, n.y, (1.0 - n.length_squared()).sqrt());
    fx.set_all(Channel::Normal, |_, _| (n * 0.5 + Vec3::splat(0.5)).to_array());
    let gl = decode(find(&fx.render_preset("glTF PBR Metal Roughness"), "_Normal.png")).to_rgb8();
    let dx = decode(find(&fx.render_preset("Unreal Engine (Packed)"), "_Normal.png")).to_rgb8();
    let (a, b) = (gl.get_pixel(9, 9), dx.get_pixel(9, 9));
    assert!((a[0] as i32 - ((n.x * 0.5 + 0.5) * 255.0).round() as i32).abs() <= 1);
    assert!((a[1] as i32 - ((n.y * 0.5 + 0.5) * 255.0).round() as i32).abs() <= 1, "{a:?}");
    assert_eq!(a[0], b[0]);
    assert!((a[1] as i32 + b[1] as i32 - 255).abs() <= 1, "{a:?} vs {b:?}");
    assert_eq!(a[2], b[2]);
    // The normal-format override applies to every normal output.
    let mut opts = ExportOptions::new(builtin_preset("glTF PBR Metal Roughness").unwrap());
    opts.normal_format = Some(NormalFormat::DirectX);
    assert_eq!(decode(find(&fx.render(&opts), "_Normal.png")).to_rgb8(), dx);
}

#[test]
fn height_becomes_normal_detail() {
    let mut fx = Fx::new(&[Channel::BaseColor, Channel::Height]);
    // Height rises to the right and down the image.
    fx.set_all(Channel::Height, |x, y| [(x as f32 + y as f32) / 160.0; 3]);
    fx.project.texture_sets[0].height_strength = 8.0;
    let files = fx.render_preset("Separate maps");
    let n = decode(find(&files, "_normal.png")).to_rgb8();
    let p = n.get_pixel(30, 30);
    assert!(p[0] < 120, "slope up to the right tilts the normal left: {p:?}");
    assert!(p[1] > 136, "slope up toward the image bottom tilts an OpenGL normal up: {p:?}");
    let h = decode(find(&files, "_height.png"));
    assert_eq!(h.color(), image::ColorType::L16);
    let v = h.to_luma16().get_pixel(30, 30)[0] as f32 / 65535.0;
    assert!((v - (60.0 / 160.0 * 0.5 + 0.5)).abs() < 1e-3, "height is stored as h·0.5+0.5: {v}");
    fx.project.texture_sets[0].height_strength = 0.0;
    let flat = decode(find(&fx.render_preset("Separate maps"), "_normal.png")).to_rgb8();
    assert_eq!(flat.get_pixel(30, 30).0, [128, 128, 255]);
}

#[test]
fn reoriented_normal_mapping_identities() {
    let d = Vec3::new(0.2, -0.3, 0.9).normalize();
    assert!((rnm(Vec3::Z, d) - d).length() < 1e-5);
    assert!((rnm(d, Vec3::Z) - d).length() < 1e-5);
    assert!(rnm(Vec3::new(0.0, 0.0, -1.0), d).is_finite());
}

#[test]
fn ao_mixes_baked_and_channel() {
    let mut channels = default_channels();
    channels.push(Channel::AmbientOcclusion);
    let mut fx = Fx::new(&channels);
    fx.set_all(Channel::AmbientOcclusion, |_, _| [0.5; 3]);
    fx.baked = BakedMaps { resolution: RES, maps: [(BakedMap::AmbientOcclusion, std::sync::Arc::new(vec![0.5; (RES * RES) as usize]))].into() };
    fx.project.texture_sets[0].baked.insert(BakedMap::AmbientOcclusion);
    let orm = decode(find(&fx.render_preset("glTF PBR Metal Roughness"), "Metallic.png")).to_rgb8();
    assert!((orm.get_pixel(3, 3)[0] as i32 - 64).abs() <= 1);
    let mesh_maps = fx.render_preset("Mesh maps");
    assert_eq!(mesh_maps.len(), 1, "only baked maps are exported");
    let ao = decode(&mesh_maps[0]);
    assert_eq!(mesh_maps[0].name, "sphere_Body_Set_ambientOcclusion.png");
    assert_eq!(ao.color(), image::ColorType::L16);
}

#[test]
fn mesh_maps_write_tangent_normals_with_the_green_of_the_project_format() {
    let green = |fx: &Fx, opts: &ExportOptions, name: &str| {
        let files = render_maps(&fx.input(), opts).unwrap();
        decode(find(&files, name)).to_rgb16().get_pixel(3, 3)[1] as i32
    };
    let mut fx = Fx::new(&default_channels());
    let stored = |v: f32| std::sync::Arc::new([0.5, v, 0.5].repeat((RES * RES) as usize));
    fx.baked = BakedMaps { resolution: RES, maps: [(BakedMap::BentNormal, stored(0.25)), (BakedMap::Normal, stored(0.25))].into() };
    fx.project.texture_sets[0].baked.extend([BakedMap::BentNormal, BakedMap::Normal]);
    let opts = ExportOptions::new(builtin_preset("Mesh maps").unwrap());
    let quarter = (0.25f32 * 65535.0).round() as i32;
    let three_quarters = (0.75f32 * 65535.0).round() as i32;
    for name in ["_bentNormal.png", "_normal.png"] {
        assert!((green(&fx, &opts, name) - quarter).abs() <= 1, "{name}: OpenGL project keeps the stored green");
    }
    fx.project.normal_format = NormalFormat::DirectX;
    for name in ["_bentNormal.png", "_normal.png"] {
        assert!((green(&fx, &opts, name) - three_quarters).abs() <= 1, "{name}: DirectX project flips green");
    }
    // The export's own format wins over the project's
    let mut gl = opts.clone();
    gl.normal_format = Some(NormalFormat::OpenGl);
    assert!((green(&fx, &gl, "_bentNormal.png") - quarter).abs() <= 1);
    fx.project.normal_format = NormalFormat::OpenGl;
    let mut dx = opts.clone();
    dx.normal_format = Some(NormalFormat::DirectX);
    assert!((green(&fx, &dx, "_bentNormal.png") - three_quarters).abs() <= 1, "a DirectX export of an OpenGL project flips green");
}

#[test]
fn templates_are_sanitized_and_collisions_suffixed() {
    let fx = Fx::new(&default_channels());
    let rough = |file: &str| MapSpec {
        file: file.into(),
        format: FileFormat::Png,
        bit_depth: 8,
        channels: vec![(Dest::L, SourceRef::Channel { channel: Channel::Roughness, component: 0 })],
        inverted: Vec::new(),
    };
    let preset = ExportPreset {
        name: "t".into(),
        maps: vec![
            rough("$project $textureSet:$channel ($colorSpace)"),
            rough("$project $textureSet:$channel ($colorSpace)"),
            rough("con"),
            rough("A"),
            rough("a"),
            rough("."),
        ],
    };
    let opts = ExportOptions::new(preset);
    let planned = list_outputs(&fx.project, 0, "sphere", &opts).unwrap();
    let names: Vec<&str> = planned.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["My_Project_Body_Set_roughness__linear_.png", "My_Project_Body_Set_roughness__linear__2.png", "_con.png", "A.png", "a_2.png", "texture.png"]
    );
    assert!(planned[0].warnings.is_empty());
    assert_eq!(planned[1].warnings.len(), 1);
    assert_eq!(planned[4].warnings.len(), 1);
    let rendered: Vec<String> = fx.render(&opts).into_iter().map(|f| f.name).collect();
    assert_eq!(rendered, names);
}

#[test]
fn padding_modes() {
    let mut fx = Fx::new(&default_channels());
    fx.coverage = (0..RES * RES).map(|i| i % RES < 32).collect();
    let rough_at = |fx: &Fx, padding: Padding, x: u32| {
        let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
        opts.padding = padding;
        decode(find(&fx.render(&opts), "_roughness.png")).to_luma8().get_pixel(x, 10)[0]
    };
    assert_eq!(rough_at(&fx, Padding::None, 10), 64);
    assert_eq!(rough_at(&fx, Padding::None, 40), 0);
    assert_eq!(rough_at(&fx, Padding::Infinite, 63), 64);
    assert_eq!(rough_at(&fx, Padding::Dilation(2), 33), 64);
    assert_eq!(rough_at(&fx, Padding::Dilation(2), 40), 0);
    assert_eq!(rough_at(&fx, Padding::DilationDefault(2), 33), 64);
    assert_eq!(rough_at(&fx, Padding::DilationDefault(2), 40), 128, "roughness default is 0.5");
    let bad = Fx { coverage: vec![true; 10], ..Fx::new(&default_channels()) };
    assert!(render_maps(&bad.input(), &ExportOptions::new(builtin_preset("Separate maps").unwrap())).is_err());
}

#[test]
fn output_size_override() {
    let fx = Fx::new(&default_channels());
    for size in [32u32, 128] {
        let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
        opts.size = Some(size);
        let files = fx.render(&opts);
        let r = find(&files, "_roughness.png");
        assert_eq!((r.width, r.height), (size, size));
        let img = decode(r).to_luma8();
        assert_eq!(img.dimensions(), (size, size));
        assert_eq!(img.get_pixel(size / 2, size / 3)[0], 64);
    }
    let mut opts = ExportOptions::new(builtin_preset("Separate maps").unwrap());
    opts.size = Some(48);
    assert!(render_maps(&fx.input(), &opts).is_err());
}

#[test]
fn every_format_encodes_and_decodes() {
    let mut channels = default_channels();
    channels.push(Channel::Opacity);
    let fx = Fx::new(&channels);
    let gray_alpha = MapSpec {
        file: "la".into(),
        format: FileFormat::Png,
        bit_depth: 8,
        channels: vec![(Dest::L, SourceRef::Channel { channel: Channel::Roughness, component: 0 }), (Dest::A, SourceRef::Constant(0.5))],
        inverted: Vec::new(),
    };
    let mut preset = builtin_preset("glTF PBR Metal Roughness").unwrap();
    preset.maps.push(gray_alpha);

    // JNG, PGM and PSD have tests of their own (the image crate reads none of them)
    for format in FileFormat::ALL.into_iter().filter(|f| !matches!(f, FileFormat::Jng | FileFormat::Pgm | FileFormat::Psd)) {
        for &depth in format.bit_depths() {
            let mut opts = ExportOptions::new(preset.clone());
            opts.format_override = Some(format);
            opts.bit_depth_override = Some(depth);
            let files = fx.render(&opts);
            assert_eq!(files.len(), 4, "{format:?}");
            for f in &files {
                assert!(f.name.ends_with(format.extension()));
                if format == FileFormat::Exr && f.name.starts_with("la") {
                    continue; // Y/A EXR is checked with the EXR crate in `exr_exports_luminance_and_alpha_at_both_depths`.
                }

                assert_eq!(decode(f).dimensions(), (RES, RES), "{}", f.name);
            }

            // Roughness 0.25 everywhere survives every format (JPEG within its loss)
            let px = decode(find(&files, "OcclusionRoughnessMetallic.")).to_rgba32f().get_pixel(1, 1)[1];
            assert!((px - 0.25).abs() < 0.01, "{format:?}/{depth}: {px}");
            let base = find(&files, "_BaseColor.");
            match format {
                FileFormat::Jpeg | FileFormat::Hdr => assert!(base.warnings.iter().any(|w| w.contains("alpha")), "{format:?}"),
                _ => assert!(decode(base).color().has_alpha(), "{format:?} keeps opacity"),
            }
        }
    }
}

#[test]
fn jpeg_texture_export_keeps_full_chroma() {
    let mut channels = default_channels();
    channels.push(Channel::Opacity);
    let fx = Fx::new(&channels);
    let mut opts = ExportOptions::new(builtin_preset("glTF PBR Metal Roughness").unwrap());
    opts.format_override = Some(FileFormat::Jpeg);
    let files = fx.render(&opts);
    let base = find(&files, "_BaseColor.jpg");

    // Find the baseline JPEG frame for the generated 64 × 64 RGB export.
    let frame_header = [0xff, 0xc0, 0, 17, 8, 0, RES as u8, 0, RES as u8, 3];
    let frame = base.bytes.windows(19).find(|segment| segment.starts_with(&frame_header)).expect("RGB JPEG frame");
    assert_eq!([frame[11], frame[14], frame[17]], [0x11; 3], "JPEG component sampling factors");
    assert_eq!(craft_image_codecs::jpeg_quality(&base.bytes), Some(95));
    assert!(base.warnings.iter().any(|w| w.contains("alpha")));
}

#[test]
fn list_outputs_matches_render_and_presets_round_trip() {
    let fx = Fx::new(&default_channels());
    for preset in builtin_presets() {
        preset.validate().unwrap();
        assert_eq!(preset_from_json(&preset_to_json(&preset).unwrap()).unwrap(), preset);
        let opts = ExportOptions::new(preset.clone());
        let planned: Vec<String> = list_outputs(&fx.project, 0, "sphere", &opts).unwrap().into_iter().map(|p| p.name).collect();
        let rendered: Vec<String> = fx.render(&opts).into_iter().map(|f| f.name).collect();
        assert_eq!(planned, rendered, "{}", preset.name);
    }
    assert!(builtin_preset("unity hdrp (mask map)").is_some());
    // OpenPBR writes the lobe maps a set has, and skips the rest.
    let openpbr = ExportOptions::new(builtin_preset("OpenPBR (separate maps)").unwrap());
    let names = |fx: &Fx| list_outputs(&fx.project, 0, "m", &openpbr).unwrap().into_iter().map(|p| p.name).collect::<Vec<String>>();
    let plain = names(&fx);
    assert!(plain.iter().any(|n| n.ends_with("_base_color.png")) && !plain.iter().any(|n| n.contains("coat_")), "{plain:?}");
    let mut lobes = default_channels();
    lobes.extend([Channel::CoatWeight, Channel::CoatRoughness, Channel::SheenWeight]);
    let coated = names(&Fx::new(&lobes));
    assert!(coated.iter().any(|n| n.ends_with("_coat_weight.png")) && coated.iter().any(|n| n.ends_with("_fuzz_weight.png")), "{coated:?}");
    assert!(builtin_preset("godot 4 (orm)").is_some());
    assert!(preset_from_json("{").is_err());
    let dup = r#"{"name":"x","maps":[{"file":"a","format":"Png","bit_depth":8,"channels":[["R",{"Constant":1.0}],["R",{"Constant":0.0}]]}]}"#;
    assert!(preset_from_json(dup).is_err());
    let mixed = r#"{"name":"x","maps":[{"file":"a","format":"Png","bit_depth":8,"channels":[["L",{"Constant":1.0}],["G",{"Constant":0.0}]]}]}"#;
    assert!(preset_from_json(mixed).is_err());
    let ok = r#"{"name":"x","maps":[{"file":"a","format":"Tga","bit_depth":8,"channels":[["L",{"Channel":{"channel":"Metallic"}}]]}]}"#;
    assert!(preset_from_json(ok).is_ok());
    assert!(list_outputs(&fx.project, 5, "m", &ExportOptions::new(builtin_presets().remove(0))).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn write_files_is_atomic_and_refuses_paths() {
    let fx = Fx::new(&default_channels());
    let files = fx.render_preset("Unreal Engine (Packed)");
    let dir = std::env::temp_dir().join(format!("tc-formats-export-{}", std::process::id()));
    let paths = write_files(&files, &dir).unwrap();
    assert_eq!(paths.len(), 3);
    for (p, f) in paths.iter().zip(&files) {
        assert_eq!(std::fs::read(p).unwrap(), f.bytes);
    }
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    let evil = ExportedFile { name: "../evil.png".into(), ..files[0].clone() };
    assert!(write_files(&[evil], &dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn write_files_accepts_planned_name_with_max_length_stem() {
    let fx = Fx::new(&default_channels());
    let files = fx.render_preset("Unreal Engine (Packed)");
    let name = format!("{}.png", sanitize_file_stem(&"a".repeat(250)));
    let dir = std::env::temp_dir().join(format!("tc-formats-export-long-{}", std::process::id()));
    let long = ExportedFile { name, ..files[0].clone() };
    let paths = write_files(&[long], &dir).unwrap();
    assert_eq!(paths.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn gray_map(file: &str, src: SourceRef) -> MapSpec {
    MapSpec { file: file.into(), format: FileFormat::Png, bit_depth: 8, channels: vec![(Dest::L, src)], inverted: Vec::new() }
}

#[test]
fn validation_rejects_bad_presets() {
    let ok = ExportPreset { name: "Mine".into(), maps: vec![gray_map("$mesh_$textureSet_$channel ($colorSpace) $project", SourceRef::Constant(1.0))] };
    ok.validate().unwrap();
    let refused = |f: &dyn Fn(&mut ExportPreset)| {
        let mut p = ok.clone();
        f(&mut p);
        p.validate().is_err()
    };
    let long = "x".repeat(300);
    for file in ["a/b", r"a\b", "..", "x..y", "$meshes_$foo", "$", "", " ", "a\u{7}b", long.as_str()] {
        assert!(refused(&|p| p.maps[0].file = file.into()), "file template {file:?} must be refused");
    }
    assert!(refused(&|p| p.name = "  ".into()));
    assert!(refused(&|p| p.name = "x".repeat(MAX_PRESET_NAME + 1)));
    assert!(refused(&|p| p.name = "a\nb".into()));
    assert!(refused(&|p| p.maps[0].bit_depth = 12));
    assert!(refused(&|p| p.maps[0].channels[0].1 = SourceRef::Channel { channel: Channel::BaseColor, component: 3 }));
    assert!(refused(&|p| p.maps[0].channels[0].1 = SourceRef::Constant(f32::NAN)));
    assert!(refused(&|p| p.maps = vec![p.maps[0].clone(); MAX_MAPS + 1]));
    // The same checks guard JSON from disk or agents.
    let json = preset_to_json(&ok).unwrap();
    assert_eq!(preset_from_json(&json).unwrap(), ok);
    assert!(preset_from_json(&json.replace("$mesh_", "../")).is_err());
    assert!(preset_from_json(&" ".repeat(2 << 20)).is_err());
}

#[test]
fn converted_maps_invert_and_derive_normals() {
    let mut fx = Fx::new(&[Channel::BaseColor, Channel::Roughness, Channel::Glossiness, Channel::Height]);
    fx.set_all(Channel::Glossiness, |_, _| [0.2; 3]);
    fx.set_all(Channel::Height, |x, y| [(x as f32 + y as f32) / 160.0; 3]);
    fx.project.texture_sets[0].height_strength = 8.0;
    let virt = |map| SourceRef::Virtual { map, component: 0 };
    let rgb = |file: &str, map| MapSpec {
        file: file.into(),
        format: FileFormat::Png,
        bit_depth: 8,
        channels: [Dest::R, Dest::G, Dest::B].into_iter().zip(0u8..).map(|(d, k)| (d, SourceRef::Virtual { map, component: k })).collect(),
        inverted: Vec::new(),
    };
    let preset = ExportPreset {
        name: "converted".into(),
        maps: vec![
            gray_map("gloss", virt(VirtualMap::Glossiness)),
            gray_map("rough", virt(VirtualMap::Roughness)),
            rgb("hn", VirtualMap::HeightNormal),
            rgb("dx", VirtualMap::NormalDirectX),
        ],
    };
    let files = fx.render(&ExportOptions::new(preset));
    assert_eq!(decode(find(&files, "gloss")).to_luma8().get_pixel(4, 4)[0], 191, "1 - roughness 0.25");
    assert_eq!(decode(find(&files, "rough")).to_luma8().get_pixel(4, 4)[0], 204, "1 - glossiness 0.2");
    let hn = decode(find(&files, "hn")).to_rgb8();
    let full = decode(find(&fx.render_preset("Separate maps"), "_normal.png")).to_rgb8();
    assert_eq!(hn.get_pixel(30, 30), full.get_pixel(30, 30), "without a normal channel the final normal is the height normal");
    let (a, b) = (hn.get_pixel(30, 30), decode(find(&files, "dx")).to_rgb8().get_pixel(30, 30).to_owned());
    assert!(a[1] > 136 && (a[1] as i32 + b[1] as i32 - 255).abs() <= 1, "DirectX flips green: {a:?} vs {b:?}");

    // Converted maps only exist when their source channel does
    let fx = Fx::new(&default_channels());
    let only = ExportPreset { name: "c".into(), maps: vec![gray_map("rough", virt(VirtualMap::Roughness)), rgb("hn", VirtualMap::HeightNormal)] };
    let names: Vec<String> = list_outputs(&fx.project, 0, "m", &ExportOptions::new(only)).unwrap().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["hn.png"], "the default set has height but no glossiness");
}

#[test]
fn uv_tiles_export_one_file_per_tile_without_padding_across() {
    // Tiles 1001 (left block: red, only x < 16 covered) and 1002 (right block: blue)
    let mut project = Project::new("My Project", "builtin:udim_plane");
    let mut ts = TextureSet::new("Body Set", 0, RES);
    ts.uv_tiles = vec![1001, 1002];
    ts.channels = vec![(Channel::BaseColor, ChannelFormat::default_for(Channel::BaseColor))];
    project.texture_sets.push(ts);
    let mut comp = Composite::with_size(&[Channel::BaseColor], 2 * RES, RES);
    let mut coverage = vec![false; (2 * RES * RES) as usize];
    for y in 0..RES {
        for x in 0..2 * RES {
            let blue = x >= RES;
            if blue || x < 16 {
                comp.channels.get_mut(&Channel::BaseColor).unwrap().set(x, y, &[to_u16(if blue { 0.0 } else { 1.0 }), 0, to_u16(if blue { 1.0 } else { 0.0 })]);
                coverage[(y * 2 * RES + x) as usize] = true;
            }
        }
    }
    let input = ExportInput { project: &project, ts: 0, composite: &comp, baked: &BakedMaps::default(), coverage: &coverage, mesh_name: "udim_plane" };
    let preset = ExportPreset {
        name: "t".into(),
        maps: vec![MapSpec {
            file: "$textureSet_color".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![
                (Dest::R, SourceRef::Channel { channel: Channel::BaseColor, component: 0 }),
                (Dest::G, SourceRef::Channel { channel: Channel::BaseColor, component: 1 }),
                (Dest::B, SourceRef::Channel { channel: Channel::BaseColor, component: 2 }),
            ],
            inverted: Vec::new(),
        }],
    };
    let files = render_maps(&input, &ExportOptions::new(preset.clone())).unwrap();
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Body_Set_color.1001.png", "Body_Set_color.1002.png"]);
    let (left, right) = (decode(&files[0]).to_rgb8(), decode(&files[1]).to_rgb8());
    assert_eq!((left.width(), left.height(), right.width()), (RES, RES, RES));

    // Infinite padding fills tile 1001 from its own island only
    assert!(left.pixels().all(|p| p.0 == [255, 0, 0]), "tile 1001 is red");
    assert!(right.pixels().all(|p| p.0 == [0, 0, 255]), "tile 1002 is blue");

    // `$udim` placed by the template; a size applies per tile; the atlas is one image
    let mut named = preset.clone();
    named.maps[0].file = "$udim_$textureSet".into();
    let opts = ExportOptions { size: Some(32), ..ExportOptions::new(named) };
    let small = render_maps(&input, &opts).unwrap();
    assert_eq!((small[1].name.as_str(), small[1].width, small[1].height), ("1002_Body_Set.png", 32, 32));
    let atlas = render_maps(&input, &ExportOptions { atlas: true, ..ExportOptions::new(preset.clone()) }).unwrap();
    assert_eq!((atlas.len(), atlas[0].name.as_str(), atlas[0].width, atlas[0].height), (1, "Body_Set_color.png", 2 * RES, RES));

    // Diffusion stays inside each tile of the atlas too: tile 1001's gap takes only its red
    let diffused = render_maps(&input, &ExportOptions { atlas: true, padding: Padding::DilationDiffusion(2), ..ExportOptions::new(preset) }).unwrap();
    let img = decode(&diffused[0]).to_rgb8();
    for x in [20, 40, RES - 1] {
        assert_eq!(img.get_pixel(x, 10).0, [255, 0, 0], "({x},10)");
    }
}

#[test]
fn small_export_size_keeps_each_nonempty_udim_tile_one_pixel_or_larger() {
    let mut project = Project::new("p", "builtin:udim_plane");
    let mut set = TextureSet::new("body", 0, 512);
    set.uv_tiles = vec![1001, 1002];
    set.tile_resolutions.insert(1002, 128);
    project.texture_sets.push(set);
    let composite = Composite::with_size(&[Channel::Roughness], 1024, 512);
    let coverage = vec![true; 1024 * 512];
    let input = ExportInput { project: &project, ts: 0, composite: &composite, baked: &BakedMaps::default(), coverage: &coverage, mesh_name: "m" };

    for format in [FileFormat::Png, FileFormat::Psd] {
        let opts = ExportOptions { size: Some(2), padding: Padding::None, ..one_map(format, 8, roughness()) };
        let planned = list_outputs(&project, 0, "m", &opts).unwrap();
        let exported = render_maps(&input, &opts).unwrap();

        assert_eq!(planned.iter().map(|file| file.tile).collect::<Vec<_>>(), [Some(1001), Some(1002)]);
        assert_eq!(exported.iter().map(|file| (file.width, file.height)).collect::<Vec<_>>(), [(2, 2), (1, 1)]);
        for (plan, file) in planned.iter().zip(&exported) {
            assert_eq!(plan.name, file.name);
        }
        if format == FileFormat::Png {
            assert_eq!(decode(&exported[1]).dimensions(), (1, 1));
        }
    }
}

#[test]
fn the_udim_name_is_the_file_name_with_its_tile_number_as_placeholder() {
    let mut project = Project::new("p", "builtin:udim_plane");
    let mut ts = TextureSet::new("Body", 0, RES);
    ts.uv_tiles = vec![1001, 1002];
    ts.channels = vec![(Channel::BaseColor, ChannelFormat::default_for(Channel::BaseColor))];
    project.texture_sets.push(ts);
    let udim_names = |file: String| {
        let channels = vec![(Dest::R, SourceRef::Channel { channel: Channel::BaseColor, component: 0 })];
        let preset = ExportPreset { name: "t".into(), maps: vec![MapSpec { file, format: FileFormat::Png, bit_depth: 8, channels, inverted: Vec::new() }] };
        list_outputs(&project, 0, "m", &ExportOptions::new(preset)).unwrap().into_iter().map(|f| f.udim_name).collect::<Vec<_>>()
    };
    let placeholder = Some("Body_<UDIM>.png".to_string());
    assert_eq!(udim_names("$textureSet_$udim".into()), [placeholder.clone(), placeholder]);

    // The length limit cuts the longer placeholder where it doesn't cut the tile number: none then
    assert_eq!(udim_names(format!("{}_$udim_$textureSet", "a".repeat(MAX_STEM - 10))), [None, None]);
}

#[test]
fn lobe_channels_export_as_document_maps_and_gltf_extension_maps() {
    let mut channels = default_channels();
    channels.extend(Channel::LOBES);
    let mut fx = Fx::new(&channels);
    fx.set_all(Channel::CoatWeight, |_, _| [1.0; 3]);
    fx.set_all(Channel::CoatRoughness, |_, _| [0.2; 3]);
    fx.set_all(Channel::SheenWeight, |_, _| [0.5; 3]);
    fx.set_all(Channel::SheenColor, |_, _| [1.0, 0.5, 0.0]);
    fx.set_all(Channel::SheenRoughness, |_, _| [0.4; 3]);
    let doc = fx.render_preset("Document channels + Normal + AO");
    for c in Channel::LOBES {
        find(&doc, &format!("_{}.", c.id()));
    }
    let gltf = fx.render_preset("glTF PBR Metal Roughness");
    let coat = decode(find(&gltf, "_Clearcoat.png")).to_rgb8();
    let px = coat.get_pixel(3, 3);
    assert!(px[0] == 255 && (px[1] as i32 - 51).abs() <= 1 && px[2] == 0, "weight in R, roughness in G: {px:?}");
    let sheen = decode(find(&gltf, "_Sheen.png")).to_rgba8();
    let px = sheen.get_pixel(3, 3);
    // sRGB color × weight (0.5, 0.25, 0) and linear roughness in alpha.
    let want = [188, 137, 0, 102];
    assert!(px.0.iter().zip(want).all(|(a, b)| (*a as i32 - b).abs() <= 1), "{px:?}");
    let plain = Fx::new(&default_channels()).render_preset("glTF PBR Metal Roughness");
    assert!(!plain.iter().any(|f| f.name.contains("Clearcoat") || f.name.contains("Sheen")), "sets without the lobes export no extension maps");
}

#[test]
fn colour_space_converts_colour_maps_only() {
    let fx = Fx::new(&default_channels());
    let preset = builtin_preset("glTF PBR Metal Roughness").unwrap();
    let want_at = |space: ColorSpace, x, y| space.from_linear_rec709(fx.comp.value(Channel::BaseColor, x, y));
    // ACEScg in a float EXR: the base colour is the converted triple, above-1 values aside.
    let mut exr = ExportOptions::new(preset.clone());
    exr.format_override = Some(FileFormat::Exr);
    exr.bit_depth_override = Some(32);
    exr.color_space = Some(ColorSpace::AcesCg);
    let files = fx.render(&exr);
    let img = decode(find(&files, "_BaseColor.exr")).to_rgb32f();
    for (x, y) in [(0, 0), (10, 40), (63, 63)] {
        let (px, want) = (img.get_pixel(x, y), want_at(ColorSpace::AcesCg, x, y));
        assert!((0..3).all(|k| (px[k] - want[k]).abs() < 1e-4), "({x},{y}) {px:?} vs {want:?}");
    }
    // Data maps are never converted: roughness and metalness (packed in the ORM map) are as before.
    let default_files =
        fx.render(&ExportOptions { format_override: Some(FileFormat::Exr), bit_depth_override: Some(32), ..ExportOptions::new(preset.clone()) });
    assert_eq!(find(&files, "OcclusionRoughnessMetallic").bytes, find(&default_files, "OcclusionRoughnessMetallic").bytes);
    assert_eq!(find(&files, "_Normal").bytes, find(&default_files, "_Normal").bytes);
    assert_ne!(find(&files, "_BaseColor").bytes, find(&default_files, "_BaseColor").bytes);

    // Linear Rec.709 in an 8-bit PNG: the values are written without the sRGB curve.
    let mut lin = ExportOptions::new(preset.clone());
    lin.color_space = Some(ColorSpace::LinearRec709);
    let png = decode(find(&fx.render(&lin), "_BaseColor.png"));
    let v = fx.comp.value(Channel::BaseColor, 10, 40);
    for k in 0..3 {
        assert!((png.get_pixel(10, 40)[k] as f32 - v[k] * 255.0).abs() <= 1.0, "[{k}]");
    }
    // The default is unchanged: sRGB for integer files (None and explicit sRGB agree).
    let default_png = fx.render(&ExportOptions::new(preset.clone()));
    let mut srgb = ExportOptions::new(preset.clone());
    srgb.color_space = Some(ColorSpace::Srgb);
    let srgb_png = fx.render(&srgb);
    assert_eq!(decode(find(&default_png, "_BaseColor.png")).to_rgb8(), decode(find(&srgb_png, "_BaseColor.png")).to_rgb8());
    // The chosen one is written into the file as its profile; the default says nothing, as before.
    assert_eq!(embedded_icc(FileFormat::Png, &find(&srgb_png, "_BaseColor.png").bytes), crate::icc::profile(ColorSpace::Srgb));
    assert_eq!(embedded_icc(FileFormat::Png, &find(&default_png, "_BaseColor.png").bytes), None);
    // Data maps carry none, even when a space is chosen.
    assert_eq!(embedded_icc(FileFormat::Png, &find(&srgb_png, "_Normal.png").bytes), None);
    // An explicit sRGB in a float file is encoded once, not twice.
    let mut float_srgb = ExportOptions::new(preset);
    float_srgb.format_override = Some(FileFormat::Exr);
    float_srgb.bit_depth_override = Some(32);
    float_srgb.color_space = Some(ColorSpace::Srgb);
    let px = *decode(find(&fx.render(&float_srgb), "_BaseColor.exr")).to_rgb32f().get_pixel(10, 40);
    assert!((px[0] - linear_to_srgb(v[0])).abs() < 1e-4, "{px:?}");
}

#[test]
fn colour_space_names_the_files_and_warns_when_integer_files_clamp() {
    let fx = Fx::new(&default_channels());
    let named = ExportPreset {
        name: "named".into(),
        maps: vec![MapSpec {
            file: "$textureSet_$channel_$colorSpace".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![
                (Dest::R, SourceRef::Channel { channel: Channel::BaseColor, component: 0 }),
                (Dest::G, SourceRef::Channel { channel: Channel::BaseColor, component: 1 }),
                (Dest::B, SourceRef::Channel { channel: Channel::BaseColor, component: 2 }),
            ],
            inverted: Vec::new(),
        }],
    };
    let plan = |space: Option<ColorSpace>, format: Option<FileFormat>| {
        let opts = ExportOptions { color_space: space, format_override: format, ..ExportOptions::new(named.clone()) };
        list_outputs(&fx.project, 0, "sphere", &opts).unwrap().remove(0)
    };
    assert!(plan(None, None).name.ends_with("_sRGB.png") && plan(None, Some(FileFormat::Exr)).name.ends_with("_linear.exr"), "defaults keep their names");
    assert!(plan(Some(ColorSpace::LinearRec709), None).name.ends_with("_linear.png"));
    let aces = plan(Some(ColorSpace::AcesCg), None);
    assert!(aces.name.ends_with("_ACEScg.png"), "{}", aces.name);
    assert!(aces.warnings.iter().any(|w| w.contains("clamped")), "{:?}", aces.warnings);
    assert!(plan(Some(ColorSpace::AcesCg), Some(FileFormat::Exr)).warnings.is_empty(), "float files keep the values");
    assert!(plan(None, None).warnings.is_empty() && plan(Some(ColorSpace::LinearRec709), None).warnings.is_empty());

    // The planned names are the rendered names
    let opts = ExportOptions { color_space: Some(ColorSpace::Aces2065), ..ExportOptions::new(named) };
    let names: Vec<String> = fx.render(&opts).into_iter().map(|f| f.name).collect();
    assert_eq!(names, list_outputs(&fx.project, 0, "sphere", &opts).unwrap().into_iter().map(|p| p.name).collect::<Vec<_>>());
    assert!(names[0].ends_with("_ACES2065-1.png"), "{names:?}");
}

#[test]
fn colour_packed_into_one_channel_warns_instead_of_converting_silently() {
    let fx = Fx::new(&default_channels());
    let preset = ExportPreset { name: "l".into(), maps: vec![gray_map("$textureSet_red", SourceRef::Channel { channel: Channel::BaseColor, component: 0 })] };
    let opts = |space| ExportOptions { color_space: space, format_override: Some(FileFormat::Exr), ..ExportOptions::new(preset.clone()) };
    assert!(fx.render(&opts(Some(ColorSpace::AcesCg)))[0].warnings.iter().any(|w| w.contains("ACEScg")));
    assert!(fx.render(&opts(None))[0].warnings.is_empty(), "linear Rec.709 needs no conversion");
}

/// The ICC profile a PNG, TIFF, JPEG or WebP file carries.
fn embedded_icc(format: FileFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    let c = || Cursor::new(bytes);
    match format {
        FileFormat::Png => image::codecs::png::PngDecoder::new(c()).ok()?.icc_profile().ok()?,
        FileFormat::Tiff => image::codecs::tiff::TiffDecoder::new(c()).ok()?.icc_profile().ok()?,
        FileFormat::Jpeg => image::codecs::jpeg::JpegDecoder::new(c()).ok()?.icc_profile().ok()?,
        FileFormat::Webp => image::codecs::webp::WebPDecoder::new(c()).ok()?.icc_profile().ok()?,
        _ => None,
    }
}

#[test]
fn a_chosen_colour_space_is_embedded_as_an_icc_profile() {
    let data = vec![0.25f32; 8 * 8 * 3];
    for format in [FileFormat::Png, FileFormat::Tiff, FileFormat::Jpeg, FileFormat::Webp] {
        for depth in [8u8, 16] {
            if depth == 16 && matches!(format, FileFormat::Jpeg | FileFormat::Webp) {
                continue;
            }

            for space in ColorSpace::ALL {
                let mut warnings = Vec::new();
                let bytes = encode(Encoding { format, depth, dither: false }, (8, 8), 3, data.clone(), &[false; 3], Some(space), &mut warnings).unwrap();
                assert_eq!(embedded_icc(format, &bytes), crate::icc::profile(space), "{format:?} {depth}-bit {space:?}");
                assert!(warnings.is_empty(), "{warnings:?}");
            }

            // No colour space chosen (a data map, or the default): nothing is embedded
            let bytes = encode(Encoding { format, depth, dither: false }, (8, 8), 3, data.clone(), &[false; 3], None, &mut Vec::new()).unwrap();
            assert_eq!(embedded_icc(format, &bytes), None, "{format:?} {depth}-bit");
        }
    }
}

#[test]
fn exr_carries_the_chromaticities_of_the_space() {
    let data = vec![0.25f32; 4 * 4 * 3];
    let read = |bytes: &[u8]| {
        let meta = exr::meta::MetaData::read_from_buffered(Cursor::new(bytes), false).unwrap();
        meta.headers[0].shared_attributes.chromaticities
    };
    assert!(
        read(&encode(Encoding { format: FileFormat::Exr, depth: 16, dither: false }, (4, 4), 3, data.clone(), &[false; 3], None, &mut Vec::new()).unwrap())
            .is_none()
    );
    for space in ColorSpace::ALL {
        let c = read(
            &encode(Encoding { format: FileFormat::Exr, depth: 32, dither: false }, (4, 4), 3, data.clone(), &[false; 3], Some(space), &mut Vec::new())
                .unwrap(),
        )
        .unwrap_or_else(|| panic!("{space:?}"));
        let [r, g, b, w] = space.chromaticities();
        for (got, want) in [(c.red, r), (c.green, g), (c.blue, b), (c.white, w)] {
            assert!((f64::from(got.0) - want[0]).abs() < 1e-5 && (f64::from(got.1) - want[1]).abs() < 1e-5, "{space:?}: {got:?} vs {want:?}");
        }
    }
}

#[test]
fn formats_without_a_place_for_a_colour_space_say_so() {
    let data = vec![0.25f32; 4 * 4 * 3];
    for format in [FileFormat::Tga, FileFormat::Bmp, FileFormat::Hdr] {
        let mut warnings = Vec::new();
        encode(Encoding { format, depth: 8, dither: false }, (4, 4), 3, data.clone(), &[false; 3], Some(ColorSpace::AcesCg), &mut warnings).unwrap();
        assert!(warnings.iter().any(|w| w.contains("colour space") && w.contains("ACEScg")), "{format:?}: {warnings:?}");
        let mut quiet = Vec::new();
        encode(Encoding { format, depth: 8, dither: false }, (4, 4), 3, data.clone(), &[false; 3], None, &mut quiet).unwrap();
        assert!(quiet.is_empty(), "{format:?}: nothing to say without a chosen space: {quiet:?}");
    }
}

/// R packs roughness (0.25) and B metallic (0.75); G and A are constants.
fn invert_map(depth: u8, inverted: Vec<Dest>) -> MapSpec {
    let ch = |channel| SourceRef::Channel { channel, component: 0 };
    MapSpec {
        file: "packed".into(),
        format: FileFormat::Png,
        bit_depth: depth,
        channels: vec![
            (Dest::R, ch(Channel::Roughness)),
            (Dest::G, SourceRef::Constant(0.5)),
            (Dest::B, ch(Channel::Metallic)),
            (Dest::A, SourceRef::Constant(1.0)),
        ],
        inverted,
    }
}

#[test]
fn inverted_channels_write_one_minus_the_source() {
    let fx = Fx::new(&default_channels());
    for depth in [8u8, 16] {
        let preset = |inverted| ExportPreset { name: "inv".into(), maps: vec![invert_map(depth, inverted)] };
        let px = |p: ExportPreset| {
            let files = fx.render(&ExportOptions::new(p));
            let img = decode(find(&files, "packed")).to_rgba16();
            let p = img.get_pixel(9, 9).0;

            // Compare 16-bit values for both depths: an 8-bit file widens to value * 257
            p.map(i32::from)
        };
        let plain = px(preset(Vec::new()));
        let inv = px(preset(vec![Dest::R, Dest::A]));
        let tol = if depth == 8 { 257 } else { 2 };

        // Roughness 0.25 inverts to 0.75, constant alpha 1 to 0; G and B stay
        assert!((inv[0] - (65535 - plain[0])).abs() <= tol, "depth {depth}: R {} vs {}", inv[0], 65535 - plain[0]);
        assert!((inv[0] - 49151).abs() <= tol, "depth {depth}: R {}", inv[0]);
        assert!(inv[3] <= tol, "depth {depth}: A {}", inv[3]);
        assert_eq!((inv[1], inv[2]), (plain[1], plain[2]), "depth {depth}: channels not listed stay as they are");
        assert!((plain[0] - 16384).abs() <= tol && plain[3] == 65535, "depth {depth}: un-inverted {plain:?}");
    }

    // Gray maps and float files too
    let gray = gray_map("g", SourceRef::Channel { channel: Channel::Metallic, component: 0 });
    let gray = MapSpec { inverted: vec![Dest::L], ..gray };
    let files = fx.render(&ExportOptions::new(ExportPreset { name: "inv".into(), maps: vec![gray] }));
    assert_eq!(decode(find(&files, "g.png")).to_luma8().get_pixel(4, 4)[0], 64, "1 - metallic 0.75");
    let mut float = invert_map(32, vec![Dest::B]);
    float.format = FileFormat::Exr;
    let files = fx.render(&ExportOptions::new(ExportPreset { name: "inv".into(), maps: vec![float] }));
    let v = decode(find(&files, "packed.exr")).to_rgba32f().get_pixel(4, 4).0;
    assert!((v[2] - 0.25).abs() < 1e-3 && (v[0] - 0.25).abs() < 1e-3, "metallic 0.75 inverted, roughness 0.25 not: {v:?}");
}

#[test]
fn inverted_flag_round_trips_and_old_json_loads() {
    let mut map = invert_map(8, vec![Dest::B]);
    let preset = ExportPreset { name: "Mine".into(), maps: vec![map.clone()] };
    let json = preset_to_json(&preset).unwrap();
    assert_eq!(preset_from_json(&json).unwrap(), preset);

    // A file written before the flag existed has no `inverted` and keeps none on save
    map.inverted.clear();
    let old = ExportPreset { name: "Old".into(), maps: vec![map] };
    let json = preset_to_json(&old).unwrap();
    assert!(!json.contains("inverted"), "{json}");
    assert_eq!(preset_from_json(&json).unwrap(), old);
    assert!(preset_from_json(&json).unwrap().maps[0].inverted.is_empty());
}

#[test]
fn validation_rejects_inverting_a_channel_the_map_lacks() {
    let ok = ExportPreset { name: "Mine".into(), maps: vec![invert_map(8, vec![Dest::R])] };
    ok.validate().unwrap();
    let mut bad = ok.clone();
    bad.maps[0].inverted = vec![Dest::L];
    assert!(bad.validate().is_err());
    bad.maps[0].inverted = vec![Dest::R, Dest::R];
    assert!(bad.validate().is_err());
}

#[test]
fn inverting_a_colour_channel_inverts_the_linear_value_before_encoding() {
    let fx = Fx::new(&default_channels());
    let base = |inverted: Vec<Dest>, size: Option<u32>| {
        let map = MapSpec {
            file: "col".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: [Dest::R, Dest::G, Dest::B]
                .into_iter()
                .zip(0u8..)
                .map(|(d, k)| (d, SourceRef::Channel { channel: Channel::BaseColor, component: k }))
                .collect(),
            inverted,
        };
        let mut opts = ExportOptions::new(ExportPreset { name: "c".into(), maps: vec![map] });
        opts.size = size;
        decode(find(&fx.render(&opts), "col")).to_rgb8()
    };
    let (plain, inv) = (base(Vec::new(), None), base(vec![Dest::R], None));

    // Texel (9, 9): linear red 9/63, written as sRGB
    let want = linear_to_srgb(1.0 - 9.0 / 63.0) * 255.0;
    assert!((inv.get_pixel(9, 9)[0] as f32 - want).abs() <= 1.0, "{} vs {want}", inv.get_pixel(9, 9)[0]);
    assert_eq!(inv.get_pixel(9, 9).0[1..], plain.get_pixel(9, 9).0[1..], "G and B stay");

    // A size override goes through the resize path: the left-most texels are 1 - 0 -> white red
    let half = base(vec![Dest::R], Some(RES / 2));
    assert_eq!(half.width(), RES / 2);
    assert!(half.get_pixel(0, 5)[0] >= 250, "{}", half.get_pixel(0, 5)[0]);
}

#[test]
fn the_default_template_keeps_its_old_name_and_file_types_their_listed_names() {
    let old = builtin_preset("Document channels + Normal + AO").unwrap();
    assert_eq!(old.name, crate::DEFAULT_PRESET, "settings saved under the old name still find the template");
    assert_eq!(builtin_preset(crate::DEFAULT_PRESET).unwrap(), old);
    for f in FileFormat::ALL {
        assert_eq!(FileFormat::from_extension(f.name()), Some(f), "{} reads back", f.name());
    }
    assert_eq!(FileFormat::Tga.name(), "targa");
    assert_eq!(FileFormat::Jpeg.name(), "jpeg");
}

/// One-map preset: `channels` written as `format` at `depth`
fn one_map(format: FileFormat, depth: u8, channels: Vec<(Dest, SourceRef)>) -> ExportOptions {
    ExportOptions::new(ExportPreset { name: "t".into(), maps: vec![MapSpec { file: "out".into(), format, bit_depth: depth, channels, inverted: Vec::new() }] })
}

fn rgba_half_alpha() -> Vec<(Dest, SourceRef)> {
    let c = |k| SourceRef::Channel { channel: Channel::BaseColor, component: k };
    vec![(Dest::R, c(0)), (Dest::G, c(1)), (Dest::B, c(2)), (Dest::A, SourceRef::Constant(0.5))]
}

fn roughness() -> Vec<(Dest, SourceRef)> {
    vec![(Dest::L, SourceRef::Channel { channel: Channel::Roughness, component: 0 })]
}

#[test]
fn the_new_file_types_have_the_reference_bit_depths() {
    assert_eq!(FileFormat::Tiff.bit_depths(), &[8, 16, 32]);
    for f in [FileFormat::Png, FileFormat::Pgm, FileFormat::Psd] {
        assert_eq!(f.bit_depths(), &[8, 16], "{f:?}");
    }
    for f in [FileFormat::Bmp, FileFormat::Jpeg, FileFormat::Jng, FileFormat::Tga, FileFormat::Webp] {
        assert_eq!(f.bit_depths(), &[8], "{f:?}");
    }
    assert_eq!((FileFormat::Exr.default_depth(), FileFormat::Hdr.default_depth(), FileFormat::Psd.default_depth()), (32, 32, 8));
    assert!(FileFormat::Tiff.holds_float(32) && !FileFormat::Tiff.holds_float(16));
}

#[test]
fn pgm_is_a_binary_graymap() {
    let fx = Fx::new(&default_channels());
    let files = fx.render(&one_map(FileFormat::Pgm, 16, roughness()));
    let f = find(&files, "out.pgm");
    let header = b"P5\n64 64\n65535\n";
    assert!(f.bytes.starts_with(header));
    assert_eq!(f.bytes.len(), header.len() + 64 * 64 * 2);
    let first = u16::from_be_bytes([f.bytes[header.len()], f.bytes[header.len() + 1]]);
    assert!((i32::from(first) - 16384).abs() <= 2, "roughness 0.25: {first}");

    // Colour becomes its luminance, weighted in linear light, with a warning
    let mut color = one_map(FileFormat::Pgm, 8, rgba_half_alpha());
    color.preset.maps[0].file = "out_$colorSpace".into();
    let files = fx.render(&color);
    let f = find(&files, "out_sRGB.pgm");
    let header = b"P5\n64 64\n255\n";
    assert!(f.bytes.starts_with(header) && f.warnings.iter().any(|w| w.contains("luminance")), "{:?}", f.warnings);
    let [r, g, b] = fx.comp.value(Channel::BaseColor, 63, 0);
    let want = (linear_to_srgb(0.2126 * r + 0.7152 * g + 0.0722 * b) * 255.0).round();
    assert!((f32::from(f.bytes[header.len() + 63]) - want).abs() <= 1.0, "{} vs {want}", f.bytes[header.len() + 63]);

    // In another colour space the luminance is still that of the colour, not of its converted primaries
    color.preset.maps[0].bit_depth = 16;
    let opts = ExportOptions { color_space: Some(ColorSpace::AcesCg), ..color };
    let files = fx.render(&opts);
    let f = find(&files, "out_linear.pgm");
    assert!(f.warnings.iter().any(|warning| warning.contains("ACEScg") && warning.contains("cannot")), "{:?}", f.warnings);
    assert!(!f.warnings.iter().any(|warning| warning.contains("clamped")), "{:?}", f.warnings);
    let header = b"P5\n64 64\n65535\n";
    let at = header.len() + 63 * 2;
    let got = f32::from(u16::from_be_bytes([f.bytes[at], f.bytes[at + 1]])) / 65535.0;
    let want = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    assert!((got - want).abs() < 1e-3, "ACEScg luminance {got} vs {want}");

    // Gray + alpha only loses the alpha
    let la = vec![(Dest::L, SourceRef::Channel { channel: Channel::Roughness, component: 0 }), (Dest::A, SourceRef::Constant(0.5))];
    let f = &fx.render(&one_map(FileFormat::Pgm, 8, la))[0];
    assert!(f.warnings.iter().any(|w| w.contains("alpha")) && !f.warnings.iter().any(|w| w.contains("luminance")), "{:?}", f.warnings);
}

#[test]
fn psd_holds_the_merged_image_planar() {
    let fx = Fx::new(&default_channels());
    let f = &fx.render(&one_map(FileFormat::Psd, 8, rgba_half_alpha()))[0];
    let b = &f.bytes;
    assert_eq!(&b[0..4], b"8BPS");
    assert_eq!(u16::from_be_bytes([b[12], b[13]]), 4, "channels");
    assert_eq!((u32::from_be_bytes(b[14..18].try_into().unwrap()), u32::from_be_bytes(b[18..22].try_into().unwrap())), (64, 64));
    assert_eq!((u16::from_be_bytes([b[22], b[23]]), u16::from_be_bytes([b[24], b[25]])), (8, 3), "8-bit RGB");
    let parsed = craft_psd::PsdFile::from_bytes(b).unwrap();
    assert!(!parsed.merged_has_alpha());
    assert_eq!(parsed.alpha_channel_names(), ["Alpha"]);
    let samples = parsed.decode_merged().unwrap();
    assert_eq!(samples.len(), 4 * 64 * 64);
    let plane = |c: usize| &samples[c * 64 * 64..(c + 1) * 64 * 64];
    assert!(plane(3).iter().all(|a| *a == 128), "alpha plane");
    let png = decode(&fx.render(&one_map(FileFormat::Png, 8, rgba_half_alpha()))[0]).to_rgba8();
    assert_eq!(plane(0)[64 * 10 + 40], png.get_pixel(40, 10)[0], "the red plane holds the same values as a PNG");

    let f16 = &fx.render(&one_map(FileFormat::Psd, 16, roughness()))[0];
    assert_eq!((u16::from_be_bytes([f16.bytes[12], f16.bytes[13]]), u16::from_be_bytes([f16.bytes[22], f16.bytes[23]])), (1, 16));
    assert_eq!(f16.bytes.len(), 26 + 12 + 2 + 2 * 64 * 64);

    let mut gray_alpha = roughness();
    gray_alpha.push((Dest::A, SourceRef::Constant(0.5)));
    let f16 = &fx.render(&one_map(FileFormat::Psd, 16, gray_alpha))[0];
    let parsed = craft_psd::PsdFile::from_bytes(&f16.bytes).unwrap();
    assert!(!parsed.merged_has_alpha());
    assert_eq!(parsed.alpha_channel_names(), ["Alpha"]);
    let samples = parsed.decode_merged().unwrap();
    assert_eq!(samples.len(), 2 * 2 * 64 * 64);
    assert!(samples[2 * 64 * 64..].as_chunks::<2>().0.iter().all(|sample| *sample == [128, 0]));
}

#[test]
fn psd_plan_rejects_a_version_one_atlas_over_its_dimension_limit() {
    let mut fx = Fx::new(&default_channels());
    fx.project.texture_sets[0].uv_tiles = vec![1001, 1002];
    let mut options = one_map(FileFormat::Psd, 8, roughness());
    options.size = Some(16_384);
    options.atlas = true;

    let error = list_outputs(&fx.project, 0, "sphere", &options).unwrap_err();

    assert!(error.to_string().contains("PSD") && error.to_string().contains("32768"));
    options.atlas = false;
    assert_eq!(list_outputs(&fx.project, 0, "sphere", &options).unwrap().len(), 2);
}

#[test]
fn psd_plan_rejects_a_file_over_the_version_one_byte_limit() {
    let fx = Fx::new(&default_channels());
    let mut options = one_map(FileFormat::Psd, 16, rgba_half_alpha());
    options.size = Some(16_384);

    let error = list_outputs(&fx.project, 0, "sphere", &options).unwrap_err();

    assert!(error.to_string().contains("PSD") && error.to_string().contains("2 GiB"), "{error}");
    options.bit_depth_override = Some(8);
    assert_eq!(list_outputs(&fx.project, 0, "sphere", &options).unwrap().len(), 1);
    options.bit_depth_override = None;
    options.preset.maps[0].channels.pop();
    assert_eq!(list_outputs(&fx.project, 0, "sphere", &options).unwrap().len(), 1);
}

#[test]
fn psd_byte_limit_includes_resources_and_keeps_separate_tiles_valid() {
    // These RGBA16 planes are 32 bytes below 2 GiB before headers and the alpha name
    let (width, height) = (10_924, 24_573);
    assert!(check_psd_size((width, height), 4, 16).is_err());
    assert!(check_psd_size((width, height - 1), 4, 16).is_ok());

    let mut fx = Fx::new(&default_channels());
    fx.project.texture_sets[0].uv_tiles = vec![1001, 1003, 1011];
    let mut options = one_map(FileFormat::Psd, 16, rgba_half_alpha());
    options.size = Some(8192);
    options.atlas = true;

    let error = list_outputs(&fx.project, 0, "sphere", &options).unwrap_err();

    assert!(error.to_string().contains("2 GiB"), "{error}");
    options.atlas = false;
    assert_eq!(list_outputs(&fx.project, 0, "sphere", &options).unwrap().len(), 3);
}

#[test]
fn jng_has_jpeg_colour_and_lossless_alpha() {
    use std::io::Read;
    let fx = Fx::new(&default_channels());
    let f = &fx.render(&one_map(FileFormat::Jng, 8, rgba_half_alpha()))[0];
    assert!(f.name.ends_with(".jng"));
    let b = &f.bytes;
    assert_eq!(&b[0..8], &[0x8B, b'J', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    // Walk the chunks, checking each CRC
    let mut at = 8;
    let mut chunks = Vec::new();
    while at < b.len() {
        let len = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        let kind = &b[at + 4..at + 8];
        let data = &b[at + 8..at + 8 + len];
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        assert_eq!(crc.sum(), u32::from_be_bytes(b[at + 8 + len..at + 12 + len].try_into().unwrap()), "{}", String::from_utf8_lossy(kind));
        chunks.push((String::from_utf8_lossy(kind).into_owned(), data.to_vec()));
        at += 12 + len;
    }
    let names: Vec<&str> = chunks.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["JHDR", "IDAT", "JDAT", "IEND"]);
    let jhdr = &chunks[0].1;
    assert_eq!(&jhdr[8..], &[14, 8, 8, 0, 8, 0, 0, 0], "colour + alpha, 8-bit JPEG, 8-bit deflate alpha");
    let jpeg = image::load_from_memory_with_format(&chunks[2].1, ImageFormat::Jpeg).unwrap();
    assert_eq!(jpeg.dimensions(), (64, 64));
    let mut alpha = Vec::new();
    flate2::read::ZlibDecoder::new(&chunks[1].1[..]).read_to_end(&mut alpha).unwrap();
    assert_eq!(alpha.len(), 64 * 65);
    assert!(alpha.chunks(65).all(|row| row[0] == 0 && row[1..].iter().all(|a| *a == 128)));

    // Gray without alpha: no alpha chunk
    let gray = &fx.render(&one_map(FileFormat::Jng, 8, roughness()))[0];
    assert_eq!(gray.bytes[8 + 8 + 8], 8, "gray colour type");
    assert!(!gray.bytes.windows(4).any(|w| w == b"IDAT"));
}

#[test]
fn tiff_32f_holds_linear_floats() {
    let fx = Fx::new(&default_channels());
    let f = &fx.render(&one_map(FileFormat::Tiff, 32, roughness()))[0];
    let img = decode(f).to_rgb32f();
    assert!((img.get_pixel(5, 5)[0] - 0.25).abs() < 1e-3, "{:?}", img.get_pixel(5, 5));
    let colour = &fx.render(&one_map(FileFormat::Tiff, 32, rgba_half_alpha()))[0];
    let img = decode(colour).to_rgba32f();
    let v = fx.comp.value(Channel::BaseColor, 40, 10);
    assert!((img.get_pixel(40, 10)[0] - v[0]).abs() < 1e-3, "colour stays linear: {:?} vs {v:?}", img.get_pixel(40, 10));
    assert_eq!(img.get_pixel(40, 10)[3], 0.5);
}

#[test]
fn float_tiff_marks_straight_alpha_and_preserves_the_color_profile() {
    use craft_image_codecs::{ExtraSampleKind, tiff_info};

    let fx = Fx::new(&default_channels());
    let options = ExportOptions { color_space: Some(ColorSpace::Srgb), ..one_map(FileFormat::Tiff, 32, rgba_half_alpha()) };
    let files = fx.render(&options);
    let bytes = &files[0].bytes;

    assert_eq!(tiff_info(bytes).unwrap().pages[0].extra_samples, [ExtraSampleKind::UnassociatedAlpha]);
    assert_eq!(embedded_icc(FileFormat::Tiff, bytes), crate::icc::profile(ColorSpace::Srgb));
    let pixel = decode(&files[0]).to_rgba32f().get_pixel(40, 10).0;
    assert!((pixel[0] - linear_to_srgb(fx.comp.value(Channel::BaseColor, 40, 10)[0])).abs() < 1e-3);
    assert_eq!(pixel[3], 0.5);

    let mut gray_alpha = roughness();
    gray_alpha.push((Dest::A, SourceRef::Constant(0.75)));
    let files = fx.render(&one_map(FileFormat::Tiff, 32, gray_alpha));
    let file = &files[0];

    assert_eq!(tiff_info(&file.bytes).unwrap().pages[0].extra_samples, [ExtraSampleKind::UnassociatedAlpha]);
    assert_eq!(embedded_icc(FileFormat::Tiff, &file.bytes), None);
    let pixel = decode(file).to_rgba32f().get_pixel(5, 5).0;
    assert!((pixel[0] - 0.25).abs() < 1e-3);
    assert_eq!(pixel, [pixel[0], pixel[0], pixel[0], 0.75]);
}

#[test]
fn dithering_is_ordered_and_keeps_the_average() {
    let fx = Fx::new(&default_channels());
    let plain = one_map(FileFormat::Png, 8, rgba_half_alpha());
    let dithered = ExportOptions { dither: true, ..plain.clone() };
    let a = decode(&fx.render(&plain)[0]).to_rgba8();
    let b = decode(&fx.render(&dithered)[0]).to_rgba8();
    assert_ne!(a, b);
    assert_eq!(b, decode(&fx.render(&dithered)[0]).to_rgba8(), "the same pattern on every export");
    let mean = |img: &image::RgbaImage| img.pixels().map(|p| f64::from(p[0])).sum::<f64>() / 4096.0;
    assert!((mean(&a) - mean(&b)).abs() < 0.6, "{} vs {}", mean(&a), mean(&b));

    // A map's own setting wins, and 16-bit files are never dithered
    let mut own = dithered.clone();
    own.maps.insert(0, MapOverride { dither: Some(false), ..MapOverride::default() });
    assert_eq!(decode(&fx.render(&own)[0]).to_rgba8(), a);
    let wide = ExportOptions { bit_depth_override: Some(16), ..dithered.clone() };
    assert!(list_outputs(&fx.project, 0, "sphere", &wide).unwrap().iter().all(|p| !p.dither));
}

#[test]
fn diffusion_padding_fills_beyond_the_dilation_smoothly() {
    let mut fx = Fx::new(&default_channels());
    fx.set_all(Channel::Roughness, |_, y| [if y < RES / 2 { 0.2 } else { 0.8 }; 3]);
    fx.coverage = (0..RES * RES).map(|i| i % RES < 16).collect();
    let mut dil = one_map(FileFormat::Png, 16, roughness());
    dil.padding = Padding::Dilation(2);
    let mut dif = dil.clone();
    dif.padding = Padding::DilationDiffusion(2);
    let a = decode(&fx.render(&dil)[0]).to_luma16();
    let b = decode(&fx.render(&dif)[0]).to_luma16();
    for y in [0, 20, 40, 63] {
        for x in 0..18 {
            assert_eq!(a.get_pixel(x, y), b.get_pixel(x, y), "islands and the dilated ring are unchanged at ({x},{y})");
        }
        assert_eq!(a.get_pixel(40, y)[0], 0, "plain dilation leaves the rest empty");
    }

    // Near the island the fill favours its row colour and blends farther away
    let v = |x, y| f32::from(b.get_pixel(x, y)[0]) / 65535.0;
    assert!(v(19, 2) < 0.35 && v(19, 61) > 0.65, "{} {}", v(19, 2), v(19, 61));
    for y in 0..64 {
        for x in 18..64 {
            assert!((0.19..=0.81).contains(&v(x, y)), "({x},{y}) {}", v(x, y));
        }
    }
}
