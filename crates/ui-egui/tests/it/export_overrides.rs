//! Export textures › a texture set's page (texturecraft #669): each set can override the output
//! template, file type, size and padding of the global settings.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{ConsoleLine, FileType, SetExport};

/// The Export textures window open on the first texture set's page
fn harness() -> Harness<'static, TextureCraftApp> {
    harness_with_session(texturecraft_engine::sample_session(256, 64).unwrap())
}

fn harness_with_session(session: texturecraft_engine::Session) -> Harness<'static, TextureCraftApp> {
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

/// Open the General Export Parameters menu
fn page_menu(h: &mut Harness<'_, TextureCraftApp>) {
    h.get_all_by_label(egui_phosphor::regular::LIST_CHECKS).nth(1).unwrap().click();
    h.run_steps(4);
}

/// The `row`th "Enable override" button (Output template, File type, Size, Padding)
fn toggle(h: &mut Harness<'_, TextureCraftApp>, row: usize) {
    h.get_all_by_label("Enable override").nth(row).unwrap().click();
    h.run_steps(4);
}

fn overrides(h: &Harness<'_, TextureCraftApp>) -> SetExport {
    h.state().ui.export.sets.get(&0).cloned().unwrap_or_default()
}

/// A scratch folder of its own for `name`
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tc-export-overrides-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Width of a PNG file from its header
fn png_width(path: &std::path::Path) -> u32 {
    let bytes = std::fs::read(path).unwrap();
    u32::from_be_bytes(bytes[16..20].try_into().unwrap())
}

#[test]
fn a_texture_set_page_shows_what_it_inherits() {
    let h = harness();
    assert_eq!(h.state().ui.export.page, Some(0));
    assert!(h.get_all_by_label("Enable override").count() >= 4, "four rows, then the output maps");
    let own = h.state().session.doc.project.texture_sets[0].resolution;
    for text in ["Document channels + Normal + AO (No Alpha)", "Based on output template", &format!("{own}x{own} (Texture Set size)"), "Dilation infinite"] {
        assert!(h.query_by_label(text).is_some(), "{text}");
    }
    assert!(h.query_by_label("*").is_none(), "no override yet");
}

#[test]
fn an_override_turns_the_row_into_a_list_and_marks_the_set() {
    let mut h = harness();
    let own = h.state().session.doc.project.texture_sets[0].resolution;
    toggle(&mut h, 2);
    assert_eq!(overrides(&h).size, Some(own), "the override starts at the inherited size");
    let star = h.get_by_label("*").rect().center();
    h.hover_at(star);
    h.run_steps(30);
    assert!(h.query_by_label("Override(s) active").is_some());

    let label = format!("{own}x{own} (Texture Set size)");
    h.get_all_by_role(egui::Role::ComboBox).find(|c| c.value().as_deref() == Some(label.as_str())).unwrap().click();
    h.run_steps(4);
    for size in ["128x128", "8192x8192"] {
        assert!(h.query_by_label(size).is_some(), "{size}");
    }
    click(&mut h, "128x128");
    assert_eq!(overrides(&h).size, Some(128));

    toggle(&mut h, 2);
    assert!(!h.state().ui.export.sets.contains_key(&0), "switching the only override off clears the mark");
}

#[test]
fn enabling_size_override_preserves_mixed_udim_tile_sizes() {
    use texturecraft_engine::{Session, execute, geom::primitives};

    let mut session = Session::new_project("Export", primitives::udim_plane(), "builtin:udim_plane", None, 256).unwrap();
    execute(&mut session, "textureSet.tile.resolution", &serde_json::json!({"tile": 1002, "resolution": 512})).unwrap();
    let mut h = harness_with_session(session);
    let expected = execute(&mut h.state_mut().session, "export.plan", &serde_json::json!({"ts": 0})).unwrap();

    toggle(&mut h, 2);
    let size = overrides(&h).size.unwrap();
    let actual = execute(&mut h.state_mut().session, "export.plan", &serde_json::json!({"ts": 0, "size": size})).unwrap();

    assert_eq!(size, 512);
    assert_eq!(actual, expected);
    assert!(h.get_all_by_role(egui::Role::ComboBox).any(|node| node.value().as_deref() == Some("512x512 (Texture Set size)")));
}

#[test]
fn the_header_menu_switches_every_override() {
    let mut h = harness();

    // The set list has the same menu button; the page header's is the second
    page_menu(&mut h);
    click(&mut h, "Override every item");
    let o = overrides(&h);
    assert!(o.preset.is_some() && o.file_type.is_some() && o.size.is_some() && o.padding.is_some(), "{o:?}");

    // The default template writes 8- and 16-bit files: the override keeps them until a type is picked
    assert_eq!(o.file_type.unwrap().format, "");
    page_menu(&mut h);
    click(&mut h, "Clear all overrides");
    assert!(h.state().ui.export.sets.is_empty());
}

#[test]
fn a_set_cannot_override_to_the_templates_the_reference_leaves_out() {
    let mut h = harness();
    toggle(&mut h, 0);
    h.get_all_by_role(egui::Role::ComboBox).find(|c| c.value().as_deref() == Some("Document channels + Normal + AO (No Alpha)")).unwrap().click();
    h.run_steps(4);
    assert!(h.query_by_label("2D View").is_some());
    assert!(h.query_by_label("glTF PBR Metal Roughness").is_none());
    assert!(h.query_by_label("Sketchfab").is_none());
}

#[test]
fn export_writes_each_set_with_its_overrides_and_cancel_drops_them() {
    let mut h = harness();
    let dir = folder("size");
    h.state_mut().ui.export.dir = dir.display().to_string();
    h.state_mut().ui.export.sets.insert(0, SetExport { size: Some(128), ..SetExport::default() });
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
    let finished = h.state().ui.export.console.iter().any(|l| matches!(l, ConsoleLine::Done(t) if t == "Texture export complete"));
    assert!(h.state().ui.export.job.is_none() && finished, "the export finished");
    let sets: Vec<String> = h.state().session.doc.project.texture_sets.iter().map(|t| format!("_{}_", t.name)).collect();
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
    let of = |set: &str| files.iter().filter(|f| f.file_name().unwrap().to_string_lossy().contains(set)).collect::<Vec<_>>();
    assert!(!of(&sets[0]).is_empty());
    for f in of(&sets[0]) {
        assert_eq!(png_width(f), 128, "{}", f.display());
    }

    // The other sets keep their own size
    for set in &sets[1..] {
        assert!(!of(set).is_empty(), "{set} was written");
        for f in of(set) {
            assert_eq!(png_width(f), 256, "{}", f.display());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    // The window stays open after an export; Save settings keeps the overrides
    click(&mut h, "Save settings");
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    h.state_mut().ui.export.sets.insert(0, SetExport { size: Some(512), ..SetExport::default() });
    h.run_steps(3);
    click(&mut h, "Cancel");
    assert_eq!(overrides(&h).size, Some(128), "Cancel goes back to the overrides the window opened with");
}

#[test]
fn overrides_stay_with_their_texture_set_and_their_project() {
    let mut h = harness();
    let names: Vec<String> = h.state().session.doc.project.texture_sets.iter().map(|t| t.name.clone()).collect();
    assert!(names.len() >= 2);
    let e = &mut h.state_mut().ui.export;
    e.sets.insert(1, SetExport { size: Some(128), ..SetExport::default() });
    e.skip = vec![1];
    click(&mut h, "Save settings");

    // The two texture sets trade names: the override and the tick follow the name
    for (ts, name) in [(0, "swap"), (1, names[0].as_str()), (0, names[1].as_str())] {
        texturecraft_engine::execute(&mut h.state_mut().session, "textureSet.rename", &serde_json::json!({"ts": ts, "name": name})).unwrap();
    }
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    assert_eq!(overrides(&h).size, Some(128), "set 0 now carries the name that had the override");
    assert_eq!(h.state().ui.export.skip, vec![0]);
    click(&mut h, "Save settings");

    // Another project starts without them
    h.state_mut().session = texturecraft_engine::sample_session(256, 64).unwrap();
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    assert!(h.state().ui.export.sets.is_empty() && h.state().ui.export.skip.is_empty());
}

#[test]
fn a_file_type_override_starts_from_the_type_of_the_template() {
    let mut h = harness();
    let library = folder("library");
    std::fs::create_dir_all(&library).unwrap();
    h.state_mut().session.load_export_presets(library.clone());
    let maps = serde_json::json!([{"file": "$textureSet_rough", "format": "Exr", "bit_depth": 32, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]}]);
    h.state_mut().run("export.preset.save", serde_json::json!({"preset": {"name": "One EXR", "maps": maps}})).unwrap();
    h.state_mut().ui.export.preset = "One EXR".into();
    h.run_steps(4);
    assert!(h.state().ui.export.format.is_empty(), "Based on output template");
    toggle(&mut h, 1);
    assert_eq!(overrides(&h).file_type, Some(FileType { format: "exr".into(), bit_depth: 32, dithering: false }), "not png");
    let _ = std::fs::remove_dir_all(&library);
}

#[test]
fn cancel_keeps_each_tick_with_its_texture_set_after_they_trade_names() {
    let mut h = harness();
    let names: Vec<String> = h.state().session.doc.project.texture_sets.iter().map(|t| t.name.clone()).collect();
    h.state_mut().ui.export.skip = vec![1];
    click(&mut h, "Save settings");
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);

    // While the window is open the two texture sets trade names: the tick follows the name
    for (ts, name) in [(0, "swap"), (1, names[0].as_str()), (0, names[1].as_str())] {
        texturecraft_engine::execute(&mut h.state_mut().session, "textureSet.rename", &serde_json::json!({"ts": ts, "name": name})).unwrap();
    }
    h.run_steps(4);
    assert_eq!(h.state().ui.export.skip, vec![0]);
    click(&mut h, "Cancel");
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    assert_eq!(h.state().ui.export.skip, vec![0], "Cancel and reopening keep the tick off the set named {}", names[1]);
}

#[test]
fn reset_all_then_cancel_restores_the_ticks_and_overrides() {
    let mut h = harness();
    let e = &mut h.state_mut().ui.export;
    e.skip = vec![1];
    e.sets.insert(1, SetExport { size: Some(128), ..SetExport::default() });
    click(&mut h, "Save settings");
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    click(&mut h, "Reset all");
    assert!(h.state().ui.export.skip.is_empty() && h.state().ui.export.sets.is_empty(), "Reset all clears them");
    click(&mut h, "Cancel");
    assert_eq!(h.state().ui.export.skip, vec![1]);
    assert_eq!(h.state().ui.export.sets.get(&1).and_then(|o| o.size), Some(128), "Cancel brings the override back");
}

#[test]
fn ticks_saved_before_settings_named_their_project_are_kept() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    // Older settings: a tick off, no owner
    let e = &mut h.state_mut().ui.export;
    (e.skip, e.owner, e.owner_id) = (vec![1], Vec::new(), None);
    e.sets.insert(1, SetExport { size: Some(128), ..SetExport::default() });
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    assert_eq!(h.state().ui.export.skip, vec![1]);
    assert_eq!(h.state().ui.export.sets.get(&1).and_then(|o| o.size), Some(128));
}

#[test]
fn an_empty_file_type_override_keeps_the_template_types_under_a_global_one() {
    let mut h = harness();
    assert!(h.query_all_by_label("png   16 bits").next().is_some(), "the default template has a 16-bit map");

    // The override kept the template's types (a mixed template); then a global type is chosen
    let keep = FileType { format: String::new(), bit_depth: 0, dithering: false };
    h.state_mut().ui.export.sets.insert(0, SetExport { file_type: Some(keep), ..SetExport::default() });
    let e = &mut h.state_mut().ui.export;
    (e.format, e.bit_depth) = ("png".into(), 8);
    h.run_steps(4);
    assert!(h.query_all_by_label("png   16 bits").next().is_some(), "the set still writes the template's types");
}
