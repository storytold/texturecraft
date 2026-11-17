//! Window > Layouts: save the panel arrangement under a name and switch to it from the menu.

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_accessibility_check(false).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    harness
}

/// Open Window > Layouts, click `entry`, and let the app answer
fn layouts_menu(harness: &mut Harness<'_, TextureCraftApp>, entry: &str) {
    harness.get_by_role_and_label(Role::Button, "Window").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Layouts").click();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, entry).click();
    harness.run_steps(3);
}

/// The dialog's text field: the one nearest the window center, where dialogs open (the shelf search is another)
fn dialog_input<'a>(harness: &'a Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'a> {
    let center = harness.ctx.content_rect().center();
    harness
        .get_all_by_role(Role::TextInput)
        .min_by(|a, b| a.rect().center().distance(center).total_cmp(&b.rect().center().distance(center)))
        .expect("the dialog's field")
}

fn save_as(harness: &mut Harness<'_, TextureCraftApp>, name: &str) {
    layouts_menu(harness, "Save current layout as...");
    dialog_input(&*harness).type_text(name);
    harness.run_steps(2);
    harness.get_by_role_and_label(Role::Button, "Save").click();
    harness.run_steps(3);
}

#[test]
fn a_layout_saved_from_the_menu_comes_back_from_the_menu() {
    let mut harness = app();
    harness.state_mut().ui.dock.close(Panel::Assets);
    save_as(&mut harness, "No shelf");
    assert!(harness.state().ui.has_named_layout("No shelf"));

    harness.state_mut().ui.dock.focus(Panel::Assets);
    harness.state_mut().ui.uv_zoom = 4.0;
    harness.state_mut().ui.wireframe_3d = true;
    harness.state_mut().ui.mode = Mode::Rendering;
    harness.run_steps(2);

    layouts_menu(&mut harness, "No shelf");

    let ui = &harness.state().ui;
    assert!(!ui.dock.is_open(Panel::Assets), "the saved panels are back");
    assert_eq!((ui.uv_zoom, ui.wireframe_3d, ui.mode), (4.0, true, Mode::Rendering), "zoom, wireframe and mode stay");
}

#[test]
fn saving_under_a_taken_name_asks_before_replacing() {
    let mut harness = app();
    harness.state_mut().ui.dock.close(Panel::Assets);
    save_as(&mut harness, "Mine");
    harness.state_mut().ui.dock.close(Panel::Layers);

    save_as(&mut harness, "Mine");
    assert!(harness.state().ui.named_layouts[0].painting.is_open(Panel::Layers), "nothing is replaced before the answer");
    harness.get_by_label("A layout named Mine already exists. Replace it?");
    harness.get_by_role_and_label(Role::Button, "Replace").click();
    harness.run_steps(3);

    assert_eq!(harness.state().ui.named_layouts.len(), 1);
    assert!(!harness.state().ui.named_layouts[0].painting.is_open(Panel::Layers));
}

#[test]
fn a_layout_is_renamed_and_deleted_from_the_manage_dialog() {
    let mut harness = app();
    save_as(&mut harness, "First");
    layouts_menu(&mut harness, "Manage layouts...");

    harness.get_by_role_and_label(Role::Button, "Rename").click();
    harness.run_steps(2);
    // The field starts with the old name; typing continues it
    dialog_input(&harness).type_text("2");
    harness.run_steps(2);
    harness.get_by_role_and_label(Role::Button, "Rename").click();
    harness.run_steps(3);
    assert!(harness.state().ui.has_named_layout("First2") && !harness.state().ui.has_named_layout("First"));

    harness.get_by_role_and_label(Role::Button, "Delete").click();
    harness.run_steps(3);
    assert_eq!(harness.state().ui.named_layouts.len(), 1, "delete asks first");
    harness.get_by_label("Delete the layout First2?");
    harness.get_by_role_and_label(Role::Button, "Delete").click();
    harness.run_steps(3);
    assert!(harness.state().ui.named_layouts.is_empty());
}
