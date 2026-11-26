//! `file.exportTextures` with `materialX: true`: a `.mtlx` OpenPBR material next to the textures.

use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const OPENPBR: &str = "OpenPBR (separate maps)";
const DEFAULT: &str = "Document channels + Normal + AO (No Alpha)";

/// A scratch folder that is removed when it goes out of scope, so a failing test leaves nothing behind.
struct Scratch(std::path::PathBuf);

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp(tag: &str) -> Scratch {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tc_mtlx_{tag}_{}_{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Scratch(dir)
}

fn export(session: &mut Session, dir: &std::path::Path, preset: &str, mtlx: bool) -> Value {
    execute(session, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": preset, "materialX": mtlx})).unwrap()
}

fn files(result: &Value) -> Vec<String> {
    result["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect()
}

#[test]
fn writes_a_material_that_names_the_exported_textures() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("ball");

    let result = export(&mut session, &dir, OPENPBR, true);

    let all = files(&result);
    let mtlx: Vec<&String> = all.iter().filter(|f| f.ends_with(".mtlx")).collect();
    assert_eq!(mtlx.len(), session.set_count(), "one material per texture set: {all:?}");
    for path in mtlx {
        let set = path.rsplit(['/', '\\']).next().unwrap().trim_end_matches(".mtlx").to_string();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("<open_pbr_surface") && text.contains("<surfacematerial"), "{text}");

        // Check that each reference resolves to a texture of this set
        let mut refs = 0;
        for part in text.split("name=\"file\" type=\"filename\" value=\"").skip(1) {
            let file = part.split('"').next().unwrap();
            assert!(dir.join(file).is_file(), "{file} is not in {all:?}");
            assert!(file.starts_with(&set), "{file} is not a texture of {set}");
            refs += 1;
        }
        assert!(refs >= 3, "base colour, roughness and normal at least: {text}");
        assert!(text.contains("name=\"base_color\"") && text.contains("name=\"geometry_normal\""), "{text}");
    }
}

#[test]
fn off_by_default() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("off");
    assert!(!files(&export(&mut session, &dir, OPENPBR, false)).iter().any(|f| f.ends_with(".mtlx")));

    let dir = temp("off-other");
    assert!(!files(&export(&mut session, &dir, DEFAULT, false)).iter().any(|f| f.ends_with(".mtlx") || f.ends_with("_base_color.png")));
}

#[test]
fn warns_when_a_template_with_openpbr_names_has_no_channels_to_write() {
    use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef};

    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    for set in &mut session.doc.project.texture_sets {
        set.channels.clear();
    }

    let preset = ExportPreset {
        name: "coat".into(),
        maps: vec![MapSpec {
            file: "$mesh_$textureSet_coat_weight".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![(Dest::L, SourceRef::Channel { channel: texturecraft_engine::core::Channel::CoatWeight, component: 0 })],
            inverted: Vec::new(),
        }],
    };
    let directory = temp("no-maps");

    let result = execute(&mut session, "file.exportTextures", &json!({"dir": directory.display().to_string(), "preset": preset, "materialX": true})).unwrap();

    assert!(files(&result).is_empty());
    assert!(warned(&result, "has no OpenPBR maps"), "{result}");
}

/// The `file` values a `.mtlx` refers to
fn referenced(text: &str) -> Vec<String> {
    text.split("name=\"file\" type=\"filename\" value=\"").skip(1).map(|part| part.split('"').next().unwrap().to_string()).collect()
}

fn names(result: &Value) -> Vec<String> {
    files(result).iter().map(|f| f.rsplit(['/', '\\']).next().unwrap().to_string()).collect()
}

#[test]
fn any_template_gets_the_openpbr_maps_beside_its_own_files() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let plain = temp("plain");
    let own = names(&export(&mut session, &plain, DEFAULT, false));
    let dir = temp("default-template");

    let result = export(&mut session, &dir, DEFAULT, true);

    let all = names(&result);
    for file in &own {
        assert!(all.contains(file), "the template's own file {file} is still written: {all:?}");
    }

    let mtlx: Vec<String> = files(&result).into_iter().filter(|f| f.ends_with(".mtlx")).collect();
    assert_eq!(mtlx.len(), session.set_count(), "{all:?}");
    for path in &mtlx {
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("<open_pbr_surface") && text.contains("name=\"base_color\"") && text.contains("name=\"geometry_normal\""), "{text}");
        let refs = referenced(&text);
        assert!(refs.len() >= 3, "{text}");
        for file in refs {
            assert!(dir.join(&file).is_file(), "{file} is not in {all:?}");
            assert!(!own.contains(&file), "{file} is a file of the chosen template");
        }
    }

    // Every file is written once
    let mut sorted = all.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), all.len(), "{all:?}");
}

#[test]
fn the_openpbr_template_writes_no_second_set_of_maps() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let without = temp("openpbr-without");
    let plain = names(&export(&mut session, &without, OPENPBR, false));

    let dir = temp("openpbr-with");
    let mut all = names(&export(&mut session, &dir, OPENPBR, true));

    all.retain(|f| !f.ends_with(".mtlx"));
    let mut plain = plain;
    plain.sort();
    all.sort();
    assert_eq!(all, plain);
}

#[test]
fn per_map_overrides_of_the_template_do_not_reach_the_openpbr_maps() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("override");

    // Use overrides that would affect different maps if applied to the OpenPBR template
    let result = execute(
        &mut session,
        "file.exportTextures",
        &json!({"dir": dir.display().to_string(), "preset": DEFAULT, "materialX": true, "maps": [{"map": 0, "enabled": false}, {"map": 19, "format": "tiff", "bitDepth": 16}]}),
    )
    .unwrap();

    let all = names(&result);
    assert!(!all.iter().any(|f| f.ends_with("_baseColor.png")), "the template's base colour was left out: {all:?}");
    assert!(all.iter().any(|f| f.ends_with("_base_color.png")), "the material's base colour is still written: {all:?}");
    assert!(all.iter().any(|f| f.ends_with("_geometry_normal.png")), "{all:?}");
    assert!(all.iter().any(|f| f.ends_with("_normal.tif")), "the override applies to the template's own normal map: {all:?}");
    assert!(!all.iter().any(|f| f.ends_with(".tif") && f.contains("_geometry_")), "{all:?}");
}

#[test]
fn a_file_type_a_material_cannot_use_falls_back_to_png() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    for (format, bit_depth, ext) in [("psd", 16, "psd"), ("pgm", 8, "pgm"), ("jng", 8, "jng"), ("tiff", 16, "tif")] {
        let dir = temp(format);

        let result = execute(
            &mut session,
            "file.exportTextures",
            &json!({"dir": dir.display().to_string(), "preset": DEFAULT, "format": format, "bitDepth": bit_depth, "materialX": true}),
        )
        .unwrap();

        let all = names(&result);
        assert!(all.iter().any(|f| f.ends_with(&format!("_baseColor.{ext}"))), "{format}: the template's files keep the file type: {all:?}");
        let mtlx = files(&result).into_iter().find(|f| f.ends_with(".mtlx")).unwrap_or_else(|| panic!("{format}: no .mtlx in {all:?}"));
        let text = std::fs::read_to_string(mtlx).unwrap();
        let wanted = if format == "tiff" { "tif" } else { "png" };
        assert!(text.contains(&format!("_base_color.{wanted}\"")), "{format}: {text}");
        for file in referenced(&text) {
            assert!(dir.join(&file).is_file(), "{format}: {file} is not in {all:?}");
        }
    }
}

#[test]
fn the_openpbr_maps_follow_the_size_and_colour_space_of_the_export() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("size");

    let result = execute(
        &mut session,
        "file.exportTextures",
        &json!({"dir": dir.display().to_string(), "preset": DEFAULT, "size": 256, "colorSpace": "ACEScg", "materialX": true}),
    )
    .unwrap();

    let png = files(&result).into_iter().find(|f| f.ends_with("_base_color.png")).unwrap();
    assert_eq!(image::image_dimensions(png).unwrap(), (256, 256));
    let text = std::fs::read_to_string(files(&result).into_iter().find(|f| f.ends_with(".mtlx")).unwrap()).unwrap();
    assert!(text.contains("_base_color.png\" colorspace=\"acescg\""), "{text}");

    // Only colour inputs carry a colour space, and the data maps are still referenced
    let roughness = referenced(&text).into_iter().find(|f| f.ends_with("_specular_roughness.png")).unwrap_or_else(|| panic!("{text}"));
    assert!(dir.join(&roughness).is_file());
    for part in text.split("name=\"file\" type=\"filename\" value=\"").skip(1) {
        let (file, rest) = part.split_once('"').unwrap();
        if rest.starts_with(" colorspace=\"acescg\"") {
            assert!(["_base_color.", "_fuzz_color.", "_emission_color."].iter().any(|input| file.contains(input)), "{file}");
        }
    }
}

#[test]
fn a_file_of_the_template_is_never_replaced_by_an_openpbr_map() {
    use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef};

    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "a"})).unwrap();

    // Set 1's file from the template has the name set 0's OpenPBR base colour would get
    execute(&mut session, "textureSet.rename", &json!({"ts": 1, "name": "a_base_color"})).unwrap();
    let source = |component| SourceRef::Channel { channel: texturecraft_engine::core::Channel::BaseColor, component };
    let preset = ExportPreset {
        name: "base colour only".into(),
        maps: vec![MapSpec {
            file: "$mesh_$textureSet".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![(Dest::R, source(0)), (Dest::G, source(1)), (Dest::B, source(2))],
            inverted: Vec::new(),
        }],
    };
    let params = |dir: &std::path::Path, mtlx: bool| json!({"dir": dir.display().to_string(), "preset": preset, "ts": [0, 1], "materialX": mtlx});
    let plain = temp("clash-plain");
    execute(&mut session, "file.exportTextures", &params(&plain, false)).unwrap();

    let dir = temp("clash");
    let result = execute(&mut session, "file.exportTextures", &params(&dir, true)).unwrap();

    assert_eq!(std::fs::read(dir.join("material_ball_a_base_color.png")).unwrap(), std::fs::read(plain.join("material_ball_a_base_color.png")).unwrap());
    assert!(
        result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("material_ball_a_base_color.png\" was not written")),
        "{result}"
    );
    let first = std::fs::read_to_string(dir.join("material_ball_a.mtlx")).unwrap();
    assert!(!referenced(&first).contains(&"material_ball_a_base_color.png".to_string()), "{first}");
}

/// A template with one map, the base colour, named `$mesh_$textureSet` (no input suffix)
fn base_colour_template() -> texturecraft_engine::formats::ExportPreset {
    use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef};

    let source = |component| SourceRef::Channel { channel: texturecraft_engine::core::Channel::BaseColor, component };
    ExportPreset {
        name: "base colour only".into(),
        maps: vec![MapSpec {
            file: "$mesh_$textureSet".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![(Dest::R, source(0)), (Dest::G, source(1)), (Dest::B, source(2))],
            inverted: Vec::new(),
        }],
    }
}

/// One `file.exportTextures` command of texture set `ts`, as the export window sends them: the
/// names of the files the earlier commands wrote are passed as `reservedNames`
fn export_set(session: &mut Session, dir: &std::path::Path, ts: usize, earlier: &[String]) -> Value {
    let params = json!({"dir": dir.display().to_string(), "preset": base_colour_template(), "ts": ts, "materialX": true, "reservedNames": earlier});
    execute(session, "file.exportTextures", &params).unwrap()
}

fn warned(result: &Value, text: &str) -> bool {
    result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains(text))
}

#[test]
fn an_openpbr_map_never_replaces_a_file_an_earlier_command_of_the_export_wrote() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();

    // Set 0's own file is named like the base colour map set 1 gets from the OpenPBR template
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "a_base_color"})).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 1, "name": "a"})).unwrap();
    let dir = temp("later-extra");

    let first = export_set(&mut session, &dir, 0, &[]);
    let kept = std::fs::read(dir.join("material_ball_a_base_color.png")).unwrap();
    let second = export_set(&mut session, &dir, 1, &names(&first));

    assert_eq!(std::fs::read(dir.join("material_ball_a_base_color.png")).unwrap(), kept, "the earlier file is untouched");
    assert!(warned(&second, "\"material_ball_a_base_color.png\" was not written: a file of the chosen template or of an earlier command"), "{second}");
    let material = std::fs::read_to_string(dir.join("material_ball_a.mtlx")).unwrap();
    assert!(!referenced(&material).contains(&"material_ball_a_base_color.png".to_string()), "{material}");
    for file in referenced(&material) {
        assert!(dir.join(&file).is_file(), "{file}");
    }
}

#[test]
fn a_file_of_the_template_never_replaces_an_openpbr_map_an_earlier_command_wrote() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();

    // Set 0's OpenPBR base colour has the name set 1's own file gets from the template
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "a"})).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 1, "name": "a_base_color"})).unwrap();
    let dir = temp("later-own");

    let first = export_set(&mut session, &dir, 0, &[]);
    let kept = std::fs::read(dir.join("material_ball_a_base_color.png")).unwrap();
    let second = export_set(&mut session, &dir, 1, &names(&first));

    assert_eq!(std::fs::read(dir.join("material_ball_a_base_color.png")).unwrap(), kept, "the earlier file is untouched");
    assert!(warned(&second, "\"material_ball_a_base_color.png\" was not written"), "{second}");

    // The earlier material still points at the file it was written with
    let earlier = std::fs::read_to_string(dir.join("material_ball_a.mtlx")).unwrap();
    assert!(referenced(&earlier).contains(&"material_ball_a_base_color.png".to_string()), "{earlier}");
    let later = std::fs::read_to_string(dir.join("material_ball_a_base_color.mtlx")).unwrap();
    assert!(!referenced(&later).contains(&"material_ball_a_base_color.png".to_string()), "{later}");
}

#[test]
fn a_material_is_not_written_over_a_file_an_earlier_command_wrote() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "a"})).unwrap();
    let dir = temp("mtlx-name");

    let result = export_set(&mut session, &dir, 0, &["MATERIAL_BALL_A.MTLX".to_string()]);

    assert!(!files(&result).iter().any(|f| f.ends_with(".mtlx")), "{result}");
    assert!(warned(&result, "exports as \"material_ball_a.mtlx\" like an earlier file of this export"), "{result}");

    let result = export_set(&mut session, &dir, 0, &["unrelated.mtlx".to_string()]);
    assert!(files(&result).iter().any(|f| f.ends_with("material_ball_a.mtlx")), "{result}");
}

#[test]
fn reserved_names_must_be_a_list_of_names() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("bad-reserved");

    for bad in [json!("a.png"), json!([1])] {
        let params = json!({"dir": dir.display().to_string(), "preset": OPENPBR, "reservedNames": bad});
        assert!(execute(&mut session, "file.exportTextures", &params).is_err());
    }

    assert!(!dir.exists(), "a bad parameter creates no folder");
}

#[test]
fn float_colour_maps_are_tagged_linear() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("exr");

    let result =
        execute(&mut session, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "format": "exr", "materialX": true}))
            .unwrap();

    let mtlx = files(&result).into_iter().find(|f| f.ends_with(".mtlx")).unwrap();
    let text = std::fs::read_to_string(mtlx).unwrap();
    assert!(text.contains("_base_color.exr\" colorspace=\"lin_rec709\"") && !text.contains("srgb_texture"), "{text}");
}

#[test]
fn colour_maps_are_tagged_with_the_export_colour_space() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    for (space, tag, warns) in [("ACEScg", "acescg", false), ("ACES2065-1", "lin_ap0_scene", true), ("sRGB", "srgb_texture", false)] {
        let dir = temp(tag);

        let result = execute(
            &mut session,
            "file.exportTextures",
            &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "format": "exr", "colorSpace": space, "materialX": true}),
        )
        .unwrap();

        let mtlx = files(&result).into_iter().find(|f| f.ends_with(".mtlx")).unwrap();
        let text = std::fs::read_to_string(mtlx).unwrap();
        assert!(text.contains(&format!("_base_color.exr\" colorspace=\"{tag}\"")), "{space}: {text}");
        let warned = result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("ACES2065-1"));
        assert_eq!(warned, warns, "{space}: {result}");
    }
}

#[test]
fn materialx_uses_rgb_fallbacks_beside_selected_pgm_colour_maps() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    for (space, tag) in [(None, "srgb_texture"), (Some("ACEScg"), "acescg"), (Some("ACES2065-1"), "lin_ap0_scene")] {
        let directory = temp(tag);

        let result = execute(
            &mut session,
            "file.exportTextures",
            &json!({"dir": directory.display().to_string(), "preset": OPENPBR, "format": "pgm", "colorSpace": space, "materialX": true}),
        )
        .unwrap();

        let paths = files(&result);
        let selected = paths.iter().find(|path| path.ends_with("_base_color.pgm")).unwrap();
        let material = paths.iter().find(|path| path.ends_with(".mtlx")).unwrap();
        let text = std::fs::read_to_string(material).unwrap();
        assert!(text.contains(&format!("_base_color.png\" colorspace=\"{tag}\"")), "{space:?}: {text}");
        assert!(std::fs::read(selected).unwrap().starts_with(b"P5"), "the selected output is still binary PGM");
        assert!(!text.contains(".pgm"), "{text}");
    }
}

#[test]
fn explicit_srgb_float_tiff_tags_match_gray_and_rgb_samples() {
    use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef};
    use texturecraft_engine::{core::Channel, texel::linear_to_srgb};

    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    execute(&mut session, "layer.fill.set", &json!({"channel": "baseColor", "color": [0.25, 0.5, 0.75]})).unwrap();
    let source = |component| SourceRef::Channel { channel: Channel::BaseColor, component };
    let mut linear_sample = None;
    for (channels, tag) in
        [(vec![(Dest::L, source(0))], "lin_rec709"), (vec![(Dest::R, source(0)), (Dest::G, source(1)), (Dest::B, source(2))], "srgb_texture")]
    {
        let directory = temp(tag);
        let preset = ExportPreset {
            name: "TIFF base colour".into(),
            maps: vec![MapSpec { file: "$mesh_$textureSet_base_color".into(), format: FileFormat::Tiff, bit_depth: 32, channels, inverted: Vec::new() }],
        };

        let result = execute(
            &mut session,
            "file.exportTextures",
            &json!({"dir": directory.display().to_string(), "preset": preset, "ts": 0, "colorSpace": "sRGB", "materialX": true}),
        )
        .unwrap();

        let paths = files(&result);
        let texture = paths.iter().find(|file| file.ends_with(".tif")).unwrap();
        let material = std::fs::read_to_string(paths.iter().find(|file| file.ends_with(".mtlx")).unwrap()).unwrap();
        let warned = result["warnings"].to_string().contains("written in linear Rec.709");
        let samples = image::open(texture).unwrap().to_rgb32f();
        let expected_name = if tag == "lin_rec709" { "_base_color_mtlx.tif" } else { "_base_color.tif" };
        assert!(material.contains(&format!("{expected_name}\" colorspace=\"srgb_texture\"")), "{material}");
        let reference = referenced(&material).into_iter().find(|name| name.ends_with(expected_name)).unwrap();
        assert_eq!(image::open(directory.join(reference)).unwrap().color(), image::ColorType::Rgb32F);
        assert_eq!(warned, tag == "lin_rec709", "{result}");
        if let Some(expected) = linear_sample {
            let (x, y, red) = expected;
            assert!((samples.get_pixel(x, y)[0] - linear_to_srgb(red)).abs() < 1e-5);
        } else {
            let (x, y, pixel) = samples.enumerate_pixels().find(|(_, _, pixel)| (0.01..0.99).contains(&pixel[0])).unwrap();
            assert_eq!(pixel.0, [pixel[0]; 3]);
            linear_sample = Some((x, y, pixel[0]));
        }
    }
}

#[test]
fn sets_with_the_same_file_name_warn_instead_of_overwriting() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    assert!(session.set_count() >= 2);
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "a b"})).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 1, "name": "A_B"})).unwrap();
    let dir = temp("clash");

    let result =
        execute(&mut session, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "ts": "all", "materialX": true})).unwrap();

    assert_eq!(files(&result).iter().filter(|f| f.ends_with(".mtlx")).count(), session.set_count() - 1, "{result}");
    assert!(result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("\"A_B\"")), "{result}");
}

#[test]
fn a_material_never_points_at_a_texture_kept_for_another_set() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();

    // The first set's name expands to the same base-colour filename as the second
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "$channel"})).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 1, "name": "baseColor"})).unwrap();
    let directory = temp("shared-map");

    let result = export(&mut session, &directory, OPENPBR, true);

    let materials: Vec<_> = files(&result).into_iter().filter(|path| path.ends_with(".mtlx")).collect();
    assert!(materials.len() >= 2, "{result}");
    assert!(std::fs::read_to_string(&materials[0]).unwrap().contains("material_ball_baseColor_base_color.png"));
    for material in materials.iter().skip(1) {
        assert!(!std::fs::read_to_string(material).unwrap().contains("material_ball_baseColor_base_color.png"), "{material}");
    }
}

#[test]
fn udim_sets_use_the_udim_token() {
    let mut session = Session::new_project("t", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    let dir = temp("udim");

    let result = export(&mut session, &dir, OPENPBR, true);

    let all = files(&result);
    let mtlx = all.iter().find(|f| f.ends_with(".mtlx")).unwrap_or_else(|| panic!("no .mtlx in {all:?}"));
    let text = std::fs::read_to_string(mtlx).unwrap();
    assert!(text.contains(".&lt;UDIM&gt;.png"), "{text}");
    assert!(!text.contains(".1001.") && !text.contains(".1002."), "{text}");
    assert!(all.iter().any(|f| f.ends_with(".1001.png")) && all.iter().any(|f| f.ends_with(".1002.png")));
}

#[test]
fn the_udim_placeholder_replaces_the_tile_number_not_a_look_alike_in_the_name() {
    let mut session = Session::new_project("t", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "cloth.1001.body"})).unwrap();
    let dir = temp("lookalike");

    let result = export(&mut session, &dir, OPENPBR, true);

    let all = files(&result);
    let mtlx = all.iter().find(|f| f.ends_with(".mtlx")).unwrap_or_else(|| panic!("no .mtlx in {all:?}"));
    let text = std::fs::read_to_string(mtlx).unwrap();
    assert!(text.contains("cloth.1001.body"), "the set's own name stays as it is: {text}");
    assert!(text.contains("&lt;UDIM&gt;"), "{text}");

    // Verify the token for the first tile
    for part in text.split("name=\"file\" type=\"filename\" value=\"").skip(1) {
        let file = part.split('"').next().unwrap().replace("&lt;UDIM&gt;", "1001");
        assert!(dir.join(&file).is_file(), "{file} is not in {all:?}");
    }
}

#[test]
fn the_directx_warning_comes_once_and_only_with_a_normal_map() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("dx");

    let result = execute(
        &mut session,
        "file.exportTextures",
        &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "materialX": true, "normalFormat": "directX"}),
    )
    .unwrap();

    let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).filter(|w| w.contains("DirectX")).collect();
    assert_eq!(warnings.len(), 1, "{result}");

    let dir = temp("gl");
    let result = export(&mut session, &dir, OPENPBR, true);
    assert!(!result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("DirectX")), "{result}");
}

/// A tangent-space normal image of one colour as PNG bytes
fn normal_png(rgb: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(4, 4, image::Rgb(rgb));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

fn green_at_centre(path: &str) -> u8 {
    let image = image::open(path).unwrap().to_rgb8();
    image.get_pixel(image.width() / 2, image.height() / 2)[1]
}

#[test]
fn the_materials_normal_map_is_opengl_whatever_the_export_asks_for() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();

    // Use a tilted normal so the green values separate by convention
    let image = session.import_resource_bytes("n.png", normal_png([128, 64, 255])).unwrap();
    execute(&mut session, "textureSet.meshMap.set", &json!({"map": "normal", "image": image})).unwrap();
    let dir = temp("normal-format");

    let result = execute(
        &mut session,
        "file.exportTextures",
        &json!({"dir": dir.display().to_string(), "preset": DEFAULT, "ts": 0, "normalFormat": "directX", "materialX": true}),
    )
    .unwrap();

    let all = files(&result);
    let own = all.iter().find(|f| f.ends_with("_normal.png")).unwrap();
    let material = all.iter().find(|f| f.ends_with("_geometry_normal.png")).unwrap();
    assert!(green_at_centre(own).abs_diff(184) <= 3, "the template's own normal map is DirectX: {}", green_at_centre(own));
    assert!(green_at_centre(material).abs_diff(71) <= 3, "the material's normal map is OpenGL: {}", green_at_centre(material));
    assert!(!result["warnings"].to_string().contains("DirectX"), "nothing to flip in the material: {result}");
}

#[test]
fn the_bit_depth_of_the_export_applies_to_the_openpbr_maps_unless_the_file_type_fell_back() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    for (extra, expected) in [(json!({}), image::ColorType::Rgb16), (json!({"format": "psd"}), image::ColorType::Rgb8)] {
        let dir = temp("depth");
        let mut params = json!({"dir": dir.display().to_string(), "preset": DEFAULT, "ts": 0, "bitDepth": 16, "materialX": true});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());

        let result = execute(&mut session, "file.exportTextures", &params).unwrap();

        let png = files(&result).into_iter().find(|f| f.ends_with("_base_color.png")).unwrap();
        assert_eq!(image::open(png).unwrap().color(), expected, "{params}");
    }
}

#[test]
fn a_set_whose_openpbr_maps_were_all_skipped_gets_no_material_and_a_warning() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    assert!(session.set_count() >= 2);
    execute(&mut session, "textureSet.rename", &json!({"ts": 0, "name": "a b"})).unwrap();
    execute(&mut session, "textureSet.rename", &json!({"ts": 1, "name": "A_B"})).unwrap();
    let dir = temp("all-skipped");

    let result =
        execute(&mut session, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": DEFAULT, "ts": "all", "materialX": true})).unwrap();

    assert_eq!(files(&result).iter().filter(|f| f.ends_with(".mtlx")).count(), session.set_count() - 1, "{result}");
    let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(warnings.contains(&"MaterialX: texture set \"A_B\" has no OpenPBR maps to refer to"), "{warnings:?}");
}

#[test]
fn openpbr_template_preserves_directx_export_and_references_an_opengl_normal() {
    let mut session = Session::new_project("material", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let image = session.import_resource_bytes("normal.png", normal_png([128, 64, 255])).unwrap();
    execute(&mut session, "textureSet.meshMap.set", &json!({"map": "normal", "image": image})).unwrap();
    let directory = temp("openpbr-directx");
    let params = json!({"dir": directory.display().to_string(), "preset": OPENPBR, "ts": 0, "normalFormat": "directX", "materialX": true});

    let result = execute(&mut session, "file.exportTextures", &params).unwrap();

    let all = files(&result);
    let selected = all.iter().find(|path| path.ends_with("_geometry_normal.png")).unwrap();
    let document = std::fs::read_to_string(all.iter().find(|path| path.ends_with(".mtlx")).unwrap()).unwrap();
    let reference = referenced(&document).into_iter().find(|name| name.contains("_geometry_normal")).unwrap();
    assert!(green_at_centre(selected) > 128, "the selected normal must retain the DirectX convention");
    assert!(green_at_centre(&directory.join(reference).to_string_lossy()) < 128, "the material must reference an OpenGL normal");
}

#[test]
fn an_openpbr_filename_does_not_turn_a_data_map_into_base_color() {
    use texturecraft_engine::formats::{Dest, ExportPreset, FileFormat, MapSpec, SourceRef};

    let mut session = Session::new_project("material", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let template = ExportPreset {
        name: "roughness with a custom filename".into(),
        maps: vec![MapSpec {
            file: "$mesh_$textureSet_base_color".into(),
            format: FileFormat::Png,
            bit_depth: 8,
            channels: vec![(Dest::L, SourceRef::Channel { channel: texturecraft_engine::core::Channel::Roughness, component: 0 })],
            inverted: Vec::new(),
        }],
    };
    let directory = temp("misleading-map");
    let params = json!({"dir": directory.display().to_string(), "preset": template, "ts": 0, "materialX": true});

    let result = execute(&mut session, "file.exportTextures", &params).unwrap();

    let all = files(&result);
    let selected = all.iter().find(|path| path.ends_with("_base_color.png")).unwrap();
    let document = std::fs::read_to_string(all.iter().find(|path| path.ends_with(".mtlx")).unwrap()).unwrap();
    let reference = referenced(&document).into_iter().find(|name| name.contains("_base_color")).unwrap();
    assert_eq!(image::open(selected).unwrap().color(), image::ColorType::L8);
    assert_eq!(image::open(directory.join(reference)).unwrap().color(), image::ColorType::Rgb8);
}

#[test]
fn a_material_udim_pattern_never_captures_a_tile_written_by_another_command() {
    let mut session = Session::new_project("material", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    let directory = temp("udim-collision");
    let plain = export(&mut session, &directory, OPENPBR, true);
    let material = files(&plain).into_iter().find(|path| path.ends_with(".mtlx")).unwrap();
    let document = std::fs::read_to_string(material).unwrap();
    let reference = referenced(&document).into_iter().find(|name| name.contains("_base_color")).unwrap();
    let other_tile = reference.replace("&lt;UDIM&gt;", "1003");
    let earlier_tile = directory.join(&other_tile);
    std::fs::copy(directory.join(reference.replace("&lt;UDIM&gt;", "1001")), &earlier_tile).unwrap();

    let result = execute(
        &mut session,
        "file.exportTextures",
        &json!({"dir": directory.display().to_string(), "preset": OPENPBR, "materialX": true, "reservedNames": [other_tile]}),
    )
    .unwrap();

    let material = files(&result).into_iter().find(|path| path.ends_with(".mtlx")).unwrap();
    let document = std::fs::read_to_string(material).unwrap();
    assert!(!referenced(&document).contains(&reference), "{document}");
    assert!(warned(&result, "also matches another texture set's files"), "{result}");

    std::fs::remove_file(&earlier_tile).unwrap();
    let mut conflicting_preset = texturecraft_engine::formats::builtin_preset(OPENPBR).unwrap();
    conflicting_preset.maps.truncate(1);
    conflicting_preset.maps[0].file = other_tile.trim_end_matches(".png").into();

    let mut plain_session = Session::new_project("other", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();

    let result = execute(
        &mut plain_session,
        "file.exportTextures",
        &json!({
            "dir": directory.display().to_string(),
            "preset": conflicting_preset,
            "reservedNames": plain["materialUdimPatterns"]
        }),
    )
    .unwrap();

    assert!(files(&result).is_empty(), "{result}");
    assert!(warned(&result, "an earlier MaterialX document uses that UDIM pattern"), "{result}");
    assert!(!directory.join(other_tile).exists());
}
