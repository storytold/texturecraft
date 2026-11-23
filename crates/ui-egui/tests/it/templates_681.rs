//! Project templates: File > More save options (Save as template..., Save and reduce file size,
//! Save as copy...), the New project list's reference names and the remembered template

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::RefCell;
use std::rc::Rc;
use texturecraft_ui_egui::file_dialogs::{Answer, Request};
use texturecraft_ui_egui::unavailable::NOT_AVAILABLE;
use texturecraft_ui_egui::{TextureCraftApp, menus};

use super::temp_dir::TempDir;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(128, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1200.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.state_mut().session.templates.dir = None;
    h.run_steps(4);
    h
}

fn open_more_save_options(h: &mut Harness<'_, TextureCraftApp>) {
    h.get_by_label("File").click();
    h.run_steps(3);
    h.get_by_label("More save options").click();
    h.run_steps(3);
}

#[test]
fn save_as_template_asks_in_the_templates_folder_and_saves_without_a_message() {
    let dir = TempDir::new("save-template-destination");
    let mut h = app();
    let library = dir.join("templates");
    h.state_mut().session.templates.dir = Some(library.clone());
    let asked: Rc<RefCell<Vec<Request>>> = Rc::default();
    let seen = asked.clone();
    let destination = dir.join("My starter v1.2.json");
    let chosen = dir.join("My starter v1.2");
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |request: &Request| {
        seen.borrow_mut().push(request.clone());
        Answer::Paths(vec![chosen.clone()])
    })));

    open_more_save_options(&mut h);
    h.get_by_label_contains("Save as template...").click();
    h.run_steps(4);

    let asked = asked.borrow();
    let Some(Request::File(dialog)) = asked.first() else { panic!("a file dialog was asked for: {asked:?}") };
    assert_eq!(dialog.title.as_deref(), Some("Save template"));
    assert!(library.is_dir(), "the save picker can open a new library folder");
    assert_eq!(dialog.directory, Some(library));
    assert_eq!(dialog.filters.len(), 1, "one file type");
    assert!(dialog.file_name.is_none(), "the name field starts empty");
    assert!(h.state().session.templates.list.iter().any(|t| t.name == "My starter v1.2"), "the template is saved under the chosen file name");
    assert!(destination.is_file(), "the save dialog's chosen destination receives the template");
    assert!(h.state().toasts.is_empty(), "no confirmation message");
}

#[test]
fn reset_ui_and_loading_a_layout_keep_the_remembered_template() {
    let mut h = app();
    h.state_mut().ui.last_template = "Unlit - Emissive".into();
    h.state_mut().ui.adopt_layout(texturecraft_ui_egui::state::UiState::default());
    assert_eq!(h.state().ui.last_template, "Unlit - Emissive");
}

#[test]
fn a_saved_template_appears_in_new_project_with_the_your_assets_suffix() {
    let mut h = app();
    h.state_mut().run("project.template.save", serde_json::json!({"name": "My starter"})).unwrap();
    h.state_mut().ui.windows.new_project = true;
    h.run_steps(4);

    h.get_by_value("Default (OpenPBR)").click();
    h.run_steps(3);
    assert!(h.query_by_label("My starter (your_assets)").is_some());
    assert!(h.query_by_label("PBR - Specular Glossiness (starter_assets)").is_some());
}

#[test]
fn save_and_reduce_file_size_is_greyed_with_the_hint() {
    let mut h = app();
    open_more_save_options(&mut h);
    let item = h.get_by_label_contains("Save and reduce file size");
    let at = item.rect().center();
    h.hover_at(at);
    h.run_steps(30);
    assert!(h.query_by_label(NOT_AVAILABLE).is_some(), "the hint shows on hover");
    assert!(h.get_by_label_contains("Save and reduce file size").accesskit_node().is_disabled(), "the item is greyed");
    assert!(!menus::enabled(h.state(), "ui.saveReduced"));
    assert!(!menus::implemented("ui.saveReduced"));
}

#[test]
fn the_project_template_info_tooltip_lists_the_settings_templates_apply() {
    let mut h = app();
    h.state_mut().ui.windows.new_project = true;
    h.run_steps(4);
    let row = h.get_by_label("Project template").rect().center().y;
    let icon = h
        .get_all_by_label(egui_phosphor::regular::INFO)
        .find(|node| (node.rect().center().y - row).abs() < 6.0)
        .expect("the info icon in the Project template row");

    // The icon's node is wider than the glyph: hover its left part
    let at = icon.rect().center() - egui::vec2(6.0, 0.0);
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run_steps(2);
    h.hover_at(at);
    h.run_steps(30);
    let tip = "Templates set the starting channels, resolution, normal map format and shader settings.";
    assert!(h.query_by_label(tip).is_some(), "hovering the info icon of the Project template row");
}

#[test]
fn new_project_opens_on_the_template_of_the_last_project() {
    let dir = TempDir::new("last-template");
    let mesh = dir.join("quad.obj");
    std::fs::write(&mesh, "v 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n").unwrap();
    let mut h = app();
    h.state_mut().run("project.template.save", serde_json::json!({"name": "My starter"})).unwrap();
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "ui.newProject");
    h.run_steps(3);
    assert_eq!(h.state().ui.new_project.template, "PBR - Metallic Roughness", "nothing remembered yet: the default");
    h.state_mut().ui.new_project.template = "My starter".into();
    h.state_mut().ui.new_project.file = Some(mesh.display().to_string());
    h.state_mut().session.modified = false;
    h.run_steps(2);
    h.get_by_role_and_label(egui::Role::Button, "Create").click();
    h.run_steps(4);
    assert!(!h.state().ui.windows.new_project, "the project was created");
    assert_eq!(h.state().ui.last_template, "My starter");

    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "ui.newProject");
    h.run_steps(4);
    assert_eq!(h.state().ui.new_project.template, "My starter");
    assert!(h.query_by_value("My starter (your_assets)").is_some());
}
