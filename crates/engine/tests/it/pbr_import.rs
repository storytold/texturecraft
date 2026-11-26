//! `file.importPbrFolder` / `file.scanPbrFolder`: a folder, files or a zip of PBR maps becomes
//! one fill layer.

use std::io::Write as _;
use std::path::Path;

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, Fill, LayerId, LayerKind, NormalFormat, Projection, Source};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

mod test_folder;

use test_folder::TestFolder;

fn new_session() -> Session {
    Session::new_project("t", primitives::builtin("plane").unwrap(), "builtin:plane", None, 128).unwrap()
}

/// A session whose texture set also has an ambient occlusion channel (the default set has none).
fn session_with_ao() -> Session {
    let mut session = new_session();
    execute(&mut session, "textureSet.channel.add", &json!({"channel": "ambientOcclusion"})).unwrap();
    session
}

fn folder(tag: &str) -> TestFolder {
    TestFolder::new(tag)
}

#[test]
fn fixture_directory_is_removed_during_unwind() {
    let path = std::cell::RefCell::new(None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let directory = folder("unwind");
        *path.borrow_mut() = Some(directory.as_path().to_path_buf());
        std::fs::write(directory.join("fixture"), b"fixture").unwrap();
        panic!("exercise fixture cleanup");
    }));

    assert!(result.is_err());
    let path = path.into_inner().unwrap();
    let remained = path.exists();
    let _ = std::fs::remove_dir_all(&path);
    assert!(!remained, "fixture directory remained after unwind: {}", path.display());
}

fn png(path: &Path, rgb: [u8; 3]) {
    image::RgbImage::from_pixel(4, 4, image::Rgb(rgb)).save(path).unwrap();
}

fn fill_of(session: &Session, layer: u64) -> Fill {
    match &session.doc.project.layer(LayerId(layer)).unwrap().kind {
        LayerKind::Fill(f) => f.clone(),
        other => panic!("not a fill: {other:?}"),
    }
}

fn import(session: &mut Session, directory: &Path, extra: Value) -> Result<Value, texturecraft_engine::EngineError> {
    let mut p = json!({ "path": directory.display().to_string() });
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    execute(session, "file.importPbrFolder", &p)
}

fn image_name(f: &Fill, c: Channel) -> String {
    match f.sources.get(&c) {
        Some(Source::Image { name, .. }) => name.clone(),
        other => panic!("{c:?} is {other:?}"),
    }
}

#[test]
fn a_folder_becomes_one_fill_layer_in_one_undo_step() {
    let directory = folder("ok");
    for (name, rgb) in [
        ("Brick_Albedo.png", [200, 40, 40]),
        ("Brick_Roughness.png", [128, 128, 128]),
        ("Brick_Metallic.png", [0, 0, 0]),
        ("Brick_Normal_DX.png", [128, 51, 255]),
        ("Brick_Height.png", [90, 90, 90]),
        ("Brick_AO.png", [255, 255, 255]),
        ("Brick_Preview.png", [1, 2, 3]),
    ] {
        png(&directory.join(name), rgb);
    }
    std::fs::write(directory.join("readme.txt"), "hi").unwrap();
    let mut session = session_with_ao();
    let res = session.doc.resources.len();
    let reply = import(&mut session, &directory, json!({})).unwrap();
    let layer = reply["layer"].as_u64().unwrap();
    let fill = fill_of(&session, layer);
    for c in [Channel::BaseColor, Channel::Roughness, Channel::Metallic, Channel::Normal, Channel::Height, Channel::AmbientOcclusion] {
        assert!(matches!(fill.sources.get(&c), Some(Source::Image { .. })), "{c:?}: {:?}", fill.sources);
    }

    // The convention comes from `_DX` in the name; the preview and the text file are ignored
    assert!(matches!(fill.sources.get(&Channel::Normal), Some(Source::Image { normal_format: NormalFormat::DirectX, .. })));
    assert_eq!(reply["normalFormat"], "directX");
    assert_eq!(reply["maps"]["baseColor"], "Brick_Albedo.png");
    let ignored = reply["ignored"].to_string();
    assert!(ignored.contains("Brick_Preview.png") && ignored.contains("preview image") && ignored.contains("readme.txt"), "{ignored}");
    assert!(!session.doc.resources.keys().any(|k| k.to_lowercase().contains("preview")));
    assert_eq!(session.doc.resources.len(), res + 6);
    assert!(matches!(fill.projection, Projection::Uv { scale, .. } if scale == [1.0, 1.0]));
    let layer_name = session.doc.project.layer(LayerId(layer)).unwrap().name.clone();
    assert!(layer_name.starts_with("tc pbr ok"), "named after the folder: {layer_name}");
    assert_eq!(reply["name"], layer_name);

    // Every decoded image is ready for the viewport
    for n in session.doc.resources.keys() {
        assert!(session.images.contains_key(n), "{n}");
    }
    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(session.doc.resources.len(), res);
    assert!(session.doc.project.layer(LayerId(layer)).is_err());
}

#[test]
fn a_directx_normal_map_is_flipped_to_opengl_where_it_is_sampled() {
    // Green 0.2 in the file: a DirectX map reads 0.8 as OpenGL, an OpenGL map stays 0.2
    let green = |files: &[&str], extra: Value| -> f32 {
        let directory = folder("flip");
        for f in files {
            png(&directory.join(f), [128, 51, 255]);
        }
        let mut session = new_session();
        import(&mut session, &directory, extra).unwrap();
        session.update();
        let v = session.caches[0].composite.value(Channel::Normal, 64, 64);
        v[1]
    };
    let (dx, gl) = (green(&["wall_normal_dx.png"], json!({})), green(&["wall_normal_gl.png"], json!({})));
    assert!((dx - 0.8).abs() < 0.02, "DirectX name flips: {dx}");
    assert!((gl - 0.2).abs() < 0.02, "OpenGL name does not: {gl}");

    // No convention in the name: OpenGL by default, and the toggle decides
    assert!((green(&["wall_normal.png"], json!({})) - 0.2).abs() < 0.02);
    assert!((green(&["wall_normal.png"], json!({"normalFormat": "directX"})) - 0.8).abs() < 0.02);

    // The toggle forces the convention even against the name
    assert!((green(&["wall_normal_dx.png"], json!({"normalFormat": "openGl"})) - 0.2).abs() < 0.02);
    assert!((green(&["wall_normal_dx.png"], json!({"normalFormat": "auto"})) - 0.8).abs() < 0.02);
}

#[test]
fn a_packed_orm_image_is_split_into_three_maps() {
    let directory = folder("orm");

    // Occlusion 0.2 (R), roughness 0.5 (G), metallic 0.9 (B)
    png(&directory.join("floor_ORM.png"), [51, 128, 230]);
    png(&directory.join("floor_BaseColor.png"), [10, 20, 30]);
    let mut session = session_with_ao();
    let reply = import(&mut session, &directory, json!({})).unwrap();
    let fill = fill_of(&session, reply["layer"].as_u64().unwrap());
    let names: Vec<String> = [Channel::AmbientOcclusion, Channel::Roughness, Channel::Metallic].iter().map(|c| image_name(&fill, *c)).collect();
    assert_eq!(names.iter().collect::<std::collections::BTreeSet<_>>().len(), 3, "three separate resources: {names:?}");
    session.update();
    let at = |c| session.caches[0].composite.value(c, 64, 64)[0];
    assert!((at(Channel::AmbientOcclusion) - 0.2).abs() < 0.01, "{}", at(Channel::AmbientOcclusion));
    assert!((at(Channel::Roughness) - 0.5).abs() < 0.01);
    assert!((at(Channel::Metallic) - 0.9).abs() < 0.01);

    // The saved resources are real images that decode to the same values after a reload
    for n in &names {
        let d = texturecraft_engine::formats::decode_image(&session.doc.resources[n]).unwrap();
        assert_eq!((d.width, d.height), (4, 4));
    }

    // A set without an occlusion channel skips it with a warning and keeps the rest
    let mut plain = new_session();
    let reply = import(&mut plain, &directory, json!({})).unwrap();
    assert!(reply["warnings"].to_string().contains("ambientOcclusion was skipped"), "{reply}");
    assert!(reply["channels"].as_array().unwrap().contains(&json!("metallic")));

    // Separate maps win over the packed image for the channels they cover
    png(&directory.join("floor_Roughness.png"), [255, 255, 255]);
    let mut session = session_with_ao();
    let reply = import(&mut session, &directory, json!({})).unwrap();
    session.update();
    assert!((session.caches[0].composite.value(Channel::Roughness, 64, 64)[0] - 1.0).abs() < 0.01);
    assert!((session.caches[0].composite.value(Channel::Metallic, 64, 64)[0] - 0.9).abs() < 0.01, "still from the ORM image");
    assert!(reply["maps"]["roughness"] == "floor_Roughness.png");
}

#[test]
fn glossiness_becomes_roughness() {
    let directory = folder("gloss");
    png(&directory.join("tile_diffuse.png"), [1, 1, 1]);
    png(&directory.join("tile_gloss.png"), [204, 204, 204]); // 0.8 glossy = 0.2 rough
    let mut session = new_session();
    let reply = import(&mut session, &directory, json!({})).unwrap();
    assert!(reply["channels"].as_array().unwrap().contains(&json!("roughness")), "{reply}");
    session.update();
    assert!((session.caches[0].composite.value(Channel::Roughness, 64, 64)[0] - 0.2).abs() < 0.01);
}

#[test]
fn missing_maps_stay_at_their_defaults() {
    let directory = folder("missing");
    png(&directory.join("only_albedo.png"), [200, 10, 10]);
    let mut session = new_session();
    let reply = import(&mut session, &directory, json!({})).unwrap();
    assert_eq!(reply["channels"], json!(["baseColor"]), "{reply}");
    let fill = fill_of(&session, reply["layer"].as_u64().unwrap());
    assert_eq!(fill.sources.len(), 1);

    // A folder with nothing recognisable is an error that changes nothing
    let empty = folder("empty");
    png(&empty.join("holiday.png"), [1, 2, 3]);
    std::fs::write(empty.join("notes.txt"), "x").unwrap();
    let (layers, res) = (session.doc.project.texture_sets[0].stack.len(), session.doc.resources.len());
    assert!(import(&mut session, &empty, json!({})).is_err());
    assert_eq!((session.doc.project.texture_sets[0].stack.len(), session.doc.resources.len()), (layers, res));
}

#[test]
fn several_candidates_use_the_best_and_can_be_chosen() {
    let directory = folder("choose");
    png(&directory.join("m_albedo_1k.png"), [1, 1, 1]);
    png(&directory.join("m_albedo_4k.png"), [2, 2, 2]);
    png(&directory.join("m_nor_dx.png"), [128, 128, 255]);
    png(&directory.join("m_nor_gl.png"), [128, 128, 255]);
    let mut session = new_session();
    let scan = execute(&mut session, "file.scanPbrFolder", &json!({"path": directory.display().to_string()})).unwrap();
    assert_eq!(scan["maps"]["baseColor"], "m_albedo_4k.png");
    assert_eq!(scan["maps"]["normal"], "m_nor_gl.png", "the OpenGL file needs no flip");
    assert_eq!(scan["candidates"]["baseColor"].as_array().unwrap().len(), 2);
    assert!(session.doc.resources.is_empty(), "scanning changes nothing");
    let reply = import(&mut session, &directory, json!({"choose": {"baseColor": "m_albedo_1k.png", "normal": "m_nor_dx.png"}})).unwrap();
    assert_eq!(reply["maps"]["baseColor"], "m_albedo_1k.png");
    assert_eq!(reply["normalFormat"], "directX");
    assert!(import(&mut session, &directory, json!({"choose": {"baseColor": "nope.png"}})).is_err());
    assert!(import(&mut session, &directory, json!({"choose": {"roughness": "m_albedo_1k.png"}})).is_err());
    assert!(import(&mut session, &directory, json!({"choose": {"bogus": "x"}})).is_err());
}

#[test]
fn projection_name_and_selected_files() {
    let directory = folder("files");
    png(&directory.join("moss_col.png"), [1, 90, 1]);
    png(&directory.join("moss_rough.png"), [200, 200, 200]);
    png(&directory.join("other_albedo.png"), [9, 9, 9]);
    let mut session = new_session();
    let files: Vec<String> = ["moss_col.png", "moss_rough.png"].iter().map(|f| directory.join(f).display().to_string()).collect();
    let reply = execute(&mut session, "file.importPbrFolder", &json!({"files": files, "projection": "triplanar", "scale": [2, 2]})).unwrap();
    assert_eq!(reply["name"], "moss 1");
    let fill = fill_of(&session, reply["layer"].as_u64().unwrap());
    assert!(matches!(fill.projection, Projection::Triplanar { scale, .. } if scale == 2.0), "{:?}", fill.projection);
    assert_eq!(fill.sources.len(), 2);
    let reply =
        execute(&mut session, "file.importPbrFolder", &json!({"files": [directory.join("moss_col.png").display().to_string()], "name": "Mossy"})).unwrap();
    assert!(session.doc.project.layer(LayerId(reply["layer"].as_u64().unwrap())).unwrap().name.starts_with("Mossy"));
}

#[test]
fn folder_and_archive_group_names_keep_dots() {
    let directory = folder("wood_floor_1.5k");
    png(&directory.join("wood_albedo.png"), [1, 2, 3]);
    let mut session = new_session();
    let reply = import(&mut session, &directory, json!({})).unwrap();
    assert!(reply["name"].as_str().unwrap().contains("1.5k"), "{reply}");

    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive.start_file("wood_1.5k/wood_albedo.png", zip::write::SimpleFileOptions::default()).unwrap();
    archive.write_all(&std::fs::read(directory.join("wood_albedo.png")).unwrap()).unwrap();
    let zip_path = directory.join("wood.zip");
    std::fs::write(&zip_path, archive.finish().unwrap().into_inner()).unwrap();
    let reply = import(&mut session, &zip_path, json!({})).unwrap();
    assert_eq!(reply["name"], "wood 1.5k 1");
}

#[test]
fn a_zip_and_a_pack_of_folders() {
    let directory = folder("zip");
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    let one = {
        let mut b = Vec::new();
        image::RgbImage::from_pixel(4, 4, image::Rgb([5, 6, 7])).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    for name in ["brick/brick_albedo.png", "brick/brick_roughness.png", "__MACOSX/brick/._brick_albedo.png", "brick/preview.png"] {
        archive.start_file(name, options).unwrap();
        archive.write_all(&one).unwrap();
    }
    let bytes = archive.finish().unwrap().into_inner();
    let path = directory.join("Brick Pack.zip");
    std::fs::write(&path, &bytes).unwrap();
    let mut session = new_session();
    let reply = execute(&mut session, "file.importPbrFolder", &json!({"path": path.display().to_string()})).unwrap();
    assert_eq!(reply["channels"], json!(["baseColor", "roughness"]), "{reply}");
    assert_eq!(reply["name"], "brick 1");

    // Two material folders in one pack: ask instead of guessing
    let pack = folder("pack");
    for m in ["a", "b"] {
        std::fs::create_dir_all(pack.join(m)).unwrap();
        png(&pack.join(m).join(format!("{m}_albedo.png")), [1, 2, 3]);
    }
    let err = import(&mut session, &pack, json!({})).unwrap_err().to_string();
    assert!(err.contains("2 folders") && err.contains('a') && err.contains('b'), "{err}");

    // Pointing at one of them works; a folder with one material below it is found
    assert!(import(&mut session, &pack.join("a"), json!({})).is_ok());
    std::fs::remove_dir_all(pack.join("b")).unwrap();
    assert!(import(&mut session, &pack, json!({})).is_ok());

    // Garbage zip
    std::fs::write(directory.join("bad.zip"), b"PK\x03\x04garbage").unwrap();
    assert!(execute(&mut session, "file.importPbrFolder", &json!({"path": directory.join("bad.zip").display().to_string()})).is_err());
}

#[test]
fn hostile_images_are_skipped_without_a_panic() {
    let directory = folder("hostile");
    png(&directory.join("w_albedo.png"), [1, 2, 3]);

    // Not an image at all, a truncated PNG, and an image that is too wide
    std::fs::write(directory.join("w_roughness.png"), b"definitely not a png").unwrap();
    let good = std::fs::read(directory.join("w_albedo.png")).unwrap();
    std::fs::write(directory.join("w_metallic.png"), &good[..good.len() / 2]).unwrap();
    image::GrayImage::new(20_000, 1).save(directory.join("w_height.png")).unwrap();

    // A header alone must reject a source beyond the per-image budget
    let mut header = [0u8; 18];
    header[2] = 2;
    header[12..14].copy_from_slice(&16_384u16.to_le_bytes());
    header[14..16].copy_from_slice(&4_097u16.to_le_bytes());
    header[16] = 24;
    std::fs::write(directory.join("w_emissive.tga"), header).unwrap();

    // A folder named like an image, and a link out of the folder
    std::fs::create_dir_all(directory.join("w_opacity.png")).unwrap();
    #[cfg(unix)]
    let outside_target = {
        let outside_dir = folder("outside");
        let outside = outside_dir.join("w_ao.png");
        png(&outside, [9, 9, 9]);
        std::os::unix::fs::symlink(&outside, directory.join("w_ao.png")).unwrap();
        outside_dir
    };
    let mut session = new_session();
    execute(&mut session, "textureSet.channel.add", &json!({"channel":"emissive"})).unwrap();
    let reply = import(&mut session, &directory, json!({})).unwrap();
    assert_eq!(reply["channels"], json!(["baseColor"]), "{reply}");
    let warnings = reply["warnings"].to_string();
    assert!(warnings.contains("more than") && warnings.contains("texels"), "header limit must run: {warnings}");
    for needle in ["roughness", "metallic", "height", "emissive"] {
        assert!(warnings.contains(needle), "{needle}: {warnings}");
    }
    #[cfg(unix)]
    {
        assert!(!reply["maps"].as_object().unwrap().contains_key("ambientOcclusion"), "a link is not followed: {reply}");
        assert!(outside_target.join("w_ao.png").exists());
    }

    // Every map hostile: an error, and the project is as it was
    let only_bad = folder("onlybad");
    std::fs::write(only_bad.join("w_albedo.png"), b"nope").unwrap();
    let before = (session.doc.project.texture_sets[0].stack.len(), session.doc.resources.len());
    assert!(import(&mut session, &only_bad, json!({})).is_err());
    assert_eq!((session.doc.project.texture_sets[0].stack.len(), session.doc.resources.len()), before);
}

#[test]
fn bad_params_are_errors() {
    let mut session = new_session();
    let run = |session: &mut Session, p: Value| execute(session, "file.importPbrFolder", &p);
    assert!(run(&mut session, json!({})).is_err());
    assert!(run(&mut session, json!({"path": ""})).is_err());
    assert!(run(&mut session, json!({"path": "/definitely/not/here"})).is_err());
    assert!(run(&mut session, json!({"files": [1, 2]})).is_err());
    assert!(run(&mut session, json!({"files": ["/definitely/not/here.png"]})).is_err());
    let directory = folder("params");
    png(&directory.join("a_albedo.png"), [1, 1, 1]);
    assert!(import(&mut session, &directory, json!({"normalFormat": "sideways"})).is_err());
    assert!(import(&mut session, &directory, json!({"projection": "nope"})).is_err());
    assert!(import(&mut session, &directory.join("a_albedo.png"), json!({})).is_err(), "a single image is not a folder or zip");
}

#[test]
fn byte_api_imports_without_a_file_system() {
    // What the browser uses: files already in memory
    let png_bytes = |rgb| {
        let mut b = Vec::new();
        image::RgbImage::from_pixel(2, 2, image::Rgb(rgb)).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    let mut session = new_session();
    let files = vec![("t_albedo.png".to_string(), png_bytes([1, 2, 3])), ("t_nrm_dx.png".to_string(), png_bytes([128, 128, 255]))];
    let reply = session.import_pbr_bytes(files, &texturecraft_engine::pbr_cmds::PbrOptions::default()).unwrap();
    assert_eq!(reply["normalFormat"], "directX");
    assert_eq!(reply["channels"], json!(["baseColor", "normal"]));

    let first_image = std::sync::Arc::downgrade(session.images.values().next().unwrap());
    let first_bytes = std::sync::Arc::downgrade(session.doc.resources.values().next().unwrap());
    let first_resource_count = session.doc.resources.len();
    let first_image_count = session.images.len();
    session
        .import_pbr_bytes(vec![("other_roughness.png".to_string(), png_bytes([64, 64, 64]))], &texturecraft_engine::pbr_cmds::PbrOptions::default())
        .unwrap();
    assert_eq!(session.doc.resources.len(), first_resource_count + 1);
    assert_eq!(session.images.len(), first_image_count + 1);

    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(session.doc.resources.len(), first_resource_count);
    assert_eq!(session.images.len(), first_image_count);

    execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(session.doc.resources.is_empty());
    assert!(session.images.is_empty());
    assert!(first_image.upgrade().is_none(), "decoded pixels must be released after undo");
    assert!(first_bytes.upgrade().is_some(), "redo intentionally retains compressed bytes");

    drop(session);

    assert!(first_bytes.upgrade().is_none(), "dropping the session releases history too");
}

#[test]
fn repeated_import_reports_the_created_layer_name() {
    let directory = folder("repeated_name");
    png(&directory.join("moss_albedo.png"), [1, 90, 1]);
    let mut session = new_session();
    import(&mut session, &directory, json!({"name": "Moss"})).unwrap();

    let reply = import(&mut session, &directory, json!({"name": "Moss"})).unwrap();

    let layer = session.doc.project.layer(LayerId(reply["layer"].as_u64().unwrap())).unwrap();
    assert_ne!(layer.name, "Moss");
    assert_eq!(reply["name"], layer.name);
}

#[test]
fn scans_reject_excessive_entries_instead_of_silently_truncating() {
    let directory = folder("scan_limit");
    let path = directory.join("moss_albedo.png");
    png(&path, [1, 90, 1]);
    let mut session = new_session();
    let files = vec![path; texturecraft_engine::pbr_cmds::MAX_FILES + 1];

    let selected = execute(&mut session, "file.scanPbrFolder", &json!({"files": files}));

    assert!(selected.unwrap_err().to_string().contains("limit"));
}

#[test]
fn scans_count_empty_directories_toward_the_limit() {
    let directory = folder("directory_limit");
    png(&directory.join("moss_albedo.png"), [1, 90, 1]);
    let mut session = new_session();
    for index in 0..texturecraft_engine::pbr_cmds::MAX_FILES {
        std::fs::create_dir(directory.join(index.to_string())).unwrap();
    }

    let scanned = execute(&mut session, "file.scanPbrFolder", &json!({"path": directory.display().to_string()}));

    assert!(scanned.unwrap_err().to_string().contains("limit"));
}
