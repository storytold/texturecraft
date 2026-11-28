//! The Log panel as in the reference: "[Source] message" lines, multi-selection, and the
//! right-click menu with Copy to clipboard, Clear (GENERAL) and Clear all / Clear selected (ASSETS)

use egui::{Event, Modifiers, OutputCommand};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

/// Enough lines that the Log cannot show them all at once
const LINES_PAST_THE_PANEL: usize = 100;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
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
    let mut harness = app();
    assert!(harness.query_by_label("Warnings").is_none(), "GENERAL has no filter button");

    harness.get_by_label("ASSETS").click();
    harness.run_steps(3);

    let button = harness.get_by_label("Warnings");
    assert!(button.accesskit_node().is_disabled());

    let at = button.rect().center();
    harness.hover_at(egui::pos2(1.0, 1.0));
    harness.run_steps(2);
    harness.hover_at(at);
    harness.run_steps(30);
    assert!(harness.query_by_label(texturecraft_ui_egui::unavailable::NOT_AVAILABLE).is_some());
}

/// The two tabs share the panel's width equally, side by side
#[test]
fn the_log_tabs_split_the_panel_width() {
    let harness = app();
    let (general, assets) = (harness.get_by_label("GENERAL").rect(), harness.get_by_label("ASSETS").rect());

    assert!((general.width() - assets.width()).abs() < 1.0, "{general:?} {assets:?}");
    assert!((general.right() - assets.left()).abs() < 1.0, "{general:?} {assets:?}");
}

/// The list's scrollbar keeps a gap to the list's frame, which ends where the tabs end
#[test]
fn the_log_scroll_bar_stands_clear_of_the_list_frame() {
    let harness = app();
    let (general, assets) = (harness.get_by_label("GENERAL").rect(), harness.get_by_label("ASSETS").rect());
    let bars =
        harness.get_all_by_role(egui::accesskit::Role::ScrollBar).filter(|bar| bar.rect().left() > general.left() && bar.rect().right() <= assets.right());
    let bar = bars.max_by(|a, b| a.rect().height().total_cmp(&b.rect().height())).expect("the list's scroll bar").rect();

    // The frame ends where the tab strip ends; its 1 point line and 1 point padding come before its inside
    let frame_inside = assets.right() - 2.0;
    assert!(bar.right() < frame_inside, "{bar:?} {assets:?}");
}

/// How many rows each painted text containing `part` has
fn painted_rows(harness: &Harness<'_, TextureCraftApp>, part: &str) -> Vec<usize> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text().contains(part) => Some(text.galley.rows.len()),
            _ => None,
        })
        .collect()
}

/// Log `message` and show it painted: the status bar repeats the newest message, so another follows it
fn log_behind_a_newer_line(harness: &mut Harness<'_, TextureCraftApp>, message: &str) {
    harness.state_mut().session.log.info("Exporter", message.to_owned());
    harness.state_mut().session.log.info("Exporter", "newest");
    harness.run_steps(3);
}

/// A line longer than the panel stays on one row
#[test]
fn a_long_line_stays_one_row() {
    let mut harness = app();
    log_behind_a_newer_line(&mut harness, &"long ".repeat(400));

    assert_eq!(painted_rows(&harness, "long long"), vec![1]);
}

/// A message with a line break is painted on its row, not over the lines around it
#[test]
fn a_message_with_a_line_break_is_painted_on_one_row() {
    let mut harness = app();
    log_behind_a_newer_line(&mut harness, "broken\nmessage");

    assert_eq!(painted_rows(&harness, "broken"), vec![1]);
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

/// Shift+Tab walks from the newest line to the first one, past the lines on screen when the panel opened
#[test]
fn the_keyboard_reaches_lines_scrolled_out_of_view() {
    let mut harness = app();
    for number in 0..LINES_PAST_THE_PANEL {
        harness.state_mut().session.log.info("Exporter", format!("line {number}"));
    }
    harness.run_steps(4);
    line(&harness, &format!("[Exporter] line {}", LINES_PAST_THE_PANEL - 1)).focus();
    harness.run_steps(2);

    let first_line_focused = |harness: &Harness<'_, TextureCraftApp>| {
        let screen = harness.ctx.content_rect();
        harness.query_all_by_label("[Exporter] first").any(|row| row.accesskit_node().is_focused() && screen.y_range().contains(row.rect().top()))
    };
    // One press per line is enough to reach the first one
    for _ in 0..harness.state().session.log.len() {
        if first_line_focused(&harness) {
            break;
        }
        harness.key_press_modifiers(Modifiers::SHIFT, egui::Key::Tab);
        harness.run_steps(2);
    }

    assert!(first_line_focused(&harness));
}
