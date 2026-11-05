//! Reference images: commands, the project round trip, linked files, corrupt data and exports.

use std::path::PathBuf;

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, Project};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    Session::new_project("t", primitives::builtin("plane").unwrap(), "builtin:plane", None, 128).unwrap()
}

fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Left half red, right half 50% grey.
fn two_tone() -> Vec<u8> {
    png(4, 2, |x, _| if x < 2 { [255, 0, 0, 255] } else { [128, 128, 128, 255] })
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tc_reference_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn list(s: &mut Session) -> Vec<Value> {
    execute(s, "reference.list", &Value::Null).unwrap()["references"].as_array().unwrap().clone()
}

#[test]
fn embedded_references_round_trip_with_the_project() {
    let mut s = session();
    let id = s.add_reference_bytes("/photos/Wood Grain.png", two_tone()).unwrap();
    execute(&mut s, "reference.set", &json!({"id": id, "flipH": true, "grayscale": true, "name": "Oak"})).unwrap();
    execute(&mut s, "reference.overlay", &json!({"reference": id, "opacity": 0.3, "x": 0.4, "y": 0.6, "scale": 0.7})).unwrap();

    let bytes = s.to_project_bytes().unwrap();
    let back = Session::from_project_bytes(&bytes).unwrap();
    assert_eq!(back.doc.project.references, s.doc.project.references);
    assert_eq!(back.doc.project.reference_overlay, s.doc.project.reference_overlay);
    let img = back.reference_image(id).expect("the embedded copy decodes after load");
    assert_eq!((img.width, img.height), (4, 2));
    let mut back = back;
    let r = &list(&mut back)[0];
    assert_eq!(
        (r["status"].as_str(), r["storage"].as_str(), r["name"].as_str(), r["flipH"].as_bool()),
        (Some("ok"), Some("embedded"), Some("Oak"), Some(true))
    );
    // The copy is not an Assets-list resource's business: it is hidden from "remove unused".
    execute(&mut back, "file.removeUnused", &Value::Null).unwrap();
    assert!(back.reference_image(id).is_some());
    assert_eq!(back.doc.resources.keys().filter(|k| k.starts_with("reference.")).count(), 1);
    assert!(execute(&mut back, "resource.list", &Value::Null).unwrap().as_array().unwrap().is_empty(), "not listed as an imported resource");
}

#[test]
fn projects_without_references_load_and_stay_clean() {
    let s = session();
    let json = serde_json::to_value(&s.doc.project).unwrap();
    assert!(json.get("references").is_none() && json.get("reference_overlay").is_none(), "empty settings are not written");
    // A project written before references existed has neither key.
    let old: Project = serde_json::from_value(json).unwrap();
    assert!(old.references.is_empty() && old.reference_overlay.is_default());
    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    assert!(back.doc.project.references.is_empty());
}

#[test]
fn removing_and_undoing() {
    let mut s = session();
    let id = s.add_reference_bytes("a.png", two_tone()).unwrap();
    execute(&mut s, "reference.overlay", &json!({"reference": id})).unwrap();
    execute(&mut s, "reference.remove", &json!({"id": id})).unwrap();
    assert!(list(&mut s).is_empty() && s.doc.project.reference_overlay.reference.is_none());
    assert!(!s.doc.resources.keys().any(|k| k.starts_with("reference.")), "the embedded copy goes with it");
    assert!(execute(&mut s, "reference.remove", &json!({"id": id})).is_err());
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(list(&mut s).len(), 1);
    assert!(s.reference_image(id).is_some(), "undo brings the image back");
    assert_eq!(s.doc.project.reference_overlay.reference, Some(id));
}

#[test]
fn picking_sets_the_brush_color_in_linear_light() {
    let mut s = session();
    let id = s.add_reference_bytes("a.png", two_tone()).unwrap();
    let r = execute(&mut s, "reference.pick", &json!({"id": id, "u": 0.9, "v": 0.5})).unwrap();
    assert_eq!(r["srgb"], "#808080");
    let lin = r["linear"][0].as_f64().unwrap();
    assert!((lin - 0.2158).abs() < 1e-3, "sRGB 128 is {lin} linear");
    assert!((s.tool.values[&Channel::BaseColor][0] as f64 - lin).abs() < 1e-6);
    // Flipped, the same spot shows the red half; `apply: false` leaves the brush alone.
    execute(&mut s, "reference.set", &json!({"id": id, "flipH": true})).unwrap();
    let r = execute(&mut s, "reference.pick", &json!({"id": id, "u": 0.9, "v": 0.5, "apply": false})).unwrap();
    assert_eq!(r["srgb"], "#ff0000");
    assert!((s.tool.values[&Channel::BaseColor][0] as f64 - lin).abs() < 1e-6);
    // A radius averages in linear light: the 3x2 window at the seam holds 2 red and 4 grey pixels.
    execute(&mut s, "reference.set", &json!({"id": id, "flipH": false})).unwrap();
    let r = execute(&mut s, "reference.pick", &json!({"id": id, "u": 0.5, "v": 0.5, "radius": 1, "apply": false})).unwrap();
    assert!((r["linear"][0].as_f64().unwrap() - (2.0 + 4.0 * 0.2158) / 6.0).abs() < 2e-3, "{r}");
    for bad in [json!({"id": id}), json!({"id": 999, "u": 0, "v": 0}), json!({"id": id, "u": "x", "v": 0})] {
        assert!(execute(&mut s, "reference.pick", &bad).is_err(), "{bad}");
    }
}

#[test]
fn corrupt_images_are_refused_or_flagged_never_a_panic() {
    let mut s = session();
    for bytes in [Vec::new(), b"definitely not an image".to_vec(), two_tone()[..20].to_vec()] {
        assert!(s.add_reference_bytes("bad.png", bytes).is_err());
    }
    assert!(s.doc.project.references.is_empty() && s.doc.resources.is_empty(), "a refused image leaves no trace");
    // An embedded copy that turns up corrupt in a project file only flags its reference.
    let id = s.add_reference_bytes("ok.png", two_tone()).unwrap();
    let resource = s.doc.resources.keys().find(|k| k.starts_with("reference.")).cloned().unwrap();
    s.doc.resources.insert(resource, std::sync::Arc::new(vec![1, 2, 3, 4]));
    let back = Session::from_project_bytes(&s.to_project_bytes().unwrap()).unwrap();
    let mut back = back;
    let r = &list(&mut back)[0];
    assert_eq!((r["id"].as_u64(), r["status"].as_str()), (Some(id), Some("invalid")));
    assert!(r["error"].is_string());
    assert!(execute(&mut back, "reference.pick", &json!({"id": id, "u": 0.5, "v": 0.5})).is_err());
    // The project still works and saves again.
    back.to_project_bytes().unwrap();
}

#[test]
fn linked_references_use_paths_relative_to_the_project_and_survive_missing_files() {
    let dir = scratch("linked");
    let refs = dir.join("refs");
    std::fs::create_dir_all(&refs).unwrap();
    std::fs::write(refs.join("wood.png"), two_tone()).unwrap();
    let project = dir.join("job").join("model.tcraft");
    std::fs::create_dir_all(project.parent().unwrap()).unwrap();

    let mut s = session();
    let id = {
        let r = execute(&mut s, "reference.add", &json!({"path": refs.join("wood.png").display().to_string(), "storage": "linked"})).unwrap();
        assert_eq!(r["storage"], "linked");
        r["id"].as_u64().unwrap()
    };
    assert!(s.doc.resources.is_empty(), "a linked image is not copied into the project");
    // Unsaved project: the absolute path. Saving rewrites it relative to the project file.
    execute(&mut s, "file.save", &json!({"path": project.display().to_string()})).unwrap();
    let stored = list(&mut s)[0]["path"].as_str().unwrap().to_string();
    assert_eq!(stored, "../refs/wood.png");
    assert!(s.reference_image(id).is_some());

    // Reopen: resolved against the project's folder.
    let mut open = texturecraft_engine::file_cmds::open_path(&project.display().to_string()).unwrap();
    assert_eq!(list(&mut open)[0]["status"], "ok");

    // The whole tree moves together: still found.
    let moved = scratch("linked_moved");
    std::fs::rename(&dir, moved.join("t")).unwrap();
    let moved_project = moved.join("t").join("job").join("model.tcraft");
    let mut open = texturecraft_engine::file_cmds::open_path(&moved_project.display().to_string()).unwrap();
    assert_eq!(list(&mut open)[0]["status"], "ok");

    // The image goes missing: the project opens, the reference says so, and reload finds it again.
    let image = moved.join("t").join("refs").join("wood.png");
    let kept = std::fs::read(&image).unwrap();
    std::fs::remove_file(&image).unwrap();
    let mut open = texturecraft_engine::file_cmds::open_path(&moved_project.display().to_string()).unwrap();
    let r = list(&mut open)[0].clone();
    assert_eq!((r["status"].as_str(), r["storage"].as_str()), (Some("missing"), Some("linked")));
    assert!(r["error"].as_str().unwrap().contains("wood.png"));
    assert!(execute(&mut open, "reference.pick", &json!({"id": id, "u": 0.5, "v": 0.5})).is_err());
    open.to_project_bytes().unwrap();
    std::fs::write(&image, kept).unwrap();
    execute(&mut open, "reference.reload", &json!({})).unwrap();
    assert_eq!(list(&mut open)[0]["status"], "ok");
    // A linked file that is not an image is flagged, not fatal.
    std::fs::write(&image, b"garbage").unwrap();
    execute(&mut open, "reference.reload", &json!({"id": id})).unwrap();
    assert_eq!(list(&mut open)[0]["status"], "invalid");
    let _ = std::fs::remove_dir_all(&moved);
}

#[test]
fn saving_in_another_folder_keeps_linked_paths_working() {
    let dir = scratch("saveas");
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    std::fs::write(dir.join("pic.png"), two_tone()).unwrap();
    let (first, second) = (dir.join("a").join("p.tcraft"), dir.join("b").join("p.tcraft"));
    let mut s = session();
    let id = s.add_reference_file(&dir.join("pic.png").display().to_string(), true).unwrap();
    execute(&mut s, "file.save", &json!({"path": first.display().to_string()})).unwrap();
    assert_eq!(list(&mut s)[0]["path"], "../pic.png");
    execute(&mut s, "file.saveAs", &json!({"path": second.display().to_string()})).unwrap();
    assert_eq!(list(&mut s)[0]["path"], "../pic.png");
    // A copy elsewhere is written for its own folder.
    std::fs::create_dir_all(dir.join("x").join("y")).unwrap();
    let copy = dir.join("x").join("y").join("c.tcraft");
    execute(&mut s, "file.saveCopy", &json!({"path": copy.display().to_string()})).unwrap();
    let mut opened = texturecraft_engine::file_cmds::open_path(&copy.display().to_string()).unwrap();
    assert_eq!(list(&mut opened)[0]["path"], "../../pic.png");
    assert_eq!(list(&mut opened)[0]["status"], "ok");
    assert_eq!(opened.reference_image(id).map(|i| i.width), Some(4));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_file_cannot_make_a_headless_session_read_files() {
    let dir = scratch("headless");
    std::fs::write(dir.join("pic.png"), two_tone()).unwrap();
    let mut s = session();
    s.add_reference_file(&dir.join("pic.png").display().to_string(), true).unwrap();
    let bytes = s.to_project_bytes().unwrap();
    // `from_project_bytes` is what the headless and MCP front ends use.
    let mut loaded = Session::from_project_bytes(&bytes).unwrap();
    let r = list(&mut loaded)[0].clone();
    assert_eq!(r["status"], "missing");
    assert!(r["error"].as_str().unwrap().contains("not read in this session"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_params_are_errors() {
    let mut s = session();
    let id = s.add_reference_bytes("a.png", two_tone()).unwrap();
    for (cmd, p) in [
        ("reference.add", json!({})),
        ("reference.add", json!({"path": "/definitely/not/here.png"})),
        ("reference.add", json!({"path": "/etc/passwd"})),
        ("reference.add", json!({"resource": "nope"})),
        ("reference.add", json!({"resource": "nope", "storage": "weird"})),
        ("reference.set", json!({"id": u64::MAX})),
        ("reference.overlay", json!({"reference": 12345})),
        ("reference.overlay", json!({"reference": "x"})),
        ("reference.reload", json!({"id": 777})),
    ] {
        assert!(execute(&mut s, cmd, &p).is_err(), "{cmd} {p}");
    }
    // Numbers are clamped, never stored raw.
    execute(&mut s, "reference.overlay", &json!({"reference": id, "opacity": 9e9, "x": -1e30, "scale": 0})).unwrap();
    let o = &s.doc.project.reference_overlay;
    assert_eq!((o.opacity, o.center[0], o.scale), (1.0, -1.0, 0.02));
    // The cap on references.
    for _ in 0..texturecraft_engine::core::MAX_REFERENCES {
        let _ = s.add_reference_bytes("a.png", two_tone());
    }
    assert_eq!(s.doc.project.references.len(), texturecraft_engine::core::MAX_REFERENCES);
    assert!(s.add_reference_bytes("a.png", two_tone()).is_err());
}

#[test]
fn references_and_the_overlay_never_reach_an_export() {
    let export = |s: &mut Session| -> Vec<(String, Vec<u8>)> {
        let sets: Vec<usize> = (0..s.set_count()).collect();
        let preset = s.export_preset("Document channels + Normal + AO").unwrap();
        let opts = texturecraft_engine::formats::export::ExportOptions::new(preset);
        s.export_files(&sets, &opts).unwrap().into_iter().map(|f| (f.name, f.bytes)).collect()
    };
    let mut plain = session();
    execute(&mut plain, "layer.add.fill", &json!({ "color": [0.8, 0.2, 0.1] })).unwrap();
    let before = export(&mut plain);
    assert!(!before.is_empty());
    let id = plain.add_reference_bytes("a.png", two_tone()).unwrap();
    execute(&mut plain, "reference.overlay", &json!({"reference": id, "opacity": 1.0, "scale": 2.0})).unwrap();
    assert_eq!(export(&mut plain), before);
}
