//! Export textures › exporting (texturecraft #673): LIST OF EXPORTS as a tree, the console, the
//! progress bar with Cancel Export, and the window staying open.

use egui_kittest::Harness;
use egui_kittest::kittest::NodeT;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{ConsoleLine, ExportJob};

/// The Export textures window writing into a scratch folder of its own for `name`
fn harness(name: &str) -> (Harness<'static, TextureCraftApp>, std::path::PathBuf) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    let dir = std::env::temp_dir().join(format!("tc-export-progress-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    h.state_mut().ui.export.dir = dir.display().to_string();
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    (h, dir)
}

fn click(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_all_by_label(label).last().unwrap_or_else(|| panic!("no {label}")).click();
    h.run_steps(1);
}

fn console(h: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    h.state()
        .ui
        .export
        .console
        .iter()
        .map(|l| match l {
            ConsoleLine::Step(t) | ConsoleLine::Done(t) => t.clone(),
            ConsoleLine::Set(name) => format!("Texture set {name}"),
        })
        .collect()
}

#[test]
fn export_shows_progress_logs_the_steps_and_keeps_the_window_open() {
    let (mut h, dir) = harness("run");
    let sets: Vec<String> = h.state().session.doc.project.texture_sets.iter().map(|t| t.name.clone()).collect();
    click(&mut h, "Export");
    assert_eq!(h.state().ui.export.tab, 2, "Export switches to LIST OF EXPORTS");
    h.run_steps(1);
    assert!(h.query_by_label("Cancel Export").is_some());
    assert_eq!(h.get_by_role(egui::Role::ProgressIndicator).accesskit_node().live(), egui::accesskit::Live::Polite);
    assert!(h.query_by_label("Cancel").is_none(), "the progress bar replaces the buttons");
    assert_eq!(console(&h).first().map(String::as_str), Some("Preparing texture export"));

    for _ in 0..64 {
        if h.state().ui.export.job.is_none() {
            break;
        }

        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(h.state().ui.windows.export, "the window stays open");
    assert!(h.query_by_label("Cancel").is_some() && h.query_by_label("Cancel Export").is_none());
    let lines = console(&h);
    let set = format!("Texture set {}", sets[0]);
    let start = lines.iter().position(|l| *l == set).unwrap();
    assert_eq!(&lines[start..start + 5], [set.clone(), set.clone(), "Rendering the selected maps".into(), set.clone(), "Saving texture files".into()]);
    assert!(lines[start + 6].starts_with("Saved texture "), "{lines:?}");
    let written: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let finished = lines.iter().position(|l| l == "Texture export complete").unwrap();
    assert_eq!(&lines[finished - 3..finished], ["Finished writing this texture set", "All selected texture sets processed", "Collecting the exported files"]);
    assert_eq!(lines[finished + 1], "Files saved");
    assert_eq!(lines.len() - finished - 2, written.len(), "every written file is listed");
    assert!(matches!(h.state().ui.export.console.last(), Some(ConsoleLine::Done(_))), "in green");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A long export: the texture sets several times over, into `dir`
fn long_export(h: &mut Harness<'_, TextureCraftApp>, dir: &std::path::Path) {
    let sets: Vec<usize> = (0..h.state().session.set_count()).collect();
    let params = json!({"dir": dir.display().to_string()});
    let pending: Vec<(usize, serde_json::Value)> = sets.iter().copied().cycle().take(sets.len() * 8).map(|ts| (ts, params.clone())).collect();
    let e = &mut h.state_mut().ui.export;
    e.tab = 2;
    e.job = Some(ExportJob { total: pending.len(), pending, ..ExportJob::default() });
}

/// How many texture sets the session wrote so far
fn exports(h: &Harness<'_, TextureCraftApp>) -> usize {
    h.state().session.log.iter().filter(|l| l.starts_with("Exported ")).count()
}

/// Run frames until the export ends (or a bound)
fn finish(h: &mut Harness<'_, TextureCraftApp>) {
    for _ in 0..64 {
        if h.state().ui.export.job.is_none() {
            break;
        }

        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn cancel_export_stops_the_export() {
    let (mut h, dir) = harness("cancel");
    long_export(&mut h, &dir);
    h.run_steps(2);
    assert!(h.query_by_label("Cancel Export").is_some());
    assert!(h.query_all_by_label_contains("%").next().is_some(), "the bar shows the percent");
    click(&mut h, "Cancel Export");
    h.run_steps(3);
    assert!(h.state().ui.export.job.is_none());
    assert!(h.state().ui.windows.export);
    let lines = console(&h);
    assert!(!lines.iter().any(|l| l == "Collecting the exported files" || l == "Texture export complete"), "the export did not finish: {lines:?}");
    assert!(h.query_by_label("Cancel").is_some(), "the buttons are back");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_list_is_a_tree_per_texture_set_with_the_console_beside_it() {
    let (mut h, dir) = harness("tree");
    h.state_mut().ui.export.tab = 2;
    h.run_steps(4);
    let set = h.state().session.doc.project.texture_sets[0].name.clone();
    assert!(
        h.query_by_role_and_label(egui::Role::DisclosureTriangle, &set).is_some(),
        "the export tree has a node for the set (the dock list row carries the same name)"
    );
    assert!(h.query_all_by_label("256x256 (Texture Set size)").next().is_some());
    assert!(h.query_all_by_label("png").next().is_some() && h.query_all_by_label("16 bits").next().is_some());
    assert!(h.query_by_label("Include all").is_none() && h.query_by_label("Reset all").is_none());
    assert!(h.query_by_label("Console").is_some());

    // A row's tooltip is the full path
    let preset = h.state().ui.export.preset.clone();
    let plan = texturecraft_engine::execute(&mut h.state_mut().session, "export.plan", &serde_json::json!({"preset": preset, "ts": 0})).unwrap();
    let file = plan[0]["file"].as_str().unwrap().to_string();
    let at = h.get_by_label(&file).rect().center();
    h.hover_at(at);
    h.run_steps(30);

    assert!(h.query_by_label(&dir.join(&file).display().to_string()).is_some());

    // The strip at the console's edge hides it, then shows it again
    h.get_by_role_and_label(egui::Role::Button, "Hide Console").click();
    h.run_steps(4);

    assert!(h.state().ui.export.console_hidden && h.query_by_label("Console").is_none());
    h.get_by_role_and_label(egui::Role::Button, "Show Console").click();
    h.run_steps(4);

    assert!(!h.state().ui.export.console_hidden);
}

#[test]
fn long_filenames_leave_the_export_details_reachable_by_scrolling() {
    let (mut h, _) = harness("wide-tree");
    let name = "Wide".repeat(44);
    h.state_mut().session.doc.project.texture_sets[0].name.clone_from(&name);
    h.state_mut().ui.export.tab = 2;
    h.run_steps(4);
    let bits_label = "16 bits";
    let bits_fit = |h: &Harness<'_, TextureCraftApp>| {
        h.output().shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == bits_label => clipped.clip_rect.contains_rect(text.visual_bounding_rect()),
            _ => false,
        })
    };
    assert!(!bits_fit(&h), "the bit-depth column starts beyond the viewport");
    h.get_by_role_and_label(egui::Role::DisclosureTriangle, &name).focus();
    h.run_steps(3);
    assert!(h.get_by_role_and_label(egui::Role::DisclosureTriangle, &name).is_focused());

    for _ in 0..12 {
        h.key_press(egui::Key::ArrowRight);
        h.run_steps(2);
    }
    h.run_steps(10);

    assert!(bits_fit(&h), "horizontal keyboard scrolling brings bit depth into the viewport");
    assert!(h.query_all_by_label(bits_label).next().is_some(), "bit depth has an accessibility label");
    assert!(h.query_by_label("Console").is_some());
}

#[test]
fn open_output_directory_asks_the_system_to_show_the_folder() {
    let (mut h, dir) = harness("open");
    h.state_mut().ui.export.tab = 2;
    h.run_steps(4);
    h.get_by_label("Open output directory").click();
    h.run_steps(2);

    // The test harness has no window: the folder is recorded instead of shown
    assert_eq!(h.state().file_dialogs.shown_folders(), std::slice::from_ref(&dir));
    assert!(dir.is_dir(), "a missing folder is created first");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_at_export_are_kept_while_it_runs() {
    let (mut h, dir) = harness("frozen");
    let moved = dir.with_extension("moved");
    let _ = std::fs::remove_dir_all(&moved);
    click(&mut h, "Export");
    h.run_steps(1);
    assert!(h.state().ui.export.job.is_some());

    // Another folder and size chosen while it runs reach none of the files
    h.state_mut().ui.export.dir = moved.display().to_string();
    h.state_mut().ui.export.size = 128;
    h.run_steps(1);

    assert!(h.query_all_by_label("256x256 (Texture Set size)").next().is_some());
    assert!(h.query_by_label("128x128").is_none(), "the preview shows the size taken at Export");

    finish(&mut h);
    assert!(!moved.exists(), "nothing was written to the folder chosen later");
    let written: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    let sets = h.state().session.set_count();
    assert_eq!(exports(&h), sets, "every texture set was written");
    for f in written.iter().filter(|p| p.extension().is_some_and(|e| e == "png")) {
        let width = u32::from_be_bytes(std::fs::read(f).unwrap()[16..20].try_into().unwrap());
        assert_eq!(width, 256, "{} keeps the size of the start", f.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changing_only_the_height_stops_a_frozen_export_before_writing() {
    let (mut h, directory) = harness("height-change");
    click(&mut h, "Export");
    let next_set = h.state().ui.export.job.as_ref().unwrap().pending[0].0;
    let set = &mut h.state_mut().session.doc.project.texture_sets[next_set];
    set.set_size(set.resolution, 128);

    finish(&mut h);

    assert_eq!(exports(&h), 0);
    assert!(console(&h).iter().any(|line| line.contains("planned files") && line.contains("changed")));
    assert!(!directory.exists());
}

#[test]
fn texture_sets_writing_the_same_file_name_do_not_start_an_export() {
    let (mut h, dir) = harness("clash");

    // A template without $textureSet gives every set the same file name (kept in a scratch library)
    let library = dir.with_extension("library");
    let _ = std::fs::remove_dir_all(&library);
    std::fs::create_dir_all(&library).unwrap();
    h.state_mut().session.load_export_presets(library.clone());
    let maps = json!([{"file": "same", "format": "Png", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]}]);
    h.state_mut().run("export.preset.save", json!({"preset": {"name": "Same", "maps": maps}})).unwrap();
    h.state_mut().ui.export.preset = "Same".into();
    h.run_steps(4);
    click(&mut h, "Export");
    h.run_steps(2);
    assert!(h.state().ui.export.job.is_none(), "no export runs");
    assert_eq!(exports(&h), 0);
    assert!(!dir.join("same.png").exists());
    assert!(console(&h).iter().any(|l| l.contains("both export same.png")), "{:?}", console(&h));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&library);
}

#[test]
fn a_texture_set_cannot_export_into_another_sets_psd_file() {
    use texturecraft_ui_egui::state::SetExport;

    let (mut h, _) = harness("psd-folder-clash");
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path().join("textures");
    let library = tempfile::tempdir().unwrap();
    let app = h.state_mut();
    app.ui.export.dir = dir.display().to_string();
    app.session.load_export_presets(library.path().to_path_buf());
    let psd = json!({"name": "Layered", "maps": [
        {"file": "roughness", "format": "Psd", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]}
    ]});
    app.run("export.preset.save", json!({"preset": psd})).unwrap();
    let plan = app.run("export.plan", json!({"preset": "Layered", "ts": 0})).unwrap();
    let file_name = plan[0]["file"].as_str().unwrap();
    let nested = json!({"name": "Nested", "maps": [
        {"file": format!("{file_name}/detail"), "format": "Png", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": "Roughness"}}]]}
    ]});
    app.run("export.preset.save", json!({"preset": nested})).unwrap();
    app.ui.export.preset = "Layered".into();
    app.ui.export.sets.insert(1, SetExport { preset: Some("Nested".into()), ..Default::default() });
    h.run_steps(4);

    click(&mut h, "Export");
    finish(&mut h);

    assert!(h.state().ui.export.job.is_none());
    assert_eq!(exports(&h), 0, "the full selection must be checked before starting");
    assert!(!dir.exists());
    assert!(console(&h).iter().any(|line| line.contains("folder")), "{:?}", console(&h));
}

#[test]
fn opening_another_project_stops_the_export() {
    let (mut h, dir) = harness("project");
    long_export(&mut h, &dir);
    h.run_steps(2);
    let before = exports(&h);
    assert!(before > 0 && h.state().ui.export.job.is_some());

    h.state_mut().session = texturecraft_engine::sample_session(256, 64).unwrap();
    h.run_steps(4);
    assert!(h.state().ui.export.job.is_none());
    assert_eq!(exports(&h), 0, "the new project exported nothing");
    assert!(console(&h).iter().any(|l| l == "Export stopped: another project was opened"), "{:?}", console(&h));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_is_disabled_without_a_ticked_texture_set() {
    let (mut h, dir) = harness("none");
    let sets = h.state().session.set_count();
    h.state_mut().ui.export.skip = (0..sets).collect();
    h.run_steps(2);
    click(&mut h, "Export");
    h.run_steps(2);
    let e = &h.state().ui.export;
    assert!(e.job.is_none() && e.console.is_empty() && e.tab == 0, "Export did nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_the_window_writes_no_further_texture_set() {
    let (mut h, dir) = harness("close");
    long_export(&mut h, &dir);
    h.run_steps(2);
    assert!(h.state().ui.export.job.is_some());
    let (writes, frame) = (exports(&h), h.ctx.cumulative_frame_nr());

    // Only the release frame closes the window and skips writing
    h.get_by_role_and_label(egui::Role::Button, "Close window").click();
    h.run_steps(1);
    assert!(!h.state().ui.windows.export && h.state().ui.export.job.is_none());
    let frames = h.ctx.cumulative_frame_nr() - frame;
    assert!(frames >= 1);
    assert_eq!((exports(&h) - writes) as u64, frames - 1, "the frame that closed the window wrote nothing");
    h.run_steps(2);
    assert_eq!((exports(&h) - writes) as u64, frames - 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_template_at_export_is_kept_while_it_runs() {
    let (mut h, dir) = harness("template");
    let library = dir.with_extension("library");
    let _ = std::fs::remove_dir_all(&library);
    std::fs::create_dir_all(&library).unwrap();
    h.state_mut().session.load_export_presets(library.clone());
    let map = |file: &str, channel: &str| json!({"file": file, "format": "Png", "bit_depth": 8, "channels": [["L", {"Channel": {"channel": channel}}]]});
    let both = json!([map("$textureSet_rough", "Roughness"), map("$textureSet_metal", "Metallic")]);
    h.state_mut().run("export.preset.save", json!({"preset": {"name": "Two", "maps": both}})).unwrap();
    h.state_mut().ui.export.preset = "Two".into();
    h.run_steps(4);
    click(&mut h, "Export");
    h.run_steps(1);
    assert!(h.state().ui.export.job.is_some());

    // The template loses a map while the export runs: the remaining sets still write it
    h.state_mut().run("export.preset.save", json!({"preset": {"name": "Two", "maps": [map("$textureSet_metal", "Metallic")]}})).unwrap();
    finish(&mut h);
    for set in h.state().session.doc.project.texture_sets.iter().map(|t| t.name.clone()).collect::<Vec<_>>() {
        let stem = texturecraft_engine::formats::export::sanitize_file_stem(&format!("{set}_rough"));
        assert!(dir.join(format!("{stem}.png")).exists(), "{stem}.png");
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&library);
}

#[test]
fn a_texture_set_that_leaves_the_export_leaves_its_count() {
    let (mut h, dir) = harness("count");
    long_export(&mut h, &dir);
    h.run_steps(2);

    // A rename the export cannot follow drops the set's queued writes
    texturecraft_engine::execute(&mut h.state_mut().session, "textureSet.rename", &json!({"ts": 1, "name": "renamed"})).unwrap();
    h.run_steps(1);
    let job = h.state().ui.export.job.clone().expect("the other sets are still queued");
    assert!(job.pending.iter().all(|(ts, _)| *ts != 1));
    assert_eq!(job.total - job.pending.len(), exports(&h), "the bar counts the writes done");
    let _ = std::fs::remove_dir_all(&dir);
}
