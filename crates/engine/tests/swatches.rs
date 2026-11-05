//! Colour swatches: commands, the project file, recent colours and palette files.

use serde_json::{Value, json};
use texturecraft_engine::core::swatch::{self, MAX_RECENT, MAX_SWATCHES};
use texturecraft_engine::core::{Channel, LayerKind, Source};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 24, 12), "builtin:sphere", None, 256).unwrap();
    s.viewport = Vec2::new(200.0, 200.0);
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    execute(s, id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v[key].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn project_swatches_survive_save_and_load_and_old_projects_load() {
    let mut s = session();
    run(&mut s, "swatch.add", json!({"color": "#336699", "name": "Denim"}));
    run(&mut s, "swatch.add", json!({"color": [0.5, 0.25, 0.125]}));
    let bytes = texturecraft_formats::save_project_bytes(&s.project_file()).unwrap();
    let back = Session::from_project_bytes(&bytes).unwrap();
    assert_eq!(back.doc.project.swatches, s.doc.project.swatches);
    assert_eq!(swatch::to_hex(back.doc.project.swatches[0].rgb), "#336699");
    assert_eq!(back.doc.project.swatches[1].rgb, [0.5, 0.25, 0.125]);

    // A project without swatches (older file, or none made) writes no `swatches` key and reads back empty.
    let plain = session();
    let json = serde_json::to_value(&plain.doc.project).unwrap();
    assert!(json.get("swatches").is_none());
    let mut old = json;
    old.as_object_mut().unwrap().remove("swatches");
    let p: texturecraft_engine::core::Project = serde_json::from_value(old).unwrap();
    assert!(p.swatches.is_empty());
}

#[test]
fn add_apply_rename_replace_move_remove() {
    let mut s = session();
    for (n, c) in [("A", "#ff0000"), ("B", "#00ff00"), ("C", "#0000ff")] {
        run(&mut s, "swatch.add", json!({"color": c, "name": n}));
    }
    // Apply by name sets the brush colour (linear) and turns the channel on.
    run(&mut s, "swatch.apply", json!({"name": "b"}));
    assert_eq!(s.tool.values[&Channel::BaseColor], [0.0, 1.0, 0.0]);
    // Rename, replace (default: the brush colour), reorder, delete.
    run(&mut s, "swatch.rename", json!({"index": 0, "newName": "Red\nPaint"}));
    run(&mut s, "swatch.replace", json!({"index": 2}));
    let l = run(&mut s, "swatch.move", json!({"from": 2, "to": 0}));
    assert_eq!(names(&l, "project"), ["C", "Red Paint", "B"], "replace keeps the name");
    assert_eq!(l["project"][0]["hex"], "#00ff00", "replaced with the brush colour");
    run(&mut s, "swatch.remove", json!({"name": "red paint"}));
    assert_eq!(s.doc.project.swatches.len(), 2);
    // Every project change is one undo step.
    run(&mut s, "edit.undo", Value::Null);
    assert_eq!(s.doc.project.swatches.len(), 3);
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = session();
    run(&mut s, "swatch.add", json!({"color": [0.1, 0.2, 0.3]}));
    for (id, p) in [
        ("swatch.apply", json!({"index": 9})),
        ("swatch.apply", json!({"index": -1})),
        ("swatch.apply", json!({})),
        ("swatch.apply", json!({"index": 0, "target": "moon"})),
        ("swatch.apply", json!({"recent": 99})),
        ("swatch.add", json!({"color": "nope"})),
        ("swatch.add", json!({"scope": "everywhere"})),
        ("swatch.move", json!({"from": 5, "to": 0})),
        ("swatch.rename", json!({"index": 0})),
        ("swatch.remove", json!({"name": "ghost"})),
        ("swatch.import", json!({"text": "not a palette"})),
        ("swatch.import", json!({"text": "#fff", "mode": "x"})),
        ("swatch.export", json!({"format": "pdf"})),
    ] {
        assert!(execute(&mut s, id, &p).is_err(), "{id} {p}");
    }
    assert_eq!(s.doc.project.swatches.len(), 1);
    // NaN and out-of-range colours are clamped.
    run(&mut s, "swatch.add", json!({"color": [5.0, -2.0, 0.5]}));
    assert_eq!(s.doc.project.swatches[1].rgb, [1.0, 0.0, 0.5]);
}

#[test]
fn global_swatches_are_separate_and_follow_the_user_to_the_next_project() {
    let mut s = session();
    run(&mut s, "swatch.add", json!({"scope": "global", "color": "#ffffff", "name": "Paper"}));
    assert!(s.doc.project.swatches.is_empty());
    assert!(!s.modified, "global swatches are not part of the project");
    let mut next = session();
    s.switch_to(std::mem::replace(&mut next, session()));
    let l = run(&mut s, "swatch.list", Value::Null);
    assert_eq!(names(&l, "global"), ["Paper"]);
    assert!(l["project"].as_array().unwrap().is_empty());
}

#[test]
fn recent_colours_follow_what_was_painted_with() {
    let mut s = session();
    run(&mut s, "tool.paint", Value::Null);
    for i in 0..10u8 {
        run(&mut s, "tool.material", json!({"channel": "baseColor", "color": format!("#{:02x}0000", i * 10 + 10)}));
        run(&mut s, "tool.stroke", json!({"points": [[100, 100]]}));
    }
    let l = run(&mut s, "swatch.list", Value::Null);
    let recent: Vec<&str> = l["recent"].as_array().unwrap().iter().map(|c| c["hex"].as_str().unwrap()).collect();
    assert_eq!(recent.len(), MAX_RECENT);
    assert_eq!(recent[0], "#640000");
    // Picking a colour in the editor is not "using" it: only painting is. Eraser and mask strokes add nothing.
    run(&mut s, "tool.material", json!({"channel": "baseColor", "color": "#123456"}));
    run(&mut s, "tool.eraser", Value::Null);
    run(&mut s, "tool.stroke", json!({"points": [[100, 100]]}));
    let l2 = run(&mut s, "swatch.list", Value::Null);
    assert_eq!(l2["recent"], l["recent"]);
    // Applying a recent colour brings it to the front.
    run(&mut s, "swatch.apply", json!({"recent": 3}));
    let l3 = run(&mut s, "swatch.list", Value::Null);
    assert_eq!(l3["recent"][0]["hex"], l["recent"][3]["hex"]);
    assert_eq!(l3["recent"].as_array().unwrap().len(), MAX_RECENT);
}

#[test]
fn apply_to_the_selected_fill_layer() {
    let mut s = session();
    run(&mut s, "layer.add.fill", json!({"color": [0.9, 0.9, 0.9]}));
    run(&mut s, "swatch.add", json!({"color": "#ff8000", "name": "Orange"}));
    let out = run(&mut s, "swatch.apply", json!({"index": 0, "target": "layer"}));
    assert_eq!(out["hex"], "#ff8000");
    let id = s.selected_layer().unwrap();
    let LayerKind::Fill(f) = &s.doc.project.layer(id).unwrap().kind else { panic!("not a fill layer") };
    let Source::Uniform(c) = f.source(Channel::BaseColor) else { panic!("not uniform") };
    assert_eq!(swatch::to_hex(c), "#ff8000");
    // The brush colour is untouched by a layer apply.
    assert_ne!(swatch::to_hex(s.tool.values[&Channel::BaseColor]), "#ff8000");
}

#[test]
fn engine_and_swatch_colour_conversions_agree_exactly() {
    for v in 0..=255u32 {
        let x = v as f32 / 255.0;
        assert_eq!(swatch::srgb_to_linear(x), texturecraft_engine::texel::srgb_to_linear(x), "{v}");
        let l = swatch::srgb_to_linear(x);
        assert_eq!(swatch::linear_to_srgb(l), texturecraft_engine::texel::linear_to_srgb(l), "{v}");
        assert_eq!((texturecraft_engine::texel::linear_to_srgb(l) * 255.0 + 0.5) as u8, v as u8);
    }
}

#[test]
fn palettes_import_and_export_both_formats() {
    let mut s = session();
    let gpl = "GIMP Palette\nName: Test\n#\n255 0 0 Red\n0 0 255 Blue\n999 0 0 Hostile\nxx\n";
    let r = run(&mut s, "swatch.import", json!({"text": gpl}));
    assert_eq!((r["added"].as_u64(), r["skipped"].as_u64()), (Some(2), Some(2)));
    let r = run(&mut s, "swatch.import", json!({"text": "#00ff00 Green\n\n; c\n#f80\n", "mode": "append"}));
    assert_eq!(r["count"], 4);
    let e = run(&mut s, "swatch.export", json!({"format": "hex"}));
    assert_eq!(e["text"], "#ff0000 Red\n#0000ff Blue\n#00ff00 Green\n#ff8800\n");
    let e = run(&mut s, "swatch.export", json!({}));
    assert!(e["text"].as_str().unwrap().starts_with("GIMP Palette\nName: t\n"));
    // Export then import gives the same colours (replace).
    let text = e["text"].as_str().unwrap().to_string();
    let before = s.doc.project.swatches.clone();
    run(&mut s, "swatch.import", json!({"text": text, "mode": "replace"}));
    assert_eq!(s.doc.project.swatches.len(), before.len());
    for (a, b) in s.doc.project.swatches.iter().zip(&before) {
        assert!(swatch::same_colour(a.rgb, b.rgb));
    }
}

#[test]
fn palette_files_on_disk_and_hostile_ones() {
    let dir = std::env::temp_dir().join(format!("tc-swatches-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = session();
    run(&mut s, "swatch.add", json!({"color": "#102030", "name": "Slate"}));
    let gpl = dir.join("p.gpl");
    let hex = dir.join("p.txt");
    run(&mut s, "swatch.export", json!({"path": gpl.to_str().unwrap()}));
    run(&mut s, "swatch.export", json!({"path": hex.to_str().unwrap()}));
    assert_eq!(std::fs::read_to_string(&hex).unwrap(), "#102030 Slate\n");
    run(&mut s, "swatch.import", json!({"path": gpl.to_str().unwrap(), "scope": "global"}));
    run(&mut s, "swatch.import", json!({"path": hex.to_str().unwrap(), "scope": "global"}));
    assert_eq!(s.swatches.global.len(), 2);
    // Missing, binary and oversized files are errors, and change nothing.
    assert!(execute(&mut s, "swatch.import", &json!({"path": dir.join("nope.gpl").to_str().unwrap()})).is_err());
    let bin = dir.join("bin.gpl");
    std::fs::write(&bin, [0u8, 159, 146, 150, 255, 0, 1]).unwrap();
    assert!(execute(&mut s, "swatch.import", &json!({"path": bin.to_str().unwrap()})).is_err());
    let big = dir.join("big.gpl");
    std::fs::write(&big, "GIMP Palette\n".to_string() + &"0 0 0 x\n".repeat(200_000)).unwrap();
    assert!(execute(&mut s, "swatch.import", &json!({"path": big.to_str().unwrap()})).is_err());
    assert_eq!(s.swatches.global.len(), 2);
    // A palette can never grow a list past the cap.
    let many = "#000000\n".repeat(MAX_SWATCHES + 10);
    run(&mut s, "swatch.import", json!({"text": many, "scope": "global"}));
    assert_eq!(s.swatches.global.len(), MAX_SWATCHES);
    assert!(execute(&mut s, "swatch.add", &json!({"scope": "global"})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
