//! The export dialog's MaterialX option writes the same `.mtlx` files as `file.exportTextures` with `materialX`.

use std::path::{Path, PathBuf};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ExportUi;

const OPENPBR: &str = "OpenPBR (separate maps)";
const OPTION: &str = "Also write a MaterialX file (.mtlx) per texture set";

fn session() -> Session {
    Session::new_project("Ball", texturecraft_engine::geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap()
}

/// A new, empty folder.
fn temp_dir(test: &str, part: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-export-materialx-{test}-{part}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn dialog(dir: &Path, preset: &str) -> Harness<'static, TextureCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session()));
    h.state_mut().ui.export.dir = dir.display().to_string();
    h.state_mut().ui.export.preset = preset.to_string();
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    h
}

fn names(dir: &Path, extension: &str) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(extension)).collect())
        .unwrap_or_default();
    found.sort();
    found
}

#[test]
fn ticking_materialx_writes_the_same_material_files_as_the_command() {
    let (ui_dir, cmd_dir) = (temp_dir("same", "ui"), temp_dir("same", "cmd"));
    let mut h = dialog(&ui_dir, OPENPBR);
    h.get_by_label(OPTION).click();
    h.run_steps(2);
    assert!(h.state().ui.export.material_x);
    h.get_by_label("Export").click();
    h.run_steps(4);
    assert!(!h.state().ui.windows.export, "the export ran and closed the dialog");

    let mut s = session();
    for ts in 0..s.set_count() {
        execute(&mut s, "file.exportTextures", &json!({"dir": cmd_dir.display().to_string(), "preset": OPENPBR, "materialX": true, "ts": ts})).unwrap();
    }
    let written = names(&ui_dir, ".mtlx");
    assert_eq!(written.len(), s.set_count(), "one material per texture set: {written:?}");
    assert_eq!(written, names(&cmd_dir, ".mtlx"));
    for name in &written {
        let (from_ui, from_command) = (std::fs::read_to_string(ui_dir.join(name)).unwrap(), std::fs::read_to_string(cmd_dir.join(name)).unwrap());
        assert!(from_ui.contains("<open_pbr_surface"), "{from_ui}");
        assert_eq!(from_ui, from_command);
    }
    assert_eq!(names(&ui_dir, ".png"), names(&cmd_dir, ".png"), "the textures are the same too");
    let _ = (std::fs::remove_dir_all(&ui_dir), std::fs::remove_dir_all(&cmd_dir));
}

#[test]
fn unticked_writes_textures_but_no_material() {
    let dir = temp_dir("off", "ui");
    let mut h = dialog(&dir, OPENPBR);
    assert!(!h.state().ui.export.material_x, "off by default");
    h.get_by_label("Export").click();
    h.run_steps(4);
    assert!(!names(&dir, ".png").is_empty(), "the textures were written");
    assert!(names(&dir, ".mtlx").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_export_list_shows_the_material_files_only_when_ticked() {
    let mut h = dialog(&temp_dir("list", "ui"), OPENPBR);
    let listed = |h: &Harness<'_, TextureCraftApp>| {
        let mut params = texturecraft_ui_egui::windows::export_params(h.state());
        params["ts"] = json!("all");
        let mut s = session();
        execute(&mut s, "export.plan", &params).unwrap().as_array().unwrap().iter().filter(|f| f["file"].as_str().unwrap().ends_with(".mtlx")).count()
    };
    assert_eq!(listed(&h), 0);
    h.state_mut().ui.export.material_x = true;
    assert_eq!(listed(&h), h.state().session.set_count());
    // The dialog's own list tab shows them.
    h.state_mut().ui.export.tab = 2;
    h.run_steps(3);
    assert!(h.query_all_by_label_contains(".mtlx").next().is_some(), "the List of exports tab names the .mtlx files");
}

#[test]
fn the_option_is_kept_with_the_export_settings_and_reset_clears_it() {
    let mut settings = ExportUi::default();
    settings.material_x = true;
    let json = serde_json::to_string(&settings).unwrap();
    let back: ExportUi = serde_json::from_str(&json).unwrap();
    assert!(back.material_x);
    // Settings saved before the option existed load with it off.
    let mut old: serde_json::Value = serde_json::from_str(&json).unwrap();
    old.as_object_mut().unwrap().remove("material_x");
    assert!(!serde_json::from_value::<ExportUi>(old).unwrap().material_x);

    let mut h = dialog(&temp_dir("keep", "ui"), OPENPBR);
    h.state_mut().ui.export.material_x = true;
    h.run_steps(2);
    h.get_by_label("Reset all").click();
    h.run_steps(2);
    assert!(!h.state().ui.export.material_x);
}

#[test]
fn ticking_materialx_with_another_template_says_why_no_material_was_written() {
    let dir = temp_dir("warn", "ui");
    let mut h = dialog(&dir, "Document channels + Normal + AO");
    h.get_by_label(OPTION).click();
    h.run_steps(2);
    h.get_by_label("Export").click();
    h.run_steps(4);
    assert!(names(&dir, ".mtlx").is_empty());
    let toasts: Vec<&str> = h.state().toasts.iter().filter(|t| t.error && t.text.starts_with("MaterialX:")).map(|t| t.text.as_str()).collect();
    assert!(!toasts.is_empty(), "a MaterialX warning is shown");
    assert!(toasts.windows(2).all(|w| w[0] != w[1]), "the same warning is not repeated per texture set: {toasts:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
