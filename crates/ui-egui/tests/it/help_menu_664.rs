//! Help > Export Log saves the log with each entry's time and level.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::Session;
use texturecraft_engine::geom::primitives;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::Answer;

fn harness() -> Harness<'static, TextureCraftApp> {
    let mut session = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
    session.log.push("first line".into());
    session.log.warn(texturecraft_engine::log::DEFAULT_SOURCE, "second line");
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

fn click_help_item(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_by_label("Help").click();
    h.run_steps(3);
    h.get_by_label(label).click();
    h.run_steps(4);
}

#[test]
fn export_log_writes_every_line_with_time_and_level() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("texturecraft-help-menu-664-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("log.txt");
    let answer = file.clone();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![answer.clone()]))));
    click_help_item(&mut h, "Export Log");

    let text = std::fs::read_to_string(&file).expect("the log file was written");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("TextureCraft log, exported "));
    let logged = h.state().session.log.entries();
    assert!(logged.len() >= 2);
    for (written, entry) in lines[1..].iter().zip(logged) {
        let expected = format!("[{}] {:5} {}", entry.time.format("%Y-%m-%dT%H:%M:%S%.6fZ"), entry.level.as_str(), entry.line());
        assert_eq!(*written, expected);
    }
    assert!(text.contains("] WARN  [TextureCraft] second line"));
    assert_eq!(lines.len(), logged.len() + 1);
    let _ = std::fs::remove_dir_all(&dir);
}
