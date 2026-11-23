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
    harness.state_mut().ui.mesh_wireframe = true;
    harness.state_mut().ui.mode = Mode::Rendering;
    harness.run_steps(2);

    layouts_menu(&mut harness, "No shelf");

    let ui = &harness.state().ui;
    assert!(!ui.dock.is_open(Panel::Assets), "the saved panels are back");
    assert_eq!((ui.uv_zoom, ui.mesh_wireframe, ui.mode), (4.0, true, Mode::Rendering), "zoom, wireframe and mode stay");
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

fn invoke(harness: &mut Harness<'_, TextureCraftApp>, params: serde_json::Value) -> serde_json::Value {
    let context = harness.ctx.clone();
    texturecraft_ui_egui::control::handle(harness.state_mut(), &context, "ui.menu.invoke", &params)
}

#[test]
fn the_layout_items_are_in_the_menu_catalog_and_open_their_dialogs() {
    use texturecraft_ui_egui::menus::CATALOG;
    use texturecraft_ui_egui::named_layouts::LayoutDialog;

    for id in ["ui.layout.saveAs", "ui.layout.manage"] {
        assert!(CATALOG.iter().any(|item| item.id == id && item.menu == "Window" && item.sub == Some("Layouts")), "{id}");
    }
    let mut harness = app();
    assert!(!texturecraft_ui_egui::menus::enabled(harness.state(), "ui.layout.manage"), "nothing to manage yet");

    assert_eq!(invoke(&mut harness, serde_json::json!({"id": "ui.layout.saveAs"}))["ok"], true);
    assert!(matches!(harness.state().ui.layout_dialog, Some(LayoutDialog::Save(_))));

    harness.state_mut().ui.save_named_layout("A");
    harness.state_mut().ui.layout_dialog = None;
    assert_eq!(invoke(&mut harness, serde_json::json!({"id": "ui.layout.manage"}))["ok"], true);
    assert!(matches!(harness.state().ui.layout_dialog, Some(LayoutDialog::Manage { .. })));
}

#[test]
fn the_control_channel_switches_to_a_layout_by_name_and_rejects_unknown_names() {
    let mut harness = app();
    harness.state_mut().ui.dock.close(Panel::Assets);
    harness.state_mut().ui.save_named_layout("No shelf");
    harness.state_mut().ui.dock.focus(Panel::Assets);
    harness.state_mut().ui.uv_zoom = 2.0;

    let reply = invoke(&mut harness, serde_json::json!({"id": "ui.layout.apply", "name": "No shelf"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!harness.state().ui.dock.is_open(Panel::Assets));
    assert_eq!(harness.state().ui.uv_zoom, 2.0);

    harness.state_mut().ui.dock.focus(Panel::Assets);
    for params in [serde_json::json!({"id": "ui.layout.apply", "name": "Nope"}), serde_json::json!({"id": "ui.layout.apply"})] {
        let reply = invoke(&mut harness, params);
        assert_eq!(reply["ok"], false, "{reply}");
    }
    assert!(harness.state().ui.dock.is_open(Panel::Assets), "an unknown name changes nothing");
}

#[test]
fn tab_moves_from_the_save_field_to_the_buttons_and_stays_there() {
    let mut harness = app();
    layouts_menu(&mut harness, "Save current layout as...");
    dialog_input(&harness).type_text("Mine");
    harness.run_steps(2);
    harness.key_press(egui::Key::Tab);
    harness.run_steps(30);
    assert!(harness.get_by_role_and_label(Role::Button, "Save").is_focused(), "the field did not take the focus back");
}

#[test]
fn tab_moves_from_the_rename_field_to_the_buttons_and_stays_there() {
    let mut harness = app();
    save_as(&mut harness, "First");
    layouts_menu(&mut harness, "Manage layouts...");
    harness.get_by_role_and_label(Role::Button, "Rename").click();
    harness.run_steps(2);
    dialog_input(&harness).type_text("2");
    harness.run_steps(2);
    harness.key_press(egui::Key::Tab);
    harness.run_steps(30);
    assert!(harness.get_by_role_and_label(Role::Button, "Rename").is_focused(), "the field did not take the focus back");
}

#[test]
fn a_second_save_dialog_focuses_its_field_again() {
    let mut harness = app();
    save_as(&mut harness, "First");
    layouts_menu(&mut harness, "Save current layout as...");
    harness.run_steps(3);
    assert!(dialog_input(&harness).is_focused(), "the field is ready for typing");
}

#[test]
fn a_long_list_of_layouts_scrolls_and_keeps_close_on_screen() {
    let mut harness = app();
    for n in 0..60 {
        harness.state_mut().ui.save_named_layout(&format!("Layout {n}"));
    }
    layouts_menu(&mut harness, "Manage layouts...");
    harness.run_steps(3);
    let screen = harness.ctx.content_rect();
    let close = harness.get_by_role_and_label(Role::Button, "Close").rect();
    assert!(screen.contains_rect(close), "Close is inside the window: {close:?} in {screen:?}");
}

#[test]
fn a_save_dialog_closed_by_a_reset_still_focuses_its_field_next_time() {
    let mut harness = app();
    layouts_menu(&mut harness, "Save current layout as...");
    harness.run_steps(3);
    assert!(dialog_input(&harness).is_focused());

    // Reset UI closes the dialog from outside
    harness.state_mut().ui.adopt_layout(texturecraft_ui_egui::state::UiState::default());
    harness.run_steps(3);
    assert!(harness.state().ui.layout_dialog.is_none());

    layouts_menu(&mut harness, "Save current layout as...");
    harness.run_steps(3);
    assert!(dialog_input(&harness).is_focused(), "the field is ready for typing again");
}
