//! `file.exportTextures` writes jng, pgm and psd, dithers 8-bit files on request and pads with
//! diffusion (texturecraft #672).

use serde_json::json;
use texturecraft_engine::execute;

#[test]
fn the_new_file_types_dithering_and_diffusion_export() {
    let mut s = texturecraft_engine::sample_session(128, 64).unwrap();
    let dir = std::env::temp_dir().join(format!("tc_export_file_types_{}", std::process::id()));
    for (format, depth) in [("jng", 8), ("pgm", 16), ("psd", 16), ("tiff", 32)] {
        let r = execute(
            &mut s,
            "file.exportTextures",
            &json!({"dir": dir.display().to_string(), "ts": 0, "format": format, "bitDepth": depth, "padding": "diffusion", "dilation": 4}),
        )
        .unwrap();
        let files = r["files"].as_array().unwrap();
        let ext = if format == "tiff" { "tif" } else { format };
        assert!(!files.is_empty() && files.iter().all(|f| f.as_str().unwrap().ends_with(&format!(".{ext}"))), "{format}: {files:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);

    let plan = execute(&mut s, "export.plan", &json!({"ts": 0, "format": "png", "bitDepth": 8, "dithering": true})).unwrap();
    assert!(plan.as_array().unwrap().iter().all(|f| f["dithering"] == true), "{plan}");
    let plan = execute(&mut s, "export.plan", &json!({"ts": 0, "format": "exr", "dithering": true})).unwrap();
    assert!(plan.as_array().unwrap().iter().all(|f| f["dithering"] == false), "float files are never dithered");
    assert!(execute(&mut s, "export.plan", &json!({"padding": "smudge"})).is_err());
}
