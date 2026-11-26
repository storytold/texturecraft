//! `file.exportTextures` / `export.plan` `maps`: per map file type, bit depth and leaving a map out
//! (texturecraft #670).

use serde_json::json;
use texturecraft_engine::execute;

fn session() -> texturecraft_engine::Session {
    texturecraft_engine::sample_session(128, 64).unwrap()
}

#[test]
fn a_map_gets_its_own_file_type_and_bit_depth_and_can_be_left_out() {
    let mut s = session();
    let plan = execute(&mut s, "export.plan", &json!({"ts": 0})).unwrap();
    let plan = plan.as_array().unwrap();
    assert!(plan.len() >= 3, "{plan:?}");
    let first = plan[0]["map"].as_u64().unwrap();
    let second = plan[1]["map"].as_u64().unwrap();
    let names: Vec<String> = plan.iter().map(|f| f["file"].as_str().unwrap().to_string()).collect();

    let maps = json!([{"map": first, "format": "targa", "bitDepth": 8}, {"map": second, "enabled": false}]);
    let changed = execute(&mut s, "export.plan", &json!({"ts": 0, "format": "png", "bitDepth": 16, "maps": maps})).unwrap();
    let changed = changed.as_array().unwrap();
    assert_eq!(changed.len(), plan.len() - 1, "the left-out map writes nothing");
    assert_eq!(changed[0]["format"], "tga");
    assert_eq!(changed[0]["bitDepth"], 8, "the map's own depth beats the export's");
    assert!(changed[1..].iter().all(|f| f["format"] == "png" && f["bitDepth"] == 16), "the others follow the export: {changed:?}");
    assert!(!changed.iter().any(|f| f["file"] == names[1].as_str()));
    assert_eq!(changed[1]["file"], names[2].as_str(), "the other maps keep their names");

    let dir = std::env::temp_dir().join(format!("tc_export_maps_{}", std::process::id()));
    let r = execute(&mut s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "ts": 0, "maps": maps})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files.len(), plan.len() - 1);
    assert!(files.iter().any(|f| f.ends_with(".tga")), "{files:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_map_index_outside_the_preset_is_refused() {
    let mut s = session();
    let e = execute(&mut s, "export.plan", &json!({"maps": [{"map": 999}]})).unwrap_err();
    assert!(e.to_string().contains("`map` must be the index"), "{e}");
    for (field, value) in [("format", json!("nope")), ("format", json!(8)), ("bitDepth", json!("16")), ("dithering", json!("true"))] {
        let mut map = json!({"map": 0});
        map[field] = value;
        assert!(execute(&mut s, "export.plan", &json!({"maps": [map]})).is_err(), "{map}");
    }
}

#[test]
fn planned_uv_tile_sizes_match_the_written_files() {
    let mut session =
        texturecraft_engine::Session::new_project("Export", texturecraft_engine::geom::primitives::udim_plane(), "builtin:udim_plane", None, 256).unwrap();
    execute(&mut session, "textureSet.tile.resolution", &json!({"tile": 1002, "resolution": 128})).unwrap();
    let directory = std::env::temp_dir().join(format!("tc-export-tile-sizes-{}", std::process::id()));
    let params = json!({"dir": directory, "format": "png", "size": 128, "ts": 0});

    let plan = execute(&mut session, "export.plan", &params).unwrap();
    let plan = plan.as_array().unwrap();
    assert!(plan.iter().any(|file| file["size"] == 64));
    assert!(plan.iter().any(|file| file["size"] == 128));
    execute(&mut session, "file.exportTextures", &params).unwrap();

    for file in plan {
        let bytes = std::fs::read(directory.join(file["file"].as_str().unwrap())).unwrap();
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(file["size"], width, "{file}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_file_name_an_earlier_texture_set_exports_is_not_overwritten() {
    let mut s = session();
    assert!(s.set_count() >= 2);
    execute(&mut s, "textureSet.rename", &json!({"ts": 0, "name": "SAME"})).unwrap();
    execute(&mut s, "textureSet.rename", &json!({"ts": 1, "name": "same"})).unwrap();

    let preset = json!({"name": "same", "maps": [
        {"file": "$textureSet", "format": "Png", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]},
        {"file": "$textureSet", "format": "Tga", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]},
        {"file": "other", "format": "Png", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]}
    ]});
    let single = execute(&mut s, "export.plan", &json!({"preset": preset, "ts": 1})).unwrap();
    let plan = execute(&mut s, "export.plan", &json!({"preset": preset, "ts": [1, 0]})).unwrap();
    let dir = std::env::temp_dir().join(format!("tc_export_same_name_{}", std::process::id()));
    let result = execute(&mut s, "file.exportTextures", &json!({"dir": dir.display().to_string(), "preset": preset, "ts": [1, 0]})).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let names = ["same.png", "same.tga", "other.png"];
    let single = single.as_array().unwrap();
    let plan = plan.as_array().unwrap();
    assert_eq!(single.len(), names.len());
    assert_eq!(plan.len(), names.len(), "{plan:?}");
    for (index, name) in names.into_iter().enumerate() {
        assert_eq!(single[index]["file"], name);
        assert_eq!(single[index]["warnings"], json!([]));
        assert_eq!(plan[index]["file"], name);
        assert_eq!(plan[index]["ts"], 1);
        let warning = plan[index]["warnings"][0].as_str().unwrap();
        assert!(warning.contains("an earlier texture set exports"), "{warning}");
        assert_eq!(result["warnings"][index], warning);
        assert_eq!(std::path::Path::new(result["files"][index].as_str().unwrap()).file_name().unwrap(), name);
    }
    assert_eq!(result["files"].as_array().unwrap().len(), names.len());
}
