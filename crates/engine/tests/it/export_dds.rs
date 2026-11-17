//! `file.exportTextures` / `export.plan` with `format: "dds"`, `ddsCompression` and `ddsMipmaps`.

use serde_json::{Value, json};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::builtin("plane").unwrap(), "builtin:plane", None, 128).unwrap();
    execute(&mut s, "layer.add.fill", &json!({ "color": [0.8, 0.2, 0.1] })).unwrap();
    s
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The written files by name, read back.
fn export(s: &mut Session, tag: &str, extra: Value) -> Vec<(String, Vec<u8>)> {
    let dir = std::env::temp_dir().join(format!("tc_export_dds_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut p = json!({ "dir": dir.display().to_string(), "preset": "glTF PBR Metal Roughness", "format": "dds" });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r = execute(s, "file.exportTextures", &p).unwrap();
    let files = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let path = f.as_str().unwrap();
            (path.rsplit(['/', '\\']).next().unwrap().to_string(), std::fs::read(path).unwrap())
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    files
}

fn file<'a>(files: &'a [(String, Vec<u8>)], part: &str) -> &'a [u8] {
    &files.iter().find(|(n, _)| n.contains(part)).unwrap_or_else(|| panic!("no {part}")).1
}

#[test]
fn dds_export_picks_a_compression_per_map_and_takes_the_options() {
    let mut s = session();
    let auto = export(&mut s, "auto", json!({}));
    assert!(auto.iter().all(|(n, b)| n.ends_with(".dds") && b.starts_with(b"DDS ")), "{:?}", auto.iter().map(|f| &f.0).collect::<Vec<_>>());
    assert_eq!(&file(&auto, "BaseColor")[84..88], b"DXT1", "colour without alpha: BC1");
    assert_eq!(&file(&auto, "OcclusionRoughnessMetallic")[84..88], b"DXT1");
    let normal = file(&auto, "Normal");
    assert_eq!((&normal[84..88], le32(normal, 128)), (&b"DX10"[..], 83), "normal maps: BC5 (DXGI_FORMAT_BC5_UNORM)");
    assert_eq!(normal.len(), 148 + 32 * 32 * 16, "one 128 px level of 16-byte 4x4 blocks after the DX10 header");
    assert!(auto.iter().all(|(_, b)| le32(b, 28) == 1), "no mipmaps unless asked");

    let bc3 = export(&mut s, "bc3", json!({ "ddsCompression": "DXT5", "ddsMipmaps": true }));
    for (name, b) in &bc3 {
        assert_eq!(&b[84..88], b"DXT5", "{name}");
        assert_eq!(le32(b, 28), 8, "{name}: 128 px down to 1 px");
    }
    let none = export(&mut s, "none", json!({ "ddsCompression": "none" }));
    assert!(none.iter().all(|(_, b)| le32(b, 80) & 0x4 == 0));
}

#[test]
fn the_plan_lists_dds_files_at_8_or_16_bits_and_bad_options_are_errors() {
    let mut s = session();
    // DDS has 8 and 16 bits; 32 is written at 16.
    for (depth, written) in [(8, 8), (16, 16), (32, 16)] {
        let plan = execute(&mut s, "export.plan", &json!({ "preset": "glTF PBR Metal Roughness", "format": "dds", "bitDepth": depth })).unwrap();
        let files = plan.as_array().unwrap();
        assert!(!files.is_empty());
        assert!(files.iter().all(|f| f["format"] == "dds" && f["bitDepth"] == written && f["file"].as_str().unwrap().ends_with(".dds")), "{plan}");
    }
    for bad in [json!("bc2"), json!("bc7"), json!(""), json!(3), json!(true)] {
        assert!(execute(&mut s, "export.plan", &json!({ "format": "dds", "ddsCompression": bad })).is_err(), "{bad}");
    }
    for bad in [json!("yes"), json!(1)] {
        assert!(execute(&mut s, "export.plan", &json!({ "format": "dds", "ddsMipmaps": bad })).is_err(), "{bad}");
    }
    assert!(execute(&mut s, "export.plan", &json!({ "format": "dds", "ddsCompression": null, "ddsMipmaps": null })).is_ok());
}
