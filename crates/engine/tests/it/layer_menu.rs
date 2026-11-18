//! Layer menu commands: lock a layer against painting, a gamma corrected mask, and exporting a
//! layer's channels to files.

use std::path::PathBuf;

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const RESOLUTION: u32 = 128;

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, RESOLUTION).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

fn add(s: &mut Session, command: &str, p: Value) -> LayerId {
    LayerId(execute(s, command, &p).unwrap()["layer"].as_u64().unwrap())
}

fn stroke(s: &mut Session) -> texturecraft_engine::Result<Value> {
    execute(s, "tool.brush", &json!({"size": 60, "hardness": 100})).unwrap();
    let done = execute(s, "tool.stroke", &json!({"points": [[150, 200], [250, 200]]}));
    s.update();
    done
}

/// The base colour of every covered texel, to see whether a stroke changed anything.
fn base_colors(s: &Session) -> Vec<[f32; 3]> {
    let g = &s.caches[0].gbuf;
    (0..g.width * g.height)
        .filter(|&i| g.tri[i as usize] != texturecraft_engine::geom::NO_TRI)
        .map(|i| s.caches[0].composite.value(Channel::BaseColor, i % g.width, i / g.width))
        .collect()
}

#[test]
fn a_locked_layer_refuses_strokes_and_unlocking_paints_again() {
    let mut s = session();
    let layer = add(&mut s, "layer.add.paint", json!({"name": "Paint"}));
    execute(&mut s, "tool.material", &json!({"material": "copper"})).unwrap();
    let blank = base_colors(&s);

    let done = execute(&mut s, "layer.lock", &json!({"layer": layer.0})).unwrap();
    assert_eq!(done["locked"], true, "no value toggles");
    assert!(s.doc.project.layer(layer).unwrap().locked);
    let error = stroke(&mut s).expect_err("a stroke on a locked layer is refused");
    assert!(error.to_string().contains("locked"), "{error}");
    assert_eq!(base_colors(&s), blank, "nothing was painted");

    execute(&mut s, "layer.lock", &json!({"layer": layer.0, "locked": false})).unwrap();
    stroke(&mut s).unwrap();
    assert_ne!(base_colors(&s), blank, "the unlocked layer paints");
}

#[test]
fn a_locked_layer_refuses_strokes_on_its_mask_and_an_instance_of_it_too() {
    let mut s = session();
    let layer = add(&mut s, "layer.add.paint", json!({"name": "Paint"}));
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "layer.lock", &json!({"layer": layer.0, "locked": true})).unwrap();
    assert!(stroke(&mut s).is_err(), "the mask of a locked layer is locked");

    // An instance paints its source's mask
    execute(&mut s, "layer.lock", &json!({"layer": layer.0, "locked": false})).unwrap();
    let instance = add(&mut s, "layer.instance", json!({"layer": layer.0}));
    execute(&mut s, "layer.select", &json!({"layer": instance.0, "target": "mask"})).unwrap();
    execute(&mut s, "layer.lock", &json!({"layer": layer.0, "locked": true})).unwrap();
    assert!(stroke(&mut s).is_err(), "painting the instance paints the locked source");
}

#[test]
fn locking_is_one_undo_step_and_the_flag_survives_saving() {
    let mut s = session();
    let layer = add(&mut s, "layer.add.paint", json!({"name": "Paint"}));
    execute(&mut s, "layer.lock", &json!({"layer": layer.0})).unwrap();
    let json = serde_json::to_string(&s.doc.project).unwrap();
    let back: texturecraft_engine::core::Project = serde_json::from_str(&json).unwrap();
    assert!(back.layer(layer).unwrap().locked, "stored with the project");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(!s.doc.project.layer(layer).unwrap().locked);
    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    assert!(s.doc.project.layer(layer).unwrap().locked);
}

#[test]
fn a_gamma_corrected_mask_converts_its_grey_values_to_linear() {
    let mut s = session();
    let layer = add(&mut s, "layer.add.fill", json!({"color": [1.0, 1.0, 1.0]}));
    assert!(execute(&mut s, "mask.gammaCorrected", &json!({"layer": layer.0})).is_err(), "no mask yet");
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.fill", &json!({"value": 0.5})).unwrap();
    s.update();
    let plain = s.mask_bitmaps(layer, false).unwrap().remove(0);
    let level = plain.values.iter().copied().fold(0.0f32, f32::max);
    assert!((level - 0.5).abs() < 1e-4, "{level}");

    let done = execute(&mut s, "mask.gammaCorrected", &json!({"layer": layer.0})).unwrap();
    assert_eq!(done["enabled"], true);
    s.update();
    let corrected = s.mask_bitmaps(layer, false).unwrap().remove(0);
    let level = corrected.values.iter().copied().fold(0.0f32, f32::max);
    let expected = texturecraft_engine::texel::srgb_to_linear(0.5);
    assert!((level - expected).abs() < 1e-4, "{level} vs {expected}");

    // Stored with the project, one undo step
    let json = serde_json::to_string(&s.doc.project).unwrap();
    let back: texturecraft_engine::core::Project = serde_json::from_str(&json).unwrap();
    assert!(back.layer(layer).unwrap().mask.as_ref().unwrap().gamma_corrected);
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(!s.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().gamma_corrected);
    s.update();
    let undone = s.mask_bitmaps(layer, false).unwrap().remove(0);
    assert_eq!(undone.values, plain.values, "plain grey values again");
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-layer-channels-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn export_channels_writes_the_layer_alone_one_file_per_channel() {
    let mut s = session();
    let layer = add(&mut s, "layer.add.fill", json!({"name": "Red", "color": [1.0, 0.0, 0.0], "roughness": 0.25}));

    // A layer above it would show in a texture export but not in this one
    add(&mut s, "layer.add.fill", json!({"name": "Green", "color": [0.0, 1.0, 0.0]}));
    execute(&mut s, "layer.select", &json!({"layer": layer.0})).unwrap();
    let dir = temp_dir("alone");
    let done = execute(&mut s, "layer.exportChannels", &json!({"layer": layer.0, "path": dir.join("wall.png").display().to_string()})).unwrap();
    assert!(s.solo.is_none(), "the view is as it was");
    assert!(base_colors(&s).iter().all(|c| c[1] > 0.9 && c[0] < 0.1), "the viewport still shows the stack");

    let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let held = s.doc.project.layer(layer).unwrap().channels.len();
    assert_eq!(done["files"].as_array().unwrap().len(), held, "one file per channel the layer holds: {names:?}");
    assert!(names.contains(&"wall_baseColor.png".to_string()), "{names:?}");
    assert!(names.contains(&"wall_roughness.png".to_string()), "{names:?}");

    // The colour map is the red layer, not the green one on top
    let color = image::open(dir.join("wall_baseColor.png")).unwrap().into_rgb16();
    assert_eq!(color.dimensions(), (RESOLUTION, RESOLUTION));
    let g = &s.caches[0].gbuf;
    let covered = (0..g.width * g.height).find(|&i| g.tri[i as usize] != texturecraft_engine::geom::NO_TRI).unwrap();
    let pixel = color.get_pixel(covered % g.width, covered / g.width).0;
    assert!(pixel[0] > 60000 && pixel[1] < 1000 && pixel[2] < 1000, "{pixel:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_channels_needs_a_path_and_follows_the_extension() {
    let mut s = session();
    let layer = add(&mut s, "layer.add.fill", json!({"color": [0.5, 0.5, 0.5]}));
    assert!(execute(&mut s, "layer.exportChannels", &json!({"layer": layer.0})).is_err(), "a path is required");
    let dir = temp_dir("ext");
    assert!(execute(&mut s, "layer.exportChannels", &json!({"layer": layer.0, "path": dir.join("a.nope").display().to_string()})).is_err());
    execute(&mut s, "layer.exportChannels", &json!({"layer": layer.0, "path": dir.join("a.tga").display().to_string()})).unwrap();
    assert!(dir.join("a_baseColor.tga").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
