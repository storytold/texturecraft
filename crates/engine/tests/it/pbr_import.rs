//! `file.importPbrFolder` / `file.scanPbrFolder`: a folder, files or a zip of PBR maps becomes
//! one fill layer.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, Fill, LayerId, LayerKind, NormalFormat, Projection, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    Session::new_project("t", primitives::builtin("plane").unwrap(), "builtin:plane", None, 128).unwrap()
}

/// A session whose texture set also has an ambient occlusion channel (the default set has none).
fn session_with_ao() -> Session {
    let mut s = session();
    execute(&mut s, "textureSet.channel.add", &json!({"channel": "ambientOcclusion"})).unwrap();
    s
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tc_pbr_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn png(path: &Path, rgb: [u8; 3]) {
    image::RgbImage::from_pixel(4, 4, image::Rgb(rgb)).save(path).unwrap();
}

fn fill_of(s: &Session, layer: u64) -> Fill {
    match &s.doc.project.layer(LayerId(layer)).unwrap().kind {
        LayerKind::Fill(f) => f.clone(),
        other => panic!("not a fill: {other:?}"),
    }
}

fn import(s: &mut Session, dir: &Path, extra: Value) -> Result<Value, texturecraft_engine::EngineError> {
    let mut p = json!({ "path": dir.display().to_string() });
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    execute(s, "file.importPbrFolder", &p)
}

fn image_name(f: &Fill, c: Channel) -> String {
    match f.sources.get(&c) {
        Some(Source::Image { name, .. }) => name.clone(),
        other => panic!("{c:?} is {other:?}"),
    }
}

#[test]
fn a_folder_becomes_one_fill_layer_in_one_undo_step() {
    let dir = folder("ok");
    for (name, rgb) in [
        ("Brick_Albedo.png", [200, 40, 40]),
        ("Brick_Roughness.png", [128, 128, 128]),
        ("Brick_Metallic.png", [0, 0, 0]),
        ("Brick_Normal_DX.png", [128, 51, 255]),
        ("Brick_Height.png", [90, 90, 90]),
        ("Brick_AO.png", [255, 255, 255]),
        ("Brick_Preview.png", [1, 2, 3]),
    ] {
        png(&dir.join(name), rgb);
    }
    std::fs::write(dir.join("readme.txt"), "hi").unwrap();
    let mut s = session_with_ao();
    let res = s.doc.resources.len();
    let r = import(&mut s, &dir, json!({})).unwrap();
    let layer = r["layer"].as_u64().unwrap();
    let fill = fill_of(&s, layer);
    for c in [Channel::BaseColor, Channel::Roughness, Channel::Metallic, Channel::Normal, Channel::Height, Channel::AmbientOcclusion] {
        assert!(matches!(fill.sources.get(&c), Some(Source::Image { .. })), "{c:?}: {:?}", fill.sources);
    }
    // The convention comes from `_DX` in the name; the preview and the text file are ignored.
    assert!(matches!(fill.sources.get(&Channel::Normal), Some(Source::Image { normal_format: NormalFormat::DirectX, .. })));
    assert_eq!(r["normalFormat"], "directX");
    assert_eq!(r["maps"]["baseColor"], "Brick_Albedo.png");
    let ignored = r["ignored"].to_string();
    assert!(ignored.contains("Brick_Preview.png") && ignored.contains("preview image") && ignored.contains("readme.txt"), "{ignored}");
    assert!(!s.doc.resources.keys().any(|k| k.to_lowercase().contains("preview")));
    assert_eq!(s.doc.resources.len(), res + 6);
    assert!(matches!(fill.projection, Projection::Uv { scale, .. } if scale == [1.0, 1.0]));
    let layer_name = s.doc.project.layer(LayerId(layer)).unwrap().name.clone();
    assert!(layer_name.starts_with("tc pbr ok"), "named after the folder: {layer_name}");
    assert_eq!(r["name"], "tc pbr ok ".to_string() + &std::process::id().to_string());
    // Every decoded image is ready for the viewport.
    for n in s.doc.resources.keys() {
        assert!(s.images.contains_key(n), "{n}");
    }
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.doc.resources.len(), res);
    assert!(s.doc.project.layer(LayerId(layer)).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directx_normal_map_is_flipped_to_opengl_where_it_is_sampled() {
    // Green 0.2 in the file: a DirectX map reads 0.8 as OpenGL, an OpenGL map stays 0.2.
    let green = |files: &[&str], extra: Value| -> f32 {
        let dir = folder("flip");
        for f in files {
            png(&dir.join(f), [128, 51, 255]);
        }
        let mut s = session();
        import(&mut s, &dir, extra).unwrap();
        s.update();
        let v = s.caches[0].composite.value(Channel::Normal, 64, 64);
        let _ = std::fs::remove_dir_all(&dir);
        v[1]
    };
    let (dx, gl) = (green(&["wall_normal_dx.png"], json!({})), green(&["wall_normal_gl.png"], json!({})));
    assert!((dx - 0.8).abs() < 0.02, "DirectX name flips: {dx}");
    assert!((gl - 0.2).abs() < 0.02, "OpenGL name does not: {gl}");
    // No convention in the name: OpenGL by default, and the toggle decides.
    assert!((green(&["wall_normal.png"], json!({})) - 0.2).abs() < 0.02);
    assert!((green(&["wall_normal.png"], json!({"normalFormat": "directX"})) - 0.8).abs() < 0.02);
    // The toggle forces the convention even against the name.
    assert!((green(&["wall_normal_dx.png"], json!({"normalFormat": "openGl"})) - 0.2).abs() < 0.02);
    assert!((green(&["wall_normal_dx.png"], json!({"normalFormat": "auto"})) - 0.8).abs() < 0.02);
}

#[test]
fn a_packed_orm_image_is_split_into_three_maps() {
    let dir = folder("orm");
    // Occlusion 0.2 (R), roughness 0.5 (G), metallic 0.9 (B).
    png(&dir.join("floor_ORM.png"), [51, 128, 230]);
    png(&dir.join("floor_BaseColor.png"), [10, 20, 30]);
    let mut s = session_with_ao();
    let r = import(&mut s, &dir, json!({})).unwrap();
    let fill = fill_of(&s, r["layer"].as_u64().unwrap());
    let names: Vec<String> = [Channel::AmbientOcclusion, Channel::Roughness, Channel::Metallic].iter().map(|c| image_name(&fill, *c)).collect();
    assert_eq!(names.iter().collect::<std::collections::BTreeSet<_>>().len(), 3, "three separate resources: {names:?}");
    s.update();
    let at = |c| s.caches[0].composite.value(c, 64, 64)[0];
    assert!((at(Channel::AmbientOcclusion) - 0.2).abs() < 0.01, "{}", at(Channel::AmbientOcclusion));
    assert!((at(Channel::Roughness) - 0.5).abs() < 0.01);
    assert!((at(Channel::Metallic) - 0.9).abs() < 0.01);
    // The saved resources are real images that decode to the same values after a reload.
    for n in &names {
        let d = texturecraft_engine::formats::decode_image(&s.doc.resources[n]).unwrap();
        assert_eq!((d.width, d.height), (4, 4));
    }
    // A set without an occlusion channel skips it with a warning and keeps the rest.
    let mut plain = session();
    let r = import(&mut plain, &dir, json!({})).unwrap();
    assert!(r["warnings"].to_string().contains("ambientOcclusion was skipped"), "{r}");
    assert!(r["channels"].as_array().unwrap().contains(&json!("metallic")));
    // Separate maps win over the packed image for the channels they cover.
    png(&dir.join("floor_Roughness.png"), [255, 255, 255]);
    let mut s = session_with_ao();
    let r = import(&mut s, &dir, json!({})).unwrap();
    s.update();
    assert!((s.caches[0].composite.value(Channel::Roughness, 64, 64)[0] - 1.0).abs() < 0.01);
    assert!((s.caches[0].composite.value(Channel::Metallic, 64, 64)[0] - 0.9).abs() < 0.01, "still from the ORM image");
    assert!(r["maps"]["roughness"] == "floor_Roughness.png");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn glossiness_becomes_roughness() {
    let dir = folder("gloss");
    png(&dir.join("tile_diffuse.png"), [1, 1, 1]);
    png(&dir.join("tile_gloss.png"), [204, 204, 204]); // 0.8 glossy = 0.2 rough
    let mut s = session();
    let r = import(&mut s, &dir, json!({})).unwrap();
    assert!(r["channels"].as_array().unwrap().contains(&json!("roughness")), "{r}");
    s.update();
    assert!((s.caches[0].composite.value(Channel::Roughness, 64, 64)[0] - 0.2).abs() < 0.01);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_maps_stay_at_their_defaults() {
    let dir = folder("missing");
    png(&dir.join("only_albedo.png"), [200, 10, 10]);
    let mut s = session();
    let r = import(&mut s, &dir, json!({})).unwrap();
    assert_eq!(r["channels"], json!(["baseColor"]), "{r}");
    let fill = fill_of(&s, r["layer"].as_u64().unwrap());
    assert_eq!(fill.sources.len(), 1);
    // A folder with nothing recognisable is an error that changes nothing.
    let empty = folder("empty");
    png(&empty.join("holiday.png"), [1, 2, 3]);
    std::fs::write(empty.join("notes.txt"), "x").unwrap();
    let (layers, res) = (s.doc.project.texture_sets[0].stack.len(), s.doc.resources.len());
    assert!(import(&mut s, &empty, json!({})).is_err());
    assert_eq!((s.doc.project.texture_sets[0].stack.len(), s.doc.resources.len()), (layers, res));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn several_candidates_use_the_best_and_can_be_chosen() {
    let dir = folder("choose");
    png(&dir.join("m_albedo_1k.png"), [1, 1, 1]);
    png(&dir.join("m_albedo_4k.png"), [2, 2, 2]);
    png(&dir.join("m_nor_dx.png"), [128, 128, 255]);
    png(&dir.join("m_nor_gl.png"), [128, 128, 255]);
    let mut s = session();
    let scan = execute(&mut s, "file.scanPbrFolder", &json!({"path": dir.display().to_string()})).unwrap();
    assert_eq!(scan["maps"]["baseColor"], "m_albedo_4k.png");
    assert_eq!(scan["maps"]["normal"], "m_nor_gl.png", "the OpenGL file needs no flip");
    assert_eq!(scan["candidates"]["baseColor"].as_array().unwrap().len(), 2);
    assert!(s.doc.resources.is_empty(), "scanning changes nothing");
    let r = import(&mut s, &dir, json!({"choose": {"baseColor": "m_albedo_1k.png", "normal": "m_nor_dx.png"}})).unwrap();
    assert_eq!(r["maps"]["baseColor"], "m_albedo_1k.png");
    assert_eq!(r["normalFormat"], "directX");
    assert!(import(&mut s, &dir, json!({"choose": {"baseColor": "nope.png"}})).is_err());
    assert!(import(&mut s, &dir, json!({"choose": {"roughness": "m_albedo_1k.png"}})).is_err());
    assert!(import(&mut s, &dir, json!({"choose": {"bogus": "x"}})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn projection_name_and_selected_files() {
    let dir = folder("files");
    png(&dir.join("moss_col.png"), [1, 90, 1]);
    png(&dir.join("moss_rough.png"), [200, 200, 200]);
    png(&dir.join("other_albedo.png"), [9, 9, 9]);
    let mut s = session();
    let files: Vec<String> = ["moss_col.png", "moss_rough.png"].iter().map(|f| dir.join(f).display().to_string()).collect();
    let r = execute(&mut s, "file.importPbrFolder", &json!({"files": files, "projection": "triplanar", "scale": [2, 2]})).unwrap();
    assert_eq!(r["name"], "moss");
    let fill = fill_of(&s, r["layer"].as_u64().unwrap());
    assert!(matches!(fill.projection, Projection::Triplanar { scale, .. } if scale == 2.0), "{:?}", fill.projection);
    assert_eq!(fill.sources.len(), 2);
    let r = execute(&mut s, "file.importPbrFolder", &json!({"files": [dir.join("moss_col.png").display().to_string()], "name": "Mossy"})).unwrap();
    assert!(s.doc.project.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().name.starts_with("Mossy"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_zip_and_a_pack_of_folders() {
    let dir = folder("zip");
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    let one = {
        let mut b = Vec::new();
        image::RgbImage::from_pixel(4, 4, image::Rgb([5, 6, 7])).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    for name in ["brick/brick_albedo.png", "brick/brick_roughness.png", "__MACOSX/brick/._brick_albedo.png", "brick/preview.png"] {
        z.start_file(name, o).unwrap();
        z.write_all(&one).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    let path = dir.join("Brick Pack.zip");
    std::fs::write(&path, &bytes).unwrap();
    let mut s = session();
    let r = execute(&mut s, "file.importPbrFolder", &json!({"path": path.display().to_string()})).unwrap();
    assert_eq!(r["channels"], json!(["baseColor", "roughness"]), "{r}");
    assert_eq!(r["name"], "brick");

    // Two material folders in one pack: ask instead of guessing.
    let pack = folder("pack");
    for m in ["a", "b"] {
        std::fs::create_dir_all(pack.join(m)).unwrap();
        png(&pack.join(m).join(format!("{m}_albedo.png")), [1, 2, 3]);
    }
    let err = import(&mut s, &pack, json!({})).unwrap_err().to_string();
    assert!(err.contains("2 folders") && err.contains('a') && err.contains('b'), "{err}");
    // Pointing at one of them works; a folder with one material below it is found.
    assert!(import(&mut s, &pack.join("a"), json!({})).is_ok());
    std::fs::remove_dir_all(pack.join("b")).unwrap();
    assert!(import(&mut s, &pack, json!({})).is_ok());
    // Garbage zip.
    std::fs::write(dir.join("bad.zip"), b"PK\x03\x04garbage").unwrap();
    assert!(execute(&mut s, "file.importPbrFolder", &json!({"path": dir.join("bad.zip").display().to_string()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&pack);
}

#[test]
fn hostile_images_are_skipped_without_a_panic() {
    let dir = folder("hostile");
    png(&dir.join("w_albedo.png"), [1, 2, 3]);
    // Not an image at all, a truncated PNG, and an image that is too wide.
    std::fs::write(dir.join("w_roughness.png"), b"definitely not a png").unwrap();
    let good = std::fs::read(dir.join("w_albedo.png")).unwrap();
    std::fs::write(dir.join("w_metallic.png"), &good[..good.len() / 2]).unwrap();
    image::GrayImage::new(20_000, 1).save(dir.join("w_height.png")).unwrap();
    // More texels than one import accepts (67 108 864) in a file that is small on disk.
    image::GrayImage::new(16_384, 4_097).save(dir.join("w_emissive.png")).unwrap();
    // A folder named like an image, and a link out of the folder.
    std::fs::create_dir_all(dir.join("w_opacity.png")).unwrap();
    #[cfg(unix)]
    {
        let outside = dir.parent().unwrap().join(format!("tc_pbr_outside_{}.png", std::process::id()));
        png(&outside, [9, 9, 9]);
        std::os::unix::fs::symlink(&outside, dir.join("w_ao.png")).unwrap();
    }
    let mut s = session();
    let r = import(&mut s, &dir, json!({})).unwrap();
    assert_eq!(r["channels"], json!(["baseColor"]), "{r}");
    let warnings = r["warnings"].to_string();
    for needle in ["roughness", "metallic", "height", "emissive"] {
        assert!(warnings.contains(needle), "{needle}: {warnings}");
    }
    #[cfg(unix)]
    {
        assert!(!r["maps"].as_object().unwrap().contains_key("ambientOcclusion"), "a link is not followed: {r}");
        let _ = std::fs::remove_file(dir.parent().unwrap().join(format!("tc_pbr_outside_{}.png", std::process::id())));
    }
    // Every map hostile: an error, and the project is as it was.
    let only_bad = folder("onlybad");
    std::fs::write(only_bad.join("w_albedo.png"), b"nope").unwrap();
    let before = (s.doc.project.texture_sets[0].stack.len(), s.doc.resources.len());
    assert!(import(&mut s, &only_bad, json!({})).is_err());
    assert_eq!((s.doc.project.texture_sets[0].stack.len(), s.doc.resources.len()), before);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&only_bad);
}

#[test]
fn bad_params_are_errors() {
    let mut s = session();
    let run = |s: &mut Session, p: Value| execute(s, "file.importPbrFolder", &p);
    assert!(run(&mut s, json!({})).is_err());
    assert!(run(&mut s, json!({"path": ""})).is_err());
    assert!(run(&mut s, json!({"path": "/definitely/not/here"})).is_err());
    assert!(run(&mut s, json!({"files": [1, 2]})).is_err());
    assert!(run(&mut s, json!({"files": ["/definitely/not/here.png"]})).is_err());
    let dir = folder("params");
    png(&dir.join("a_albedo.png"), [1, 1, 1]);
    assert!(import(&mut s, &dir, json!({"normalFormat": "sideways"})).is_err());
    assert!(import(&mut s, &dir, json!({"projection": "nope"})).is_err());
    assert!(import(&mut s, &dir.join("a_albedo.png"), json!({})).is_err(), "a single image is not a folder or zip");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn byte_api_imports_without_a_file_system() {
    // What the browser uses: files already in memory.
    let png_bytes = |rgb| {
        let mut b = Vec::new();
        image::RgbImage::from_pixel(2, 2, image::Rgb(rgb)).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    let mut s = session();
    let files = vec![("t_albedo.png".to_string(), png_bytes([1, 2, 3])), ("t_nrm_dx.png".to_string(), png_bytes([128, 128, 255]))];
    let r = s.import_pbr_bytes(files, &texturecraft_engine::pbr_cmds::PbrOptions::default()).unwrap();
    assert_eq!(r["normalFormat"], "directX");
    assert_eq!(r["channels"], json!(["baseColor", "normal"]));
}
