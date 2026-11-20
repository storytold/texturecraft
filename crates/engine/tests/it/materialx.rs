//! `file.exportTextures` with `materialX: true`: a `.mtlx` OpenPBR material next to the textures.

use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const OPENPBR: &str = "OpenPBR (separate maps)";

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

fn export(s: &mut Session, dir: &std::path::Path, preset: &str, mtlx: bool) -> Value {
    execute(s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": preset, "materialX": mtlx})).unwrap()
}

fn files(r: &Value) -> Vec<String> {
    r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect()
}

#[test]
fn writes_a_material_that_names_the_exported_textures() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("ball");
    let r = export(&mut s, &dir, OPENPBR, true);
    let all = files(&r);
    let mtlx: Vec<&String> = all.iter().filter(|f| f.ends_with(".mtlx")).collect();
    assert_eq!(mtlx.len(), s.set_count(), "one material per texture set: {all:?}");
    for path in mtlx {
        let set = path.rsplit(['/', '\\']).next().unwrap().trim_end_matches(".mtlx").to_string();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("<open_pbr_surface") && text.contains("<surfacematerial"), "{text}");
        // Every texture the document points at was written beside it, and belongs to its set.
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
fn off_by_default_and_warns_without_openpbr_maps() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("off");
    assert!(!files(&export(&mut s, &dir, OPENPBR, false)).iter().any(|f| f.ends_with(".mtlx")));
    let dir = temp("other");
    let r = export(&mut s, &dir, "Document channels + Normal + AO", true);
    assert!(!files(&r).iter().any(|f| f.ends_with(".mtlx")));
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("OpenPBR")), "{r}");
}

#[test]
fn float_colour_maps_are_tagged_linear() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("exr");
    let r = execute(&mut s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "format": "exr", "materialX": true})).unwrap();
    let mtlx = files(&r).into_iter().find(|f| f.ends_with(".mtlx")).unwrap();
    let text = std::fs::read_to_string(mtlx).unwrap();
    assert!(text.contains("_base_color.exr\" colorspace=\"lin_rec709\"") && !text.contains("srgb_texture"), "{text}");
}

#[test]
fn colour_maps_are_tagged_with_the_export_colour_space() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    for (space, tag, warns) in [("ACEScg", "acescg", false), ("ACES2065-1", "lin_ap0_scene", true), ("sRGB", "srgb_texture", false)] {
        let dir = temp(tag);
        let r = execute(
            &mut s,
            "file.exportTextures",
            &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "format": "exr", "colorSpace": space, "materialX": true}),
        )
        .unwrap();
        let mtlx = files(&r).into_iter().find(|f| f.ends_with(".mtlx")).unwrap();
        let text = std::fs::read_to_string(mtlx).unwrap();
        assert!(text.contains(&format!("_base_color.exr\" colorspace=\"{tag}\"")), "{space}: {text}");
        let warned = r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("ACES2065-1"));
        assert_eq!(warned, warns, "{space}: {r}");
    }
}

#[test]
fn pgm_colour_maps_are_tagged_with_the_written_luminance_space() {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    for (space, tag) in [(None, "srgb_texture"), (Some("ACEScg"), "lin_rec709"), (Some("ACES2065-1"), "lin_rec709")] {
        let directory = temp(tag);
        let result = execute(
            &mut session,
            "file.exportTextures",
            &json!({"dir": directory.display().to_string(), "preset": OPENPBR, "format": "pgm", "colorSpace": space, "materialX": true}),
        )
        .unwrap();

        let material = files(&result).into_iter().find(|path| path.ends_with(".mtlx")).unwrap();
        let text = std::fs::read_to_string(material).unwrap();
        assert!(text.contains(&format!("_base_color.pgm\" colorspace=\"{tag}\"")), "{space:?}: {text}");
        assert!(!result["warnings"].to_string().contains("lin_ap0_scene"), "{result}");
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
        assert!(material.contains(&format!("_base_color.tif\" colorspace=\"{tag}\"")), "{material}");
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
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    assert!(s.set_count() >= 2);
    execute(&mut s, "textureSet.rename", &json!({"ts": 0, "name": "a b"})).unwrap();
    execute(&mut s, "textureSet.rename", &json!({"ts": 1, "name": "A_B"})).unwrap();
    let dir = temp("clash");
    let r = execute(&mut s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "ts": "all", "materialX": true})).unwrap();
    assert_eq!(files(&r).iter().filter(|f| f.ends_with(".mtlx")).count(), s.set_count() - 1, "{r}");
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("\"A_B\"")), "{r}");
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
    let mut s = Session::new_project("t", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    let dir = temp("udim");
    let r = export(&mut s, &dir, OPENPBR, true);
    let all = files(&r);
    let mtlx = all.iter().find(|f| f.ends_with(".mtlx")).unwrap_or_else(|| panic!("no .mtlx in {all:?}"));
    let text = std::fs::read_to_string(mtlx).unwrap();
    assert!(text.contains(".&lt;UDIM&gt;.png"), "{text}");
    assert!(!text.contains(".1001.") && !text.contains(".1002."), "{text}");
    assert!(all.iter().any(|f| f.ends_with(".1001.png")) && all.iter().any(|f| f.ends_with(".1002.png")));
}

#[test]
fn the_udim_placeholder_replaces_the_tile_number_not_a_look_alike_in_the_name() {
    let mut s = Session::new_project("t", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    execute(&mut s, "textureSet.rename", &json!({"ts": 0, "name": "cloth.1001.body"})).unwrap();
    let dir = temp("lookalike");
    let r = export(&mut s, &dir, OPENPBR, true);
    let all = files(&r);
    let mtlx = all.iter().find(|f| f.ends_with(".mtlx")).unwrap_or_else(|| panic!("no .mtlx in {all:?}"));
    let text = std::fs::read_to_string(mtlx).unwrap();
    assert!(text.contains("cloth.1001.body"), "the set's own name stays as it is: {text}");
    assert!(text.contains("&lt;UDIM&gt;"), "{text}");
    // Every texture the document names exists for the first tile.
    for part in text.split("name=\"file\" type=\"filename\" value=\"").skip(1) {
        let file = part.split('"').next().unwrap().replace("&lt;UDIM&gt;", "1001");
        assert!(dir.join(&file).is_file(), "{file} is not in {all:?}");
    }
}

#[test]
fn the_directx_warning_comes_once_and_only_with_a_normal_map() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    let dir = temp("dx");
    let r = execute(&mut s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": OPENPBR, "materialX": true, "normalFormat": "directX"}))
        .unwrap();
    let warnings: Vec<&str> = r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).filter(|w| w.contains("DirectX")).collect();
    assert_eq!(warnings.len(), 1, "{r}");
    let dir = temp("gl");
    let r = export(&mut s, &dir, OPENPBR, true);
    assert!(!r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("DirectX")), "{r}");
}
