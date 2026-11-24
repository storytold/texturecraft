//! Export textures › a texture set's Output maps (texturecraft #670): a tick per map, its file
//! name pattern, and its own file type and bit depth.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{FileType, MapExport, SetExport};

/// The Export textures window open on the first texture set's page
fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    let name = h.state().session.doc.project.texture_sets[0].name.clone();
    h.get_all_by_label(&name).last().unwrap().click();
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_all_by_label(label).last().unwrap_or_else(|| panic!("no {label}")).click();
    h.run_steps(4);
}

/// The number in "N textures selected for export"
fn selected(h: &Harness<'_, TextureCraftApp>) -> usize {
    (0..500).find(|n| h.query_by_label(&format!("{n} textures selected for export")).is_some()).expect("the footer count")
}

fn maps(h: &Harness<'_, TextureCraftApp>) -> std::collections::BTreeMap<usize, MapExport> {
    h.state().ui.export.sets.get(&0).map(|o| o.maps.clone()).unwrap_or_default()
}

/// The first texture set's planned files with the window's settings
fn plan(h: &mut Harness<'_, TextureCraftApp>) -> Vec<serde_json::Value> {
    let preset = h.state().ui.export.preset.clone();
    let v = texturecraft_engine::execute(&mut h.state_mut().session, "export.plan", &serde_json::json!({"preset": preset, "ts": 0})).unwrap();
    v.as_array().unwrap().clone()
}

#[test]
fn the_rows_show_each_map_with_its_pattern_and_file_type() {
    let mut h = harness();
    let rows = plan(&mut h).len();
    assert!(h.query_by_label("Output maps").is_some());

    // Four rows of General Export Parameters, then one per map
    assert_eq!(h.get_all_by_label("Enable override").count(), 4 + rows);
    assert!(h.query_all_by_label("png   8 bits").next().is_some());
    assert!(h.query_all_by_label("png   16 bits").next().is_some(), "height is 16-bit in the default template");

    // A pattern longer than the column is cut at the front, the whole one is its tooltip
    let preset = texturecraft_engine::formats::builtin_preset(&h.state().ui.export.preset).unwrap();
    let long =
        plan(&mut h).iter().map(|f| preset.maps[f["map"].as_u64().unwrap() as usize].file.clone()).find(|p| p.chars().count() > 26).expect("a long pattern");
    let cut: String = format!("…{}", long.chars().skip(long.chars().count() - 25).collect::<String>());
    assert!(h.query_by_label(&long).is_none(), "{long} is cut");
    let at = h.get_all_by_label(&cut).next().unwrap().rect().center();
    h.hover_at(at);
    h.run_steps(30);
    assert!(h.query_by_label(&long).is_some(), "the tooltip has the whole {long}");
}

#[test]
fn unticking_a_map_leaves_it_out_and_the_menu_ticks_all() {
    let mut h = harness();
    let before = selected(&h);
    let rows = plan(&mut h).len();

    // The window comes last, and its map rows last in it
    let boxes = h.get_all_by_role(egui::Role::CheckBox).count();
    h.get_all_by_role(egui::Role::CheckBox).nth(boxes - rows).unwrap().click();
    h.run_steps(4);
    assert_eq!(selected(&h), before - 1);
    assert_eq!(maps(&h).values().filter(|m| !m.enabled).count(), 1);
    assert!(h.query_by_label("*").is_none(), "a left-out map is not an override");

    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Uncheck all");
    assert_eq!(selected(&h), before - rows);
    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Invert all");
    assert_eq!(selected(&h), before);
    assert!(!h.state().ui.export.sets.contains_key(&0), "nothing left to remember");

    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Uncheck all");
    click(&mut h, "Include all");
    assert_eq!(selected(&h), before, "Include all ticks the output maps too");
}

#[test]
fn a_map_override_sets_its_own_file_type_and_marks_the_set() {
    let mut h = harness();
    let rows = plan(&mut h).len();
    h.get_all_by_label("Enable override").nth(4).unwrap().click();
    h.run_steps(4);
    let m = maps(&h);
    let (_, first) = m.iter().next().unwrap();
    assert!(first.file_type.is_some(), "{m:?}");
    assert!(h.query_by_label("*").is_some(), "the set is marked");

    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Override every item");
    assert_eq!(maps(&h).values().filter(|m| m.file_type.is_some()).count(), rows);
    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Clear all overrides");
    assert!(maps(&h).is_empty());
}

#[test]
fn export_writes_a_map_with_its_own_file_type() {
    let mut h = harness();
    let first = plan(&mut h)[0]["map"].as_u64().unwrap() as usize;
    let dir = std::env::temp_dir().join(format!("tc-export-map-overrides-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let template = h.state().ui.export.preset.clone();
    let mut o = SetExport { maps_template: template, ..SetExport::default() };
    o.maps.insert(first, MapExport { enabled: true, file_type: Some(FileType { format: "targa".into(), bit_depth: 8, dithering: false }) });
    let e = &mut h.state_mut().ui.export;
    e.dir = dir.display().to_string();
    e.sets.insert(0, o);
    e.skip = (1..32).collect();
    h.run_steps(3);
    click(&mut h, "Export");

    // The export writes a texture set per frame
    for _ in 0..64 {
        if h.state().ui.export.job.is_none() {
            break;
        }

        h.run_steps(1);
    }
    h.run_steps(2);
    let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.iter().filter(|n| n.ends_with(".tga")).count(), 1, "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_map_the_template_no_longer_has_is_ignored() {
    let mut h = harness();
    let before = selected(&h);
    let template = h.state().ui.export.preset.clone();
    let mut o = SetExport { maps_template: template, ..SetExport::default() };
    o.maps.insert(999, MapExport { enabled: false, file_type: None });
    h.state_mut().ui.export.sets.insert(0, o);
    h.run_steps(3);
    assert_eq!(selected(&h), before, "the export still plans every file");
}
