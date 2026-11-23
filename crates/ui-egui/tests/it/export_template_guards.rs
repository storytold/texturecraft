//! Export dialog > Output templates: deleting a template asks first, and switching away from a
//! template whose last edit does not validate (so is not saved) asks before dropping that edit.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_ui_egui::TextureCraftApp;

mod library;

use library::Library;

/// The Output templates tab with two user templates, "Alpha tpl" (selected) and "Beta tpl", saved in `library`.
fn harness(library: &Library) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h.state_mut().session.load_export_presets(library.0.clone());
    let base = texturecraft_engine::formats::builtin_presets().remove(0);
    for name in ["Alpha tpl", "Beta tpl"] {
        let preset = texturecraft_engine::formats::ExportPreset { name: name.into(), ..base.clone() };
        h.state_mut().run("export.preset.save", json!({ "preset": preset })).unwrap();
    }
    let ui = &mut h.state_mut().ui;
    ui.windows.export = true;
    ui.export.tab = 1;
    ui.export.preset = "Alpha tpl".into();
    h.run_steps(4);
    h
}

/// Type an invalid file name into the first output of the selected template, as the editor would.
fn break_draft(h: &mut Harness<'_, TextureCraftApp>) {
    let draft = h.state_mut().ui.export.draft.as_mut().unwrap();
    draft.maps[0].file = "a/b".into();
    assert!(draft.validate().is_err());
    h.run_steps(3);
}

fn selected(h: &Harness<'_, TextureCraftApp>) -> String {
    h.state().ui.export.preset.clone()
}

#[test]
fn delete_asks_and_cancel_keeps_the_template() {
    let library = Library::new("delete-cancel");
    let mut h = harness(&library);
    assert_eq!(library.files(), 2);
    h.get_by_label("Delete the selected template").click();
    h.run_steps(3);
    assert_eq!(library.files(), 2, "nothing is deleted before the answer");

    // The export dialog has a Cancel too; the question's comes last
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().ui.windows.export, "the export dialog stays open");
    assert_eq!(library.files(), 2, "Cancel keeps the file");
    assert!(h.state().session.export_presets.list.iter().any(|p| p.name == "Alpha tpl"));
    assert!(h.state().ui.export.ask.is_none());
}

#[test]
fn escape_keeps_the_template_being_deleted() {
    let library = Library::new("delete-escape");
    let mut h = harness(&library);
    h.get_by_label("Delete the selected template").click();
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(library.files(), 2);
    assert!(h.state().ui.export.ask.is_none(), "Escape closes the question");
}

#[test]
fn delete_ok_removes_the_template_and_its_file() {
    let library = Library::new("delete-ok");
    let mut h = harness(&library);
    h.get_by_label("Delete the selected template").click();
    h.run_steps(3);
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert_eq!(library.files(), 1, "the file is gone");
    assert!(!h.state().session.export_presets.list.iter().any(|p| p.name == "Alpha tpl"));
    assert!(h.state().ui.export.ask.is_none());
}

#[test]
fn unfinished_edit_asks_before_selecting_another_template_and_keep_editing_stays() {
    let library = Library::new("keep-editing");
    let mut h = harness(&library);
    break_draft(&mut h);
    h.get_by_label_contains("Beta tpl").scroll_to_me();
    h.run_steps(12);
    h.get_by_label_contains("Beta tpl").click();
    h.run_steps(3);
    assert_eq!(selected(&h), "Alpha tpl", "the selection waits for the answer");
    h.get_by_label("Keep editing").click();
    h.run_steps(3);
    assert_eq!(selected(&h), "Alpha tpl");
    assert_eq!(h.state().ui.export.draft.as_ref().unwrap().maps[0].file, "a/b", "the edit is intact");
    assert!(h.state().ui.export.ask.is_none());
}

#[test]
fn unfinished_edit_discard_switches_to_the_other_template() {
    let library = Library::new("discard");
    let mut h = harness(&library);
    break_draft(&mut h);
    h.get_by_label_contains("Beta tpl").scroll_to_me();
    h.run_steps(12);
    h.get_by_label_contains("Beta tpl").click();
    h.run_steps(3);
    h.get_by_label("Discard changes").click();
    h.run_steps(3);
    assert_eq!(selected(&h), "Beta tpl");
    let draft = h.state().ui.export.draft.as_ref().unwrap();
    assert_eq!(draft.name, "Beta tpl");
    assert!(draft.validate().is_ok(), "the dropped edit did not follow");
    let alpha = h.state().session.export_preset("Alpha tpl").unwrap();
    assert_ne!(alpha.maps[0].file, "a/b", "the invalid edit was never saved");
}

#[test]
fn unfinished_edit_also_asks_before_new_template() {
    let library = Library::new("new");
    let mut h = harness(&library);
    break_draft(&mut h);
    h.get_by_label("New template").click();
    h.run_steps(3);
    assert_eq!(library.files(), 2, "no template is added before the answer");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(selected(&h), "Alpha tpl");
    assert_eq!(library.files(), 2);
}

#[test]
fn a_saved_edit_switches_without_asking() {
    let library = Library::new("saved");
    let mut h = harness(&library);
    let mut draft = h.state().ui.export.draft.clone().unwrap();
    draft.maps[0].file = "$mesh_Edited".into();
    assert!(draft.validate().is_ok());
    h.state_mut().run("export.preset.save", json!({ "preset": draft })).unwrap();
    h.run_steps(3);
    h.get_by_label_contains("Beta tpl").scroll_to_me();
    h.run_steps(12);
    h.get_by_label_contains("Beta tpl").click();
    h.run_steps(3);
    assert!(h.state().ui.export.ask.is_none(), "nothing to ask: the edit is saved");
    assert_eq!(selected(&h), "Beta tpl");
    assert_eq!(h.state().session.export_preset("Alpha tpl").unwrap().maps[0].file, "$mesh_Edited");
}

#[test]
fn the_settings_dropdown_asks_about_an_unfinished_edit_too() {
    let library = Library::new("dropdown");
    let mut h = harness(&library);
    break_draft(&mut h);
    texturecraft_ui_egui::export_templates::select(h.state_mut(), "Beta tpl".into());
    assert_eq!(selected(&h), "Alpha tpl");
    assert!(h.state().ui.export.ask.is_some());
    h.state_mut().ui.export.ask = None;
    h.state_mut().ui.export.draft.as_mut().unwrap().maps[0].file = "$mesh_ok".into();
    texturecraft_ui_egui::export_templates::select(h.state_mut(), "Beta tpl".into());
    assert_eq!(selected(&h), "Beta tpl", "a valid edit switches at once");
}
