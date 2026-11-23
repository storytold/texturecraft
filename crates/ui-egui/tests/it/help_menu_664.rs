//! Help ▸ Release notes... opens the releases page; Help ▸ Export log... saves the log to a text file;
//! Help ▸ Join the community (Discord)... opens the community Discord.

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
fn release_notes_opens_the_releases_page() {
    for id in ["ui.help.releaseNotes", "ui.help.exportLog"] {
        assert!(texturecraft_ui_egui::keymap::bindings().iter().any(|binding| binding.id == id));
    }

    let mut h = harness();
    h.get_by_label("Help").click();
    h.run_steps(3);
    h.get_by_label("Release notes...").click();

    // The open-url command is in the output of the frame that handled the click only
    h.step();
    let opened = h
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, egui::OutputCommand::OpenUrl(u) if u.url == "https://github.com/storytold/texturecraft/releases"));
    assert!(opened, "the releases page was opened");
}

const DISCORD: &str = "https://discord.gg/artcraft";

fn opened(h: &Harness<'_, TextureCraftApp>, url: &str) -> bool {
    h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::OpenUrl(u) if u.url == url))
}

#[test]
fn join_the_community_opens_the_discord() {
    let mut h = harness();
    h.get_by_label("Help").click();
    h.run_steps(3);
    h.get_by_label("Join the community (Discord)...").click();
    h.step();
    assert!(opened(&h, DISCORD), "the community Discord was opened");
}

#[test]
fn join_the_community_is_bindable_and_a_bound_key_opens_the_discord() {
    assert!(texturecraft_ui_egui::keymap::bindings().iter().any(|b| b.id == "ui.help.discord"), "ui.help.discord can be bound");
    let mut h = harness();
    h.state_mut().ui.keymap.insert("ui.help.discord".into(), "F12".into());
    h.run_steps(2);
    h.event(egui::Event::Key { key: egui::Key::F12, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.step();
    assert!(opened(&h, DISCORD), "the bound key opened the community Discord");
}

#[test]
fn export_log_writes_every_line_with_time_and_level() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("texturecraft-help-menu-664-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("log.txt");
    let answer = file.clone();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![answer.clone()]))));
    click_help_item(&mut h, "Export log...");

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

#[test]
fn a_canceled_export_log_dialog_writes_nothing() {
    let mut h = harness();
    h.state_mut().file_dialogs.set_override(Some(Box::new(|_| Answer::Cancelled)));
    click_help_item(&mut h, "Export log...");
    assert!(h.state().toasts.is_empty());
}
