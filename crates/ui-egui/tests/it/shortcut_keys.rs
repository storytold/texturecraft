//! Key presses run the commands bound to exactly that chord.

use egui::{Event, Key, Modifiers, OutputCommand};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::ViewLayout;

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

fn press(key: Key, modifiers: Modifiers, repeat: bool) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat, modifiers }
}

fn hidden(h: &Harness<'_, TextureCraftApp>) -> bool {
    h.state().ui.hide_ui
}

#[test]
fn a_bound_key_runs_its_command() {
    let mut h = harness();
    h.event(press(Key::Tab, Modifiers::NONE, false));
    h.run_steps(2);
    assert!(hidden(&h), "Tab hides the UI");
}

#[test]
fn extra_shift_or_alt_does_not_run_a_plain_key_shortcut() {
    let mut h = harness();
    h.event(press(Key::Tab, Modifiers::SHIFT, false));
    h.event(press(Key::Tab, Modifiers::ALT, false));
    h.run_steps(2);
    assert!(!hidden(&h));
}

#[test]
fn a_held_key_runs_its_command_again() {
    let mut h = harness();
    // Tab also moves egui's keyboard focus, and a focused text field stops the second press.
    h.state_mut().ui.keymap.insert("ui.hideUi".into(), "F7".into());
    h.run_steps(2);
    h.event(press(Key::F7, Modifiers::NONE, false));
    h.run_steps(2);
    assert!(hidden(&h));
    h.event(press(Key::F7, Modifiers::NONE, true));
    h.run_steps(2);
    assert!(!hidden(&h), "the repeat toggled it back");
}

#[test]
fn a_shortcut_stored_in_the_settings_replaces_the_default() {
    let mut h = harness();
    assert_ne!(h.state().ui.layout, ViewLayout::Only3D);
    h.state_mut().ui.keymap.insert("ui.layout.3d".into(), "Ctrl+Shift+H".into());
    h.run_steps(2);
    h.event(press(Key::F2, Modifiers::NONE, false));
    h.run_steps(2);
    assert_ne!(h.state().ui.layout, ViewLayout::Only3D, "the default no longer runs it");
    h.event(press(Key::H, Modifiers::COMMAND | Modifiers::SHIFT, false));
    h.run_steps(2);
    assert_eq!(h.state().ui.layout, ViewLayout::Only3D);
}

#[test]
fn copy_with_a_log_line_selected_puts_its_text_on_the_clipboard() {
    let mut h = harness();
    h.state_mut().session.log.push("a line to copy".into());
    h.state_mut().ui.dock.focus(Panel::Log);
    h.run_steps(4);
    let line = h.get_all_by_label("[TextureCraft] a line to copy").next().unwrap().rect();
    h.get_all_by_label("[TextureCraft] a line to copy").next().unwrap().click();
    h.run_steps(2);
    h.hover_at(line.center());
    h.step();
    h.event(Event::Copy);
    h.step();
    let last_copied = h.output().platform_output.commands.iter().rev().find_map(|c| match c {
        OutputCommand::CopyText(text) => Some(text.clone()),
        _ => None,
    });
    assert_eq!(last_copied.as_deref(), Some("[TextureCraft] a line to copy"), "the selected line is the last text copied");
}

#[test]
fn a_pasted_image_counts_as_the_paste_shortcut() {
    let mut h = harness();
    let before = h.state().session.doc.project.layers.len();
    h.event(Event::Copy);
    h.run_steps(3);
    assert!(h.state().session.clipboard.is_some());
    let image = std::sync::Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::WHITE));
    h.event(Event::PasteImage(image));
    h.run_steps(3);
    assert!(h.state().session.doc.project.layers.len() > before, "the paste ran layer.paste");
}

#[test]
fn a_command_without_a_default_key_runs_from_the_key_it_is_bound_to() {
    let mut h = harness();
    let before = h.state().session.doc.project.layers.len();
    h.event(press(Key::F7, Modifiers::NONE, false));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.project.layers.len(), before, "F7 is free");
    h.state_mut().ui.keymap.insert("layer.add.fill".into(), "F7".into());
    h.run_steps(2);
    h.event(press(Key::F7, Modifiers::NONE, false));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.project.layers.len(), before + 1, "F7 added a fill layer");
}

#[test]
fn the_shortcut_editor_lists_a_command_without_a_key_and_hides_one_that_needs_a_parameter() {
    let mut h = harness();
    h.state_mut().ui.windows.shortcuts = true;
    h.run_steps(4);
    assert!(h.query_by_label("Toggle layer visibility").is_some(), "Toggle layer visibility is listed");
    assert!(h.query_by_label("Select texture set").is_none(), "Select texture set needs a texture set");
}
