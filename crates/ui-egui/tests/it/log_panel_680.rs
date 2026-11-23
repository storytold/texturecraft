//! The Log panel as in the reference: "[Source] message" lines, multi-selection, and the
//! right-click menu with Copy to clipboard, Clear (GENERAL) and Clear all / Clear selected (ASSETS)

use egui::{Event, Modifiers, OutputCommand};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    {
        let log = &mut h.state_mut().session.log;
        log.clear();
        log.info("Exporter", "first");
        log.warn("Exporter", "second");
        log.error("Python", "third");
        log.error("Library agent", "asset one");
        log.warn("Library agent", "asset two");
    }
    h.state_mut().ui.dock.focus(Panel::Log);
    h.run_steps(5);
    h
}

fn line<'h>(h: &'h Harness<'_, TextureCraftApp>, text: &'h str) -> egui_kittest::Node<'h> {
    // The status bar repeats the last message; the panel's line is the topmost one
    h.get_all_by_label(text).min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap_or_else(|| panic!("{text}"))
}

fn key_click(h: &mut Harness<'_, TextureCraftApp>, text: &str, modifiers: Modifiers) {
    let at = line(h, text).rect().center();
    h.hover_at(at);
    h.step();
    h.event_modifiers(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers }, modifiers);
    h.step();
    h.event_modifiers(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers }, modifiers);
    h.run_steps(3);
}

#[test]
fn lines_read_source_and_message_without_a_time_stamp() {
    let h = app();
    assert!(h.query_all_by_label("[Exporter] first").next().is_some());
    assert!(h.query_all_by_label("[Python] third").next().is_some());
    assert!(h.query_all_by_label("[Library agent] asset one").next().is_none(), "asset lines wait under ASSETS");
}

#[test]
fn lines_select_with_ctrl_and_shift() {
    let mut h = app();
    key_click(&mut h, "[Exporter] first", Modifiers::NONE);
    assert_eq!(h.state().ui.log_selection.len(), 1);
    key_click(&mut h, "[Python] third", Modifiers::COMMAND);
    assert_eq!(h.state().ui.log_selection.len(), 2);
    key_click(&mut h, "[Python] third", Modifiers::COMMAND);
    assert_eq!(h.state().ui.log_selection.len(), 1, "Ctrl+click on a selected line deselects it");
    key_click(&mut h, "[Exporter] first", Modifiers::NONE);
    key_click(&mut h, "[Python] third", Modifiers::SHIFT);
    assert_eq!(h.state().ui.log_selection.len(), 3, "Shift+click extends from the last plain click");
}

#[test]
fn ctrl_c_copies_the_selected_lines_as_plain_text() {
    let mut h = app();
    key_click(&mut h, "[Exporter] first", Modifiers::NONE);
    key_click(&mut h, "[Python] third", Modifiers::SHIFT);
    let at = line(&h, "[Exporter] second").rect().center();
    h.hover_at(at);
    h.step();
    h.event(Event::Copy);
    h.step();
    let copied = h.output().platform_output.commands.iter().rev().find_map(|c| match c {
        OutputCommand::CopyText(text) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(copied.as_deref(), Some("[Exporter] first\n[Exporter] second\n[Python] third"));
    assert!(h.state().session.clipboard.is_none(), "the same Ctrl+C does not also copy the layer");
}

#[test]
fn log_copy_takes_precedence_over_a_rebound_shortcut() {
    let mut h = app();
    h.state_mut().run("layer.add.fill", serde_json::json!({})).unwrap();
    h.state_mut().ui.keymap.insert("layer.copy".into(), "".into());
    h.state_mut().ui.keymap.insert("layer.duplicate".into(), "Ctrl+C".into());
    let layer_count = h.state().session.doc.project.layers.len();
    key_click(&mut h, "[Exporter] first", Modifiers::NONE);

    h.event(Event::Copy);
    h.step();

    assert!(h.output().platform_output.commands.contains(&OutputCommand::CopyText("[Exporter] first".into())));
    assert_eq!(h.state().session.doc.project.layers.len(), layer_count, "copying a log line cannot duplicate a layer");
}

#[test]
fn a_new_project_starts_with_no_line_selected() {
    let mut h = app();
    key_click(&mut h, "[Exporter] first", Modifiers::NONE);
    assert_eq!(h.state().ui.log_selection.len(), 1);
    h.state_mut().run("project.new", serde_json::json!({}));
    h.run_steps(3);
    assert!(h.state().ui.log_selection.is_empty());
}

#[test]
fn mesh_reimport_clears_log_selection_within_the_same_project() {
    let dir = super::temp_dir::TempDir::new("log-mesh-reimport");
    let path = dir.join("quad.obj");
    std::fs::write(&path, "v 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n").unwrap();
    let mut h = app();
    h.state_mut().run("file.new", serde_json::json!({"mesh": path, "resolution": 128})).unwrap();
    h.state_mut().session.log.info("Exporter", "before reimport");
    h.run_steps(3);
    key_click(&mut h, "[Exporter] before reimport", Modifiers::NONE);
    assert_eq!(h.state().ui.log_selection.len(), 1);
    let project = h.state().session.project_id;

    h.state_mut().run("file.reimportMesh", serde_json::json!({"path": path})).unwrap();
    h.run_steps(3);

    assert_eq!(h.state().session.project_id, project);
    assert!(h.state().ui.log_selection.is_empty());
}

#[test]
fn assets_has_a_greyed_filter_button_with_the_hint() {
    let mut h = app();
    assert!(h.query_by_label(egui_phosphor::regular::WARNING).is_none(), "GENERAL has no filter button");
    h.get_by_label("ASSETS").click();
    h.run_steps(3);
    let button = h.get_by_label(egui_phosphor::regular::WARNING);
    assert!(button.accesskit_node().is_disabled());
    let at = button.rect().center();
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run_steps(2);
    h.hover_at(at);
    h.run_steps(30);
    assert!(h.query_by_label(texturecraft_ui_egui::unavailable::NOT_AVAILABLE).is_some());
}

#[test]
fn renderer_settings_stays_open_when_opened_in_rendering_mode() {
    let mut h = app();
    h.state_mut().ui.mode = texturecraft_ui_egui::state::Mode::Rendering;
    h.run_steps(3);
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "ui.view.renderer");
    h.run_steps(5);
    assert!(h.state().ui.dock.is_open(Panel::Renderer));
    assert!(h.query_by_label("Tracer").is_some());
}

#[test]
fn the_general_menu_has_copy_and_clear_on_a_line_and_on_empty_space() {
    let mut h = app();
    let at = line(&h, "[Exporter] second").rect().center();
    h.hover_at(at);
    h.step();
    line(&h, "[Exporter] second").click_secondary();
    h.run_steps(3);
    assert!(h.query_by_label_contains("Copy to clipboard").is_some());
    assert!(h.query_by_label("Clear").is_some());
    assert!(h.query_by_label("Clear all").is_none());
    assert_eq!(h.state().ui.log_selection.len(), 1, "a right-click selects the line under it");

    h.get_by_label("Clear").click();
    h.run_steps(3);
    let log = &h.state().session.log;
    assert_eq!(log.len(), 2, "Clear drops the GENERAL lines and keeps the asset lines: {log:?}");
    assert!(log.iter().all(|entry| entry.is_asset()));

    // The empty space under the lines opens the same menu
    let tab = h.get_by_label("ASSETS").rect();
    let empty = egui::pos2(tab.left() + 40.0, tab.bottom() + 150.0);
    h.hover_at(empty);
    h.step();
    h.event(Event::PointerButton { pos: empty, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: empty, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(h.query_by_label_contains("Copy to clipboard").is_some() && h.query_by_label("Clear").is_some(), "the empty space has the line menu");
}

#[test]
fn the_assets_menu_clears_all_or_only_the_selected_lines() {
    let mut h = app();
    h.get_by_label("ASSETS").click();
    h.run_steps(3);
    key_click(&mut h, "[Library agent] asset one", Modifiers::NONE);
    line(&h, "[Library agent] asset one").click_secondary();
    h.run_steps(3);
    assert!(h.query_by_label("Clear all").is_some());
    h.get_by_label("Clear selected").click();
    h.run_steps(3);
    let remaining: Vec<String> = h.state().session.log.iter().filter(|entry| entry.is_asset()).map(|entry| entry.message.clone()).collect();
    assert_eq!(remaining, vec!["asset two".to_string()]);

    line(&h, "[Library agent] asset two").click_secondary();
    h.run_steps(3);
    h.get_by_label("Clear all").click();
    h.run_steps(3);
    assert!(h.state().session.log.iter().all(|entry| !entry.is_asset()));
    assert_eq!(h.state().session.log.len(), 3, "the GENERAL lines stay");
}
