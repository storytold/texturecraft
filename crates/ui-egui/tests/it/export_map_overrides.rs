//! Export textures › a texture set's Output maps (texturecraft #670): a tick per map, its file
//! name pattern, and its own file type and bit depth.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{FileType, MapExport, SetExport};

use super::temp_dir::TempDir;

/// The Export textures window open on the first texture set's page
fn harness() -> Harness<'static, TextureCraftApp> {
    harness_at(egui::vec2(1600.0, 960.0))
}

fn harness_at(size: egui::Vec2) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder()
        .with_size(size)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    assert_eq!(h.get_all_by_label_contains("Enable override for ").count(), 4 + rows);
    assert!(h.query_all_by_label("png   8 bits").next().is_some());
    assert!(h.query_all_by_label("png   16 bits").next().is_some(), "height is 16-bit in the default template");

    // A pattern longer than the column (the packed template's) is cut at the front, the whole one is its tooltip
    h.state_mut().ui.export.preset = "Unreal Engine (Packed)".into();
    h.run_steps(4);
    let preset = texturecraft_engine::formats::builtin_preset(&h.state().ui.export.preset).unwrap();
    let long = preset.maps.iter().map(|map| map.file.clone()).max_by_key(|file| file.chars().count()).unwrap();
    let cut = h
        .query_all_by_label_contains("…")
        .find_map(|node| node.accesskit_node().value().filter(|text| long.ends_with(text.trim_start_matches('…'))))
        .expect("the pattern's end, cut at the front");
    assert!(h.query_by_label(&long).is_none(), "{long} is cut");
    let at = h.get_all_by_role_and_label(egui::Role::Label, &cut).next().unwrap().rect().center();
    h.hover_at(at);
    h.run_steps(30);

    assert!(h.query_by_label(&long).is_some(), "the tooltip has the whole {long}");
}

#[test]
fn feedback_psd_preflight_keeps_map_controls_available() {
    let mut h = harness();
    let available = plan(&mut h).len();
    assert!(available >= 3);
    let preset = h.state().ui.export.preset.clone();
    let settings = &mut h.state_mut().ui.export;
    settings.format = "psd".into();
    settings.bit_depth = 16;
    settings.size = 8192;
    let params = serde_json::json!({"ts": 0, "preset": preset, "format": "psd", "bitDepth": 16, "size": 8192});

    let error = texturecraft_engine::execute(&mut h.state_mut().session, "export.plan", &params).unwrap_err();
    assert!(error.to_string().contains("2 GiB"), "{error}");
    h.run_steps(4);

    assert_eq!(h.get_all_by_label_contains("Enable override for map ").count(), available);
    let size = h.get_all_by_role(egui::Role::ComboBox).find(|combo| combo.value().as_deref() == Some("Export size")).expect("a map's size list");
    size.click();
    h.run_steps(4);
    click(&mut h, "128x128");

    assert!(maps(&h).values().any(|map| map.size == Some(128)));
    assert_eq!(h.state().ui.export.size, 8192);
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

    click(&mut h, "Output maps menu");
    click(&mut h, "Uncheck all");

    assert_eq!(selected(&h), before - rows);
    click(&mut h, "Output maps menu");
    click(&mut h, "Invert all");

    assert_eq!(selected(&h), before);
    assert!(!h.state().ui.export.sets.contains_key(&0), "nothing left to remember");

    click(&mut h, "Output maps menu");
    click(&mut h, "Uncheck all");
    click(&mut h, "Include all");

    assert_eq!(selected(&h), before, "Include all ticks the output maps too");
}

#[test]
fn a_map_override_sets_its_own_file_type_and_marks_the_set() {
    let mut h = harness();
    let rows = plan(&mut h).len();
    h.get_all_by_label_contains("Enable override for ").nth(4).unwrap().click();
    h.run_steps(4);
    let map_overrides = maps(&h);
    let (_, first) = map_overrides.iter().next().unwrap();

    assert!(first.file_type.is_some(), "{map_overrides:?}");
    assert!(h.query_by_label("*").is_some(), "the set is marked");

    click(&mut h, "Output maps menu");
    click(&mut h, "Override every item");

    assert_eq!(maps(&h).values().filter(|map_overrides| map_overrides.file_type.is_some()).count(), rows);
    click(&mut h, "Output maps menu");
    click(&mut h, "Clear all overrides");

    assert!(maps(&h).is_empty());
}

#[test]
fn export_writes_a_map_with_its_own_file_type() {
    let mut h = harness();
    let first = plan(&mut h)[0]["map"].as_u64().unwrap() as usize;
    let dir = TempDir::new("export-map-overrides");
    let template = h.state().ui.export.preset.clone();
    let mut overrides = SetExport { maps_template: template, ..SetExport::default() };
    overrides.maps.insert(first, MapExport { enabled: true, file_type: Some(FileType { format: "targa".into(), bit_depth: 8, dithering: false }), size: None });
    let export = &mut h.state_mut().ui.export;
    export.dir = dir.display().to_string();
    export.sets.insert(0, overrides);
    export.skip = (1..32).collect();
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
    let names: Vec<String> = std::fs::read_dir(&*dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();

    assert_eq!(names.iter().filter(|n| n.ends_with(".tga")).count(), 1, "{names:?}");
}

#[test]
fn a_map_the_template_no_longer_has_is_ignored() {
    let mut h = harness();
    let before = selected(&h);
    let template = h.state().ui.export.preset.clone();
    let mut overrides = SetExport { maps_template: template, ..SetExport::default() };
    overrides.maps.insert(999, MapExport { enabled: false, file_type: None, size: None });
    h.state_mut().ui.export.sets.insert(0, overrides);
    h.run_steps(3);

    assert_eq!(selected(&h), before, "the export still plans every file");
}

#[test]
fn each_row_names_its_own_map() {
    let h = harness();

    // The default template names its maps with $channel: each row shows the map it writes
    assert!(h.query_all_by_label_contains("$channel").next().is_none(), "the channel variable is filled in");
    for id in ["baseColor", "metallic", "roughness"] {
        assert!(h.query_all_by_label_contains(&format!("_{id}")).next().is_some(), "a row for {id}");
    }
}

#[test]
fn texture_set_map_sizes_are_reachable_on_a_narrow_screen() {
    let mut h = harness_at(egui::vec2(800.0, 480.0));
    let header = h.get_by_label("General Export Parameters").rect().center();
    let size_rect = |h: &Harness<'_, TextureCraftApp>| {
        h.get_all_by_role(egui::Role::ComboBox).find(|combo| combo.value().as_deref() == Some("Export size")).expect("a map size list").rect()
    };
    let before = size_rect(&h);

    h.hover_at(header);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-500.0, -200.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(12);

    let after = size_rect(&h);

    assert!(after.left() < before.left(), "{before:?} → {after:?}");
    assert!(h.ctx.content_rect().contains_rect(after), "{after:?}");
    h.get_all_by_role(egui::Role::ComboBox).find(|combo| combo.value().as_deref() == Some("Export size")).unwrap().click();
    h.run_steps(4);
    click(&mut h, "128x128");

    assert!(maps(&h).values().any(|map| map.size == Some(128)));
}

#[test]
fn a_map_picks_its_own_size_and_the_export_writes_it_at_that_size() {
    let mut h = harness();
    let expected = plan(&mut h);
    let first = expected[0]["map"].as_u64().unwrap() as usize;
    let combo = h.get_all_by_role(egui::Role::ComboBox).find(|combo| combo.value().as_deref() == Some("Export size")).expect("a map's size list");
    combo.click();
    h.run_steps(4);
    click(&mut h, "128x128");

    assert_eq!(maps(&h).get(&first).and_then(|map| map.size), Some(128), "{:?}", maps(&h));
    assert!(h.query_by_label("*").is_some(), "the set is marked");

    let dir = TempDir::new("export-map-sizes");
    let set_count = h.state().session.set_count();
    let export = &mut h.state_mut().ui.export;
    export.dir = dir.display().to_string();
    export.skip = (1..set_count).collect();
    h.run_steps(3);
    click(&mut h, "Export");
    for _ in 0..64 {
        if h.state().ui.export.job.is_none() {
            break;
        }

        h.run_steps(1);
    }
    h.run_steps(2);

    assert_eq!(std::fs::read_dir(&*dir).unwrap().count(), expected.len());
    for file in expected {
        let path = dir.join(file["file"].as_str().unwrap());
        let side = if file["map"].as_u64().unwrap() as usize == first { 128 } else { 256 };

        assert_eq!(image::image_dimensions(&path).unwrap(), (side, side), "{}", path.display());
    }
}

#[test]
fn rows_with_a_one_depth_file_type_each_have_their_own_bit_depth_list() {
    let mut h = harness();
    let rows: Vec<usize> = plan(&mut h).iter().take(2).map(|file| file["map"].as_u64().unwrap() as usize).collect();
    let template = h.state().ui.export.preset.clone();
    let mut overrides = SetExport { maps_template: template, ..SetExport::default() };
    for map in &rows {
        overrides
            .maps
            .insert(*map, MapExport { enabled: true, file_type: Some(FileType { format: "hdr".into(), bit_depth: 32, dithering: false }), size: None });
    }
    h.state_mut().ui.export.sets.insert(0, overrides);
    h.run_steps(4);

    let greyed: Vec<_> =
        h.get_all_by_role(egui::Role::ComboBox).filter(|node| node.value().as_deref() == Some("32f bits")).map(|node| node.accesskit_node().id()).collect();

    assert_eq!(rows.len(), 2);
    assert_eq!(greyed.len(), 2, "each row's greyed bit depth list is its own node");
    assert_ne!(greyed[0], greyed[1]);
}
