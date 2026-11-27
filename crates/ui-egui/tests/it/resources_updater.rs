//! The Resources Updater window, driven from the keyboard: the shelf button opens it, outdated
//! resources come first, Update reads a changed file again as one undo step and Select new resource
//! follows another file. File pickers are answered by the override, never shown.

use std::path::{Path, PathBuf};

use egui::{Key, Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::Session;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::{Answer, Request};
use texturecraft_ui_egui::state::ViewLayout;

use super::temp_dir::TempDir;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = Session::new_project("Test", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    h
}

/// Write a `size` × `size` PNG (each size is a different file size, so a change is seen at once)
fn png(dir: &Path, name: &str, size: u32) -> PathBuf {
    let file = dir.join(name);
    image::RgbaImage::from_pixel(size, size, image::Rgba([10, 200, 30, 255])).save(&file).unwrap();

    file
}

fn import(h: &mut Harness<'_, TextureCraftApp>, file: &Path) {
    h.state_mut().run("resource.import", json!({ "path": file.to_str().unwrap() })).unwrap();
    h.run_steps(2);
}

fn press(h: &mut Harness<'_, TextureCraftApp>, key: Key) {
    h.key_press(key);
    h.run_steps(3);
}

/// Move the keyboard focus to button `label` and press Enter
fn activate(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).focus();
    h.run_steps(2);
    press(h, Key::Enter);
}

fn shows(h: &Harness<'_, TextureCraftApp>, text: &str) -> bool {
    h.query_by_label(text).is_some()
}

fn width_of(h: &Harness<'_, TextureCraftApp>, name: &str) -> u32 {
    h.state().session.images[name].width
}

#[test]
fn the_window_opens_from_the_keyboard_and_updates_an_outdated_resource_as_one_undo_step() {
    let dir = TempDir::new("resources-updater-update");
    let (kept, changed) = (png(&dir, "aa_kept.png", 4), png(&dir, "zz_changed.png", 4));
    let mut h = app();
    import(&mut h, &kept);
    import(&mut h, &changed);
    png(&dir, "zz_changed.png", 8);

    activate(&mut h, "Resources Updater");

    let rows = &h.state().ui.windows.resources_updater.as_ref().expect("the window is open").rows;
    assert_eq!(rows[0].name, "zz_changed.png", "outdated resources come first, before names that sort earlier");
    assert!(shows(&h, "Project : Test"));
    assert_eq!(h.get_all_by_label("Shelf : project").count(), 2, "the reference's shelf line, one per row");
    assert!(shows(&h, "(2 resources, 1 outdated)"));
    assert!(shows(&h, "Outdated"), "the status is written out, not only coloured");
    assert!(h.get_by_role_and_label(Role::Button, "Update aa_kept.png").accesskit_node().is_disabled(), "only outdated resources update");

    let steps = h.state().session.history_labels().len();
    activate(&mut h, "Update zz_changed.png");

    assert_eq!(width_of(&h, "zz_changed.png"), 8, "the new file's pixels are in the project");
    assert_eq!(h.state().session.history_labels().len(), steps + 1, "one undo step");
    assert!(shows(&h, "(2 resources, 0 outdated)"));
    let result = h.get_by_label("Updated 1 resources");
    assert_eq!(result.accesskit_node().live(), egui::accesskit::Live::Polite, "the update result is announced away from focus");

    // Undo brings the old pixels back, so Refresh shows the file as changed again
    h.state_mut().run("edit.undo", json!({})).unwrap();
    activate(&mut h, "Refresh resources");

    assert_eq!(width_of(&h, "zz_changed.png"), 4);
    assert!(shows(&h, "(2 resources, 1 outdated)"));

    activate(&mut h, "Update All");
    assert_eq!(width_of(&h, "zz_changed.png"), 8);

    // Update All went grey with nothing left to update; the keyboard moves on to another button of the window
    h.get_by_role_and_label(Role::Button, "Select new resource aa_kept.png").focus();
    h.run_steps(2);
    press(&mut h, Key::Escape);
    assert!(h.state().ui.windows.resources_updater.is_none(), "Escape closes the window while it has the keyboard");
}

#[test]
fn select_new_resource_follows_the_chosen_file_and_the_name_filter_narrows_the_list() {
    let dir = TempDir::new("resources-updater-relink");
    let (first, second) = (png(&dir, "first.png", 4), png(&dir, "second.png", 4));
    let other = png(&dir, "other.png", 16);
    let mut h = app();
    import(&mut h, &first);
    import(&mut h, &second);
    activate(&mut h, "Resources Updater");

    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_: &Request| Answer::Paths(vec![other.clone()]))));
    activate(&mut h, "Select new resource first.png");
    h.run_steps(3);

    assert_eq!(width_of(&h, "first.png"), 16, "the resource keeps its name and takes the chosen file's pixels");
    let source = &h.state().session.doc.project.resource_sources["first.png"].path;
    assert!(source.ends_with("other.png"));
    let announcement = format!("first.png now reads {source}");
    let result = h.get_by_label(&announcement);
    assert_eq!(result.accesskit_node().live(), egui::accesskit::Live::Polite, "the relink result is announced away from focus");

    h.get_by_role_and_label(Role::TextInput, "Name filter:").focus();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Name filter:").type_text("SEC");
    h.run_steps(3);

    assert!(h.query_by_label("Update first.png").is_none(), "the filter hides other names");
    assert!(h.query_by_label("Update second.png").is_some(), "names match ignoring case");
}

#[test]
fn show_in_folder_shows_the_folder_of_the_resource_file_and_materials_have_no_file() {
    let dir = TempDir::new("resources-updater-folder");
    let file = png(&dir, "bricks.png", 4);
    let mut h = app();
    import(&mut h, &file);

    // A linked MaterialCraft material keeps its package as a resource without a file
    let layer = h.state_mut().run("layer.add.fill", json!({})).unwrap()["layer"].as_u64().unwrap();
    let session = &mut h.state_mut().session;
    session.doc.resources.insert("brick.mcmat".to_string(), std::sync::Arc::new(b"package".to_vec()));
    let link = serde_json::from_value(json!({"name": "Brick", "package": "brick.mcmat", "graph": "Brick", "params": [], "size": 4})).unwrap();
    if let texturecraft_engine::core::LayerKind::Fill(fill) = &mut session.doc.project.layers.get_mut(&texturecraft_engine::core::LayerId(layer)).unwrap().kind
    {
        fill.link = Some(link);
    }
    activate(&mut h, "Resources Updater");

    activate(&mut h, "Show in folder bricks.png");

    let shown = h.state().file_dialogs.shown_folders().to_vec();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].file_name(), dir.file_name(), "the file's own folder");
    assert!(shows(&h, "Name : brick.mcmat"), "the material is listed");
    assert!(h.query_by_label("Select new resource brick.mcmat").is_none(), "a material cannot point at an image file");
    assert!(h.query_by_label("Show in folder brick.mcmat").is_none(), "it has no file");
    assert!(h.get_by_label("Update brick.mcmat").accesskit_node().is_disabled());
    assert!(
        h.get_all_by_label("Material").any(|node| node.accesskit_node().description().is_some_and(|text| text.contains("stored in the project"))),
        "the material status explains the stored package in the accessibility tree"
    );
}

#[test]
fn escape_cancels_a_stroke_while_the_window_is_open_without_the_keyboard() {
    const FRAMES_PER_DRAG_EVENT: usize = 2;

    let mut h = app();
    h.state_mut().run("layer.add.paint", json!({})).unwrap();
    h.state_mut().ui.layout = ViewLayout::Only3D;
    h.run_steps(3);
    let center = h.state().viewport_rect.center();
    h.drag_at(center);
    h.run_steps(FRAMES_PER_DRAG_EVENT);
    h.hover_at(center + vec2(20.0, 0.0));
    h.run_steps(FRAMES_PER_DRAG_EVENT);
    assert!(h.state().session.stroke_active());

    // Opened from the shelf mid-stroke, the keyboard stays on the shelf button
    activate(&mut h, "Resources Updater");
    assert!(h.state().ui.windows.resources_updater.is_some());
    assert!(h.state().session.stroke_active());

    press(&mut h, Key::Escape);

    assert!(!h.state().session.stroke_active(), "Escape cancels the stroke");
    assert!(h.state().ui.windows.resources_updater.is_some(), "the window keeps open: its controls do not have the keyboard");
    h.drop_at(center + vec2(30.0, 0.0));
    h.run_steps(3);
}

#[test]
fn a_file_changed_while_the_window_stays_open_shows_without_refresh() {
    /// Steps of the harness's quarter second: over the window's half-second check
    const FRAMES: usize = 4;

    let dir = TempDir::new("resources-updater-idle");
    let file = png(&dir, "bricks.png", 4);
    let mut h = app();
    import(&mut h, &file);
    activate(&mut h, "Resources Updater");
    assert!(shows(&h, "(1 resources, 0 outdated)"));

    png(&dir, "bricks.png", 8);
    h.run_steps(FRAMES);

    let summary = h.get_by_label("(1 resources, 1 outdated)");
    assert_eq!(summary.accesskit_node().live(), egui::accesskit::Live::Polite, "an idle change is announced away from keyboard focus");
}

#[test]
fn the_keyboard_focus_stays_with_its_resource_when_the_rows_reorder() {
    /// Steps of the harness's quarter second: over the window's half-second check
    const FRAMES: usize = 4;
    const SELECT_LATER: &str = "Select new resource zz_later.png";

    let dir = TempDir::new("resources-updater-focus");
    let (earlier, later) = (png(&dir, "aa_earlier.png", 4), png(&dir, "zz_later.png", 4));
    let mut h = app();
    import(&mut h, &earlier);
    import(&mut h, &later);
    activate(&mut h, "Resources Updater");
    h.get_by_role_and_label(Role::Button, SELECT_LATER).focus();
    h.run_steps(2);

    // The changed file moves to the top of the list
    png(&dir, "zz_later.png", 8);
    h.run_steps(FRAMES);

    let rows = &h.state().ui.windows.resources_updater.as_ref().unwrap().rows;
    assert_eq!(rows[0].name, "zz_later.png", "the outdated resource moved up");
    assert!(h.get_by_role_and_label(Role::Button, SELECT_LATER).accesskit_node().is_focused(), "the focus moved with it");
}
