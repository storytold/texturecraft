//! Export textures › SETTINGS: the Export USD asset option. It writes `<mesh>.usd` (the mesh and a
//! material per texture set that reads the exported textures) next to the textures, with the
//! export's last texture set.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{ConsoleLine, ExportJob};

/// A scratch folder that is removed when it goes out of scope, so a failing test leaves nothing behind.
struct Scratch(std::path::PathBuf);

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The Export textures window writing into a scratch folder of its own for `name`
fn harness(name: &str) -> (Harness<'static, TextureCraftApp>, Scratch) {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    let dir = std::env::temp_dir().join(format!("tc-export-usd-asset-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir = Scratch(dir);
    h.state_mut().ui.export.dir = dir.display().to_string();
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    (h, dir)
}

fn checkbox<'a>(h: &'a Harness<'static, TextureCraftApp>) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(egui::Role::CheckBox, "Export USD asset")
}

/// Click Export and run frames until the export ends
fn export(h: &mut Harness<'static, TextureCraftApp>) {
    h.get_all_by_label("Export").last().unwrap().click();
    for _ in 0..64 {
        h.run_steps(1);
        if h.state().ui.export.job.is_none() {
            break;
        }
    }
    h.run_steps(2);
}

/// The texture files `<dir>/material_ball.usd` refers to
fn referenced_files(dir: &std::path::Path) -> Vec<String> {
    let bytes = std::fs::read(dir.join("material_ball.usd")).expect("material_ball.usd");
    let layer = craft_usd::scene::read_layer(&bytes).unwrap();
    layer
        .prims
        .iter()
        .filter_map(|prim| match prim.props.get("inputs:file")?.value.as_ref()? {
            craft_usd::scene::Val::Tok(values) => values.first().map(|file| file.trim_start_matches("./").to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_window_offers_export_usd_asset_off_by_default() {
    let (h, _dir) = harness("offer");
    assert!(!checkbox(&h).accesskit_node().is_disabled());
    assert!(!h.state().ui.export.usd_asset);
}

#[test]
fn ticking_export_usd_asset_writes_one_file_that_refers_to_the_exported_textures() {
    let (mut h, dir) = harness("on");
    checkbox(&h).click();
    h.run_steps(3);
    assert!(h.state().ui.export.usd_asset);
    export(&mut h);

    let files = referenced_files(&dir);
    assert!(h.state().session.set_count() >= 2, "one file has to cover several texture sets");
    assert!(files.len() >= h.state().session.set_count(), "every material reads textures: {files:?}");
    for file in &files {
        assert!(dir.join(file).is_file(), "{file} was written");
    }
    let usd = std::fs::read_dir(&*dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "usd")).count();
    assert_eq!(usd, 1, "one file for the whole export, not one per texture set");
}

#[test]
fn the_file_refers_to_the_textures_each_texture_set_was_exported_with() {
    let (mut h, dir) = harness("overrides");
    h.state_mut().ui.export.sets.entry(1).or_default().preset = Some("Unreal Engine (Packed)".into());
    checkbox(&h).click();
    h.run_steps(3);
    export(&mut h);

    let files = referenced_files(&dir);
    assert!(files.iter().any(|f| f.ends_with("_OcclusionRoughnessMetallic.png")), "the second texture set's template: {files:?}");
    assert!(files.iter().any(|f| f.ends_with("_roughness.png")), "the first texture set's template: {files:?}");
    for file in &files {
        assert!(dir.join(file).is_file(), "{file} was written");
    }
}

#[test]
fn only_the_exported_texture_sets_have_textures() {
    let (mut h, dir) = harness("subset");
    h.state_mut().ui.export.skip = vec![0];
    checkbox(&h).click();
    h.run_steps(3);
    export(&mut h);

    let first = texturecraft_engine::formats::export::sanitize_file_stem(&h.state().session.doc.project.texture_sets[0].name);
    let files = referenced_files(&dir);
    assert!(!files.is_empty());
    assert!(files.iter().all(|f| !f.starts_with(&format!("material_ball_{first}_"))), "the skipped texture set has none: {files:?}");
}

#[test]
fn the_console_lists_the_asset_as_a_file_but_not_as_a_saved_texture() {
    let (mut h, _dir) = harness("console");
    checkbox(&h).click();
    h.run_steps(3);
    export(&mut h);

    let console = &h.state().ui.export.console;
    assert!(console.iter().any(|line| matches!(line, ConsoleLine::Done(file) if file == "material_ball.usd")), "{console:?}");
    assert!(!console.iter().any(|line| matches!(line, ConsoleLine::Step(text) if text == "Saved texture material_ball")), "{console:?}");
    assert!(console.iter().any(|line| matches!(line, ConsoleLine::Step(text) if text.starts_with("Saved texture "))), "the textures are still listed");
}

#[test]
fn a_canceled_export_writes_no_asset() {
    let (mut h, dir) = harness("cancel");

    // A long export, as the window starts it with the option ticked
    let sets = h.state().session.set_count();
    let params = serde_json::json!({"dir": dir.display().to_string()});
    let pending: Vec<(usize, serde_json::Value)> = (0..sets).cycle().take(sets * 8).map(|ts| (ts, params.clone())).collect();
    let e = &mut h.state_mut().ui.export;
    e.tab = 2;
    e.job = Some(ExportJob { total: pending.len(), pending, usd_asset: true, ..ExportJob::default() });
    h.run_steps(2);
    h.get_all_by_label("Cancel Export").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().ui.export.job.is_none());
    assert!(!dir.join("material_ball.usd").exists());

    // The same job run to its end writes it
    let pending: Vec<(usize, serde_json::Value)> = (0..sets).map(|ts| (ts, params.clone())).collect();
    h.state_mut().ui.export.job = Some(ExportJob { total: pending.len(), pending, usd_asset: true, ..ExportJob::default() });
    for _ in 0..64 {
        if h.state().ui.export.job.is_none() {
            break;
        }
        h.run_steps(1);
    }
    assert!(dir.join("material_ball.usd").is_file());
}

#[test]
fn no_asset_is_written_when_the_option_is_off() {
    let (mut h, dir) = harness("off");
    export(&mut h);
    assert!(std::fs::read_dir(&*dir).unwrap().next().is_some(), "the textures were written");
    assert!(!dir.join("material_ball.usd").exists());
}

#[test]
fn restoring_the_default_parameters_unticks_export_usd_asset() {
    let (mut h, _dir) = harness("reset");
    checkbox(&h).click();
    h.run_steps(3);
    assert!(h.state().ui.export.usd_asset);
    h.get_by_label("Restore the default export parameters").click();
    h.run_steps(3);
    assert!(!h.state().ui.export.usd_asset);
}

#[test]
fn reset_all_unticks_export_usd_asset() {
    let (mut h, _dir) = harness("reset-all");
    checkbox(&h).click();
    h.run_steps(3);
    h.get_by_label("Reset all").click();
    h.run_steps(3);
    assert!(!h.state().ui.export.usd_asset);
}

#[test]
fn an_asset_that_cannot_be_written_shows_an_error_toast_and_the_textures_stay() {
    let (mut h, dir) = harness("blocked");

    // A folder where the file belongs: the asset cannot be written there
    std::fs::create_dir_all(dir.join("material_ball.usd")).unwrap();
    checkbox(&h).click();
    h.run_steps(3);
    export(&mut h);

    let toast = h.state().toasts.iter().find(|toast| toast.text.starts_with("The USD asset was not written")).expect("a toast names the failure");
    assert!(toast.error);
    assert!(h.state().ui.export.job.is_none(), "the export ran to its end");
    assert!(std::fs::read_dir(&*dir).unwrap().flatten().any(|e| e.path().extension().is_some_and(|x| x == "png")), "the textures were written");
}

/// A running USD export that has already written texture sets 0 and 1 (the `tag` of each sent command tells them apart)
fn running_with_two_sent(h: &mut Harness<'static, TextureCraftApp>, dir: &std::path::Path) {
    let sets = h.state().session.set_count();
    let params = serde_json::json!({"dir": dir.display().to_string()});
    let pending: Vec<(usize, serde_json::Value)> = (0..sets).cycle().take(sets * 8).map(|ts| (ts, params.clone())).collect();
    let sent = (0..2).map(|ts| serde_json::json!({"dir": dir.display().to_string(), "ts": ts, "tag": ts})).collect();
    let e = &mut h.state_mut().ui.export;
    e.tab = 2;
    e.job = Some(ExportJob { total: pending.len(), pending, usd_asset: true, sent, ..ExportJob::default() });
}

/// The `ts` of the sent commands that carry `tag`
fn sent_ts(h: &Harness<'static, TextureCraftApp>, tag: u64) -> Vec<u64> {
    let job = h.state().ui.export.job.as_ref().expect("the export is still running");
    job.sent.iter().filter(|params| params["tag"] == tag).map(|params| params["ts"].as_u64().unwrap()).collect()
}

#[test]
fn the_sent_commands_of_a_running_export_follow_their_texture_sets_when_they_move() {
    let (mut h, dir) = harness("sent-move");
    running_with_two_sent(&mut h, &dir);
    h.run_steps(1);
    assert_eq!((sent_ts(&h, 0), sent_ts(&h, 1)), (vec![0], vec![1]));

    // The two texture sets trade places (names), as a reorder does
    let names: Vec<String> = h.state().session.doc.project.texture_sets.iter().map(|t| t.name.clone()).collect();
    for (ts, name) in [(0, "swap"), (1, names[0].as_str()), (0, names[1].as_str())] {
        texturecraft_engine::execute(&mut h.state_mut().session, "textureSet.rename", &serde_json::json!({"ts": ts, "name": name})).unwrap();
    }
    h.run_steps(1);
    assert_eq!((sent_ts(&h, 0), sent_ts(&h, 1)), (vec![1], vec![0]), "each command stays with its texture set");
}

#[test]
fn the_sent_command_of_a_texture_set_that_leaves_the_export_is_dropped() {
    let (mut h, dir) = harness("sent-drop");
    running_with_two_sent(&mut h, &dir);
    h.run_steps(1);

    // A rename the export cannot follow takes the texture set out of it
    texturecraft_engine::execute(&mut h.state_mut().session, "textureSet.rename", &serde_json::json!({"ts": 1, "name": "renamed"})).unwrap();
    h.run_steps(1);
    assert_eq!((sent_ts(&h, 0), sent_ts(&h, 1)), (vec![0], vec![]));
}

#[test]
fn each_sent_command_keeps_the_names_it_reserved_for_the_asset() {
    let (mut app_harness, dir) = harness("sent-reserved");
    let params = serde_json::json!({"dir": dir.display().to_string()});
    let pending = vec![(0, params.clone()), (1, params)];
    let settings = &mut app_harness.state_mut().ui.export;
    settings.tab = 2;
    settings.job = Some(ExportJob { total: pending.len(), pending, usd_asset: true, written: vec!["earlier.png".into()], ..ExportJob::default() });

    for _ in 0..8 {
        app_harness.run_steps(1);
        if app_harness.state().ui.export.job.as_ref().is_none_or(|job| !job.sent.is_empty()) {
            break;
        }
    }

    let job = app_harness.state().ui.export.job.as_ref().expect("the export is still running");
    let reserved = job.sent.first().and_then(|params| params["reservedNames"].as_array()).expect("the sent command carries its reserved names");
    assert!(reserved.iter().any(|name| name == "earlier.png"), "the asset skips what the texture export skipped: {reserved:?}");
}
