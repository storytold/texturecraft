//! Edit > Project Configuration...: the 3D mesh section. File pickers are answered by the
//! override, never shown.

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::Session;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::{Answer, Request};

mod temp_dir;
use temp_dir::TempDir;

const QUAD: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n";
const TRIANGLE: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n";
/// The format combo's tooltip, which is its accessible label.
const FORMAT_HINT: &str = "For the whole project: how normal map images set as mesh maps are read (set them again after a change), and the default for export";
const TITLE: &str = "Project configuration";

fn temp_dir(test: &str) -> TempDir {
    TempDir::new(test)
}

/// An app whose project is the quad in `dir/quad.obj` with a fill layer on it.
fn app_on_a_quad(dir: &std::path::Path) -> Harness<'static, TextureCraftApp> {
    let file = dir.join("quad.obj");
    std::fs::write(&file, QUAD).unwrap();
    let mut session = Session::from_mesh_bytes("quad.obj", QUAD.as_bytes(), 128, "quad").unwrap();
    texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap();
    session.doc.project.mesh_path = file.display().to_string();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);

    h
}

/// The widget of `role` and `label` inside the window, which sits at the middle of the screen
/// (a panel of the dock may show the same text).
fn in_window<'h>(h: &'h Harness<'_, TextureCraftApp>, role: Role, label: &'h str) -> egui_kittest::Node<'h> {
    let middle = egui::pos2(800.0, 480.0);

    h.get_all_by_role_and_label(role, label)
        .min_by(|a, b| a.rect().center().distance(middle).total_cmp(&b.rect().center().distance(middle)))
        .expect("the widget in the window")
}

fn click_in_window(h: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    in_window(h, role, label).click();
    h.run_steps(3);
}

fn open_window(h: &mut Harness<'_, TextureCraftApp>) {
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "ui.projectConfig");
    h.run_steps(3);
}

fn window_is_open(h: &Harness<'_, TextureCraftApp>) -> bool {
    h.query_by_label(TITLE).is_some()
}

fn click(h: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(3);
}

/// Select... answered with `file`.
fn select(h: &mut Harness<'_, TextureCraftApp>, file: &std::path::Path) {
    let file = file.to_path_buf();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_: &Request| Answer::Paths(vec![file.clone()]))));
    click(h, Role::Button, "Select...");
}

/// The Reimport mesh box as the window shows it.
fn reimport_ticked(h: &Harness<'_, TextureCraftApp>) -> bool {
    h.get_by_role_and_label(Role::CheckBox, "Reimport mesh").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// The tick the user made themselves, as stored.
fn own_tick(h: &Harness<'_, TextureCraftApp>) -> bool {
    h.state().ui.windows.project_config.as_ref().is_some_and(|c| c.reimport)
}

#[test]
fn the_menu_opens_the_window() {
    let dir = temp_dir("menu");
    let mut h = app_on_a_quad(&dir);

    assert!(!window_is_open(&h));

    open_window(&mut h);

    assert!(window_is_open(&h));
    assert!(h.query_by_label("3D MESH").is_some(), "the 3D mesh section");
    assert!(!reimport_ticked(&h), "nothing is ticked at first");
    assert!(h.state().ui.rail_panel.is_none(), "the Display panel stays where it was");
}

#[test]
fn many_texture_sets_keep_dialog_actions_reachable_in_a_short_viewport() {
    let dir = temp_dir("many-tiles");
    let mut h = app_on_a_quad(&dir);
    let template = h.state().session.doc.project.texture_sets[0].clone();
    h.state_mut().session.doc.project.texture_sets = (0..30)
        .map(|index| {
            let mut set = template.clone();
            set.name = format!("Set {index}");
            set.uv_tiles = vec![1001];
            set
        })
        .collect();
    h.set_size(egui::vec2(800.0, 480.0));

    open_window(&mut h);
    h.get_by_role_and_label(Role::Button, "Cancel").scroll_to_me();
    h.run_steps(3);

    let ok = h.get_by_role_and_label(Role::Button, "OK");
    assert!(ok.rect().top() >= 0.0 && ok.rect().bottom() <= 480.0, "OK must be inside the viewport after scrolling: {:?}", ok.rect());
    let cancel = h.get_by_role_and_label(Role::Button, "Cancel");
    assert!(cancel.rect().top() >= 0.0 && cancel.rect().bottom() <= 480.0, "Cancel must be inside the viewport after scrolling: {:?}", cancel.rect());
    cancel.click();
    h.run_steps(3);
    assert!(!window_is_open(&h));
}

#[test]
fn every_setting_has_an_info_icon_and_the_window_ends_with_a_note() {
    let dir = temp_dir("info");
    let mut h = app_on_a_quad(&dir);

    open_window(&mut h);

    let icons: Vec<_> = h.get_all_by_label(egui_phosphor::regular::INFO).collect();
    let tip = "Green channel convention of normal maps: OpenGL (Y+) or DirectX (Y-). Imported and exported normal maps follow it.";

    assert!(h.query_by_label("Choosing or reimporting a mesh recomputes all channels.").is_some(), "the note above the buttons");
    assert_eq!(icons.len(), 6, "mesh path, Select, Reimport mesh, Preserve strokes, Normal map format, UV tiles");
    assert!(h.query_by_label(tip).is_none(), "no tooltip before hovering");

    // The format icon is fifth in the window's rows
    let mut centers: Vec<_> = icons.iter().map(|n| n.rect().center()).collect();
    centers.sort_by(|a, b| a.y.total_cmp(&b.y));
    let format_center = centers[4];
    h.hover_at(format_center);
    h.run_steps(30);

    assert!(h.query_by_label(tip).is_some(), "hovering the normal map format icon explains it");
}

#[test]
fn another_file_shows_its_path_and_ticks_reimport() {
    let dir = temp_dir("select");
    let mut h = app_on_a_quad(&dir);
    let tri = dir.join("tri.obj");
    std::fs::write(&tri, TRIANGLE).unwrap();

    open_window(&mut h);
    select(&mut h, &tri);

    assert!(h.query_by_label_contains("tri.obj").is_some(), "the chosen file is shown");
    assert!(reimport_ticked(&h));
    assert!(!own_tick(&h), "shown ticked, not stored as the user's tick");
    assert_eq!(h.state().session.mesh.triangle_count(), 2, "nothing changes before OK");
}

#[test]
fn a_delayed_selection_does_not_change_a_reopened_window() {
    let dir = temp_dir("stale-selection");
    let mut harness = app_on_a_quad(&dir);
    let triangle = dir.join("tri.obj");
    std::fs::write(&triangle, TRIANGLE).unwrap();
    open_window(&mut harness);

    harness.state_mut().file_dialogs.set_override(Some(Box::new(move |_: &Request| Answer::Paths(vec![triangle.clone()]))));
    harness.get_by_role_and_label(Role::Button, "Select...").click();
    harness.run_steps(1);

    assert_eq!(harness.state().file_dialogs.pending(), 1, "the answer has not been applied yet");

    harness.state_mut().ui.windows.project_config = None;
    open_window(&mut harness);

    assert!(harness.state().ui.windows.project_config.as_ref().is_some_and(|config| config.mesh.is_none()));
    assert_eq!(harness.state().file_dialogs.pending(), 0, "the old answer was released");

    let replacement = dir.join("other.obj");
    std::fs::write(&replacement, TRIANGLE).unwrap();
    select(&mut harness, &replacement);

    assert_eq!(harness.state().ui.windows.project_config.as_ref().and_then(|config| config.mesh.as_deref()), replacement.to_str());
}

#[test]
fn a_delayed_selection_does_not_change_a_window_reopened_after_layout_reset() {
    let dir = temp_dir("reset-layout-selection");
    let mut harness = app_on_a_quad(&dir);
    let triangle = dir.join("tri.obj");
    std::fs::write(&triangle, TRIANGLE).unwrap();
    open_window(&mut harness);

    harness.state_mut().file_dialogs.set_override(Some(Box::new(move |_: &Request| Answer::Paths(vec![triangle.clone()]))));
    harness.get_by_role_and_label(Role::Button, "Select...").click();
    harness.run_steps(1);

    assert_eq!(harness.state().file_dialogs.pending(), 1);

    let context = harness.ctx.clone();
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "ui.resetUi");
    harness.run_steps(2);
    harness.get_by_label("Reset").click();
    harness.run_steps(2);
    open_window(&mut harness);

    assert!(harness.state().ui.windows.project_config.as_ref().is_some_and(|config| config.mesh.is_none()));
    assert_eq!(harness.state().file_dialogs.pending(), 0);
}

#[test]
fn ok_moves_the_project_to_the_chosen_mesh_and_keeps_the_layers() {
    let dir = temp_dir("ok");
    let mut h = app_on_a_quad(&dir);
    let tri = dir.join("tri.obj");
    std::fs::write(&tri, TRIANGLE).unwrap();
    let layers = h.state().session.doc.project.layers.len();

    open_window(&mut h);
    select(&mut h, &tri);
    click(&mut h, Role::Button, "OK");
    let session = &h.state().session;

    assert_eq!(session.mesh.triangle_count(), 1, "the new mesh is in use");
    assert_eq!(session.doc.project.mesh_file, "tri.obj");
    assert!(session.doc.project.mesh_path.ends_with("tri.obj"), "Reimport mesh now follows the new file");
    assert_eq!(session.doc.project.layers.len(), layers, "the layers stay");
    assert!(!window_is_open(&h));
}

#[test]
fn ticking_reimport_alone_reads_the_same_file_again() {
    let dir = temp_dir("reimport");
    let mut h = app_on_a_quad(&dir);
    let project = h.state().session.project_id;
    let layers = h.state().session.doc.project.layers.len();

    // The file changed on disk since it was imported
    std::fs::write(dir.join("quad.obj"), TRIANGLE).unwrap();

    open_window(&mut h);
    click(&mut h, Role::CheckBox, "Reimport mesh");

    assert!(reimport_ticked(&h) && own_tick(&h));

    click(&mut h, Role::Button, "OK");
    let session = &h.state().session;

    assert_eq!(session.mesh.triangle_count(), 1, "the edited file was read again");
    assert_eq!(session.project_id, project);
    assert_eq!(session.doc.project.layers.len(), layers);
    assert!(!window_is_open(&h));
}

#[test]
fn ok_with_nothing_chosen_changes_nothing() {
    let dir = temp_dir("nothing");
    let mut h = app_on_a_quad(&dir);
    std::fs::write(dir.join("quad.obj"), TRIANGLE).unwrap();
    let mesh = h.state().session.mesh_id;

    open_window(&mut h);
    click(&mut h, Role::Button, "OK");

    assert_eq!(h.state().session.mesh_id, mesh);
    assert_eq!(h.state().session.mesh.triangle_count(), 2);
    assert!(!window_is_open(&h));
}

#[test]
fn cancel_changes_nothing() {
    let dir = temp_dir("cancel");
    let mut h = app_on_a_quad(&dir);
    let tri = dir.join("tri.obj");
    std::fs::write(&tri, TRIANGLE).unwrap();
    let mesh = h.state().session.mesh_id;

    open_window(&mut h);
    select(&mut h, &tri);
    click(&mut h, Role::Button, "Cancel");

    assert!(!window_is_open(&h));
    assert_eq!(h.state().session.mesh_id, mesh);
    assert_eq!(h.state().session.mesh.triangle_count(), 2);

    // Opened again, the old choice is gone
    open_window(&mut h);

    assert!(!reimport_ticked(&h));
    assert!(h.state().ui.windows.project_config.as_ref().is_some_and(|c| c.mesh.is_none()));
}

#[test]
fn escape_closes_the_window_and_leaves_a_stroke_alone() {
    let dir = temp_dir("escape");
    let mut h = app_on_a_quad(&dir);

    assert_eq!(h.state().ui.mode, texturecraft_ui_egui::state::Mode::Painting, "Painting is where Escape cancels strokes");

    texturecraft_engine::execute(&mut h.state_mut().session, "layer.add.paint", &json!({})).unwrap();
    h.state_mut().session.begin_stroke_uv(texturecraft_engine::geom::glam::Vec2::splat(0.5), 1.0).unwrap();

    open_window(&mut h);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    assert!(!window_is_open(&h));
    assert!(h.state().session.stroke_active(), "Escape went to the window, not to the stroke");
}

#[test]
fn escape_with_the_dropdown_open_closes_only_the_dropdown() {
    let dir = temp_dir("escape-dropdown");
    let mut h = app_on_a_quad(&dir);

    open_window(&mut h);
    click_in_window(&mut h, Role::ComboBox, FORMAT_HINT);

    assert!(egui::Popup::is_any_open(&h.ctx), "the dropdown is open");

    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    assert!(!egui::Popup::is_any_open(&h.ctx), "Escape closed the dropdown");
    assert!(window_is_open(&h), "and left the window");

    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    assert!(!window_is_open(&h), "the next Escape closes the window");
}

#[test]
fn enter_with_the_dropdown_open_does_not_press_ok() {
    let dir = temp_dir("enter-dropdown");
    let mut h = app_on_a_quad(&dir);
    std::fs::write(dir.join("quad.obj"), TRIANGLE).unwrap();

    open_window(&mut h);
    h.state_mut().ui.windows.project_config.as_mut().unwrap().reimport = true;
    click_in_window(&mut h, Role::ComboBox, FORMAT_HINT);

    // Nothing focused, as the shortcut handler needs for Enter to mean OK
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.run_steps(1);

    assert!(h.ctx.memory(|m| m.focused().is_none()), "nothing has the keyboard");
    assert!(egui::Popup::is_any_open(&h.ctx), "the dropdown is still open");

    h.key_press(egui::Key::Enter);
    h.run_steps(3);

    assert!(window_is_open(&h), "Enter did not press OK behind the dropdown");
    assert_eq!(h.state().session.mesh.triangle_count(), 2, "nothing was applied");
}

#[test]
fn ok_after_the_ticked_file_went_missing_reports_it_and_stays_open() {
    let dir = temp_dir("ticked-then-missing");
    let mut h = app_on_a_quad(&dir);

    open_window(&mut h);
    click_in_window(&mut h, Role::CheckBox, "Reimport mesh");

    assert!(own_tick(&h));

    std::fs::remove_file(dir.join("quad.obj")).unwrap();
    h.run_steps(2);
    click(&mut h, Role::Button, "OK");

    assert!(window_is_open(&h), "the window stays open");
    assert!(h.state().toasts.iter().any(|t| t.error && t.text == "File not found"), "the error shows as a toast");
    assert!(!own_tick(&h), "the tick that can't be applied is dropped");
    assert_eq!(h.state().session.mesh.triangle_count(), 2);

    click(&mut h, Role::Button, "OK");

    assert!(!window_is_open(&h), "a second OK has nothing left to read");
}

#[test]
fn enter_confirms_but_not_a_text_fields_enter() {
    let dir = temp_dir("enter");
    let mut h = app_on_a_quad(&dir);
    std::fs::write(dir.join("quad.obj"), TRIANGLE).unwrap();

    open_window(&mut h);

    // Ticked without a click, which would leave the box focused
    h.state_mut().ui.windows.project_config.as_mut().unwrap().reimport = true;

    // Renaming a layer: Enter belongs to the text field
    let layer = h.state().session.selected_layer().unwrap().0;
    let name = h.state().session.doc.project.layers.values().find(|l| l.id.0 == layer).unwrap().name.clone();
    h.state_mut().ui.rename = Some((layer, name));
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);

    assert!(h.state().ui.rename.is_none(), "Enter ended the rename");
    assert!(window_is_open(&h), "and did not press OK");
    assert_eq!(h.state().session.mesh.triangle_count(), 2);

    // With nothing focused, Enter is OK
    h.key_press(egui::Key::Enter);
    h.run_steps(3);

    assert!(!window_is_open(&h));
    assert_eq!(h.state().session.mesh.triangle_count(), 1, "the tick was applied");
}

#[test]
fn picking_the_projects_own_file_again_drops_the_choice_and_its_tick() {
    let dir = temp_dir("own");
    let mut h = app_on_a_quad(&dir);
    let tri = dir.join("tri.obj");
    std::fs::write(&tri, TRIANGLE).unwrap();
    std::fs::write(dir.join("quad.obj"), TRIANGLE).unwrap();
    let mesh = h.state().session.mesh_id;

    open_window(&mut h);
    select(&mut h, &tri);
    let own = h.state().session.doc.project.mesh_path.clone();
    select(&mut h, std::path::Path::new(&own));

    assert!(!reimport_ticked(&h), "the tick was only ever the file's");

    click(&mut h, Role::Button, "OK");

    assert_eq!(h.state().session.mesh_id, mesh, "OK does nothing");
    assert_eq!(h.state().session.mesh.triangle_count(), 2);
}

#[test]
fn a_missing_file_cannot_be_reimported() {
    let dir = temp_dir("missing");
    let mut h = app_on_a_quad(&dir);
    std::fs::remove_file(dir.join("quad.obj")).unwrap();

    open_window(&mut h);

    assert!(h.query_by_label("File not found").is_some());
    assert!(h.get_by_role_and_label(Role::CheckBox, "Reimport mesh").accesskit_node().is_disabled());

    click(&mut h, Role::Button, "OK");

    assert_eq!(h.state().file_dialogs.pending(), 0, "no second dialog");
    assert!(!window_is_open(&h));
    assert!(h.state().toasts.is_empty());
}

#[test]
fn a_built_in_mesh_shows_its_name_and_has_nothing_to_reimport() {
    let dir = temp_dir("builtin");
    let mut h = app_on_a_quad(&dir);
    h.state_mut().session.doc.project.mesh_path.clear();

    open_window(&mut h);

    assert!(h.query_by_label("quad.obj").is_some(), "the name it was imported under");
    assert!(h.query_by_label("File not found").is_none());
    assert!(h.get_by_role_and_label(Role::CheckBox, "Reimport mesh").accesskit_node().is_disabled());

    h.state_mut().session.doc.project.mesh_file.clear();
    h.run_steps(2);

    assert!(h.query_by_label("Built-in mesh").is_some());
}

#[test]
fn a_failing_change_is_reported_as_a_toast() {
    let dir = temp_dir("error");
    let mut h = app_on_a_quad(&dir);
    let missing = dir.join("missing.obj");

    open_window(&mut h);
    select(&mut h, &missing);
    click(&mut h, Role::Button, "OK");

    assert!(h.state().toasts.iter().any(|t| t.error), "the error shows as a toast");
    assert!(window_is_open(&h), "the window stays open");
    assert_eq!(h.state().ui.windows.project_config.as_ref().and_then(|c| c.mesh.clone()), Some(missing.display().to_string()), "with the choice kept");
    assert_eq!(h.state().session.mesh.triangle_count(), 2, "the project keeps its mesh");
}

fn project_normal_format(h: &Harness<'_, TextureCraftApp>) -> texturecraft_engine::core::NormalFormat {
    h.state().session.doc.project.normal_format
}

#[test]
fn the_normal_map_format_changes_on_ok_and_shows_in_the_texture_set_panel() {
    use texturecraft_engine::core::NormalFormat;

    let dir = temp_dir("normal-format");
    let mut h = app_on_a_quad(&dir);

    open_window(&mut h);
    click_in_window(&mut h, Role::ComboBox, FORMAT_HINT);
    click_in_window(&mut h, Role::Button, "DirectX");

    assert_eq!(project_normal_format(&h), NormalFormat::OpenGl, "nothing changes before OK");

    click(&mut h, Role::Button, "OK");

    assert_eq!(project_normal_format(&h), NormalFormat::DirectX);

    h.state_mut().ui.dock.focus(texturecraft_ui_egui::docking::Panel::TextureSetSettings);
    h.run_steps(3);

    assert!(h.query_all_by_value("DirectX").next().is_some(), "the Texture Set Settings panel shows the project's format");

    // One undo step puts it back
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);

    assert_eq!(project_normal_format(&h), NormalFormat::OpenGl);
}

#[test]
fn cancel_discards_a_normal_map_format() {
    use texturecraft_engine::core::NormalFormat;

    let dir = temp_dir("normal-format-cancel");
    let mut h = app_on_a_quad(&dir);

    open_window(&mut h);
    click_in_window(&mut h, Role::ComboBox, FORMAT_HINT);
    click_in_window(&mut h, Role::Button, "DirectX");
    click(&mut h, Role::Button, "Cancel");

    assert_eq!(project_normal_format(&h), NormalFormat::OpenGl);
}

#[test]
fn uv_tiles_are_shown_read_only() {
    let dir = temp_dir("udim");
    let mut h = app_on_a_quad(&dir);

    open_window(&mut h);

    assert!(h.query_by_label("UV Tile settings (UDIMs)").is_some());
    assert!(h.query_by_label("No UV tiles").is_some(), "a project without UV tiles");
    assert!(h.query_by_label("Change them in Texture Set Settings").is_some());

    click(&mut h, Role::Button, "Cancel");

    texturecraft_engine::execute(&mut h.state_mut().session, "textureSet.uvTiles", &json!({"ts": 0, "enabled": true})).unwrap();

    open_window(&mut h);

    assert!(h.query_by_label("No UV tiles").is_none());
    assert!(h.query_by_label("1001").is_some(), "the tile list of a UDIM project");
}

#[test]
fn preserving_strokes_is_an_import_setting_of_a_mesh_that_is_read() {
    let dir = temp_dir("preserve");
    let mut h = app_on_a_quad(&dir);
    let tri = dir.join("tri.obj");
    std::fs::write(&tri, TRIANGLE).unwrap();
    let preserve_is_disabled =
        |h: &Harness<'_, TextureCraftApp>| h.get_by_role_and_label(Role::CheckBox, "Preserve stroke positions on mesh").accesskit_node().is_disabled();

    open_window(&mut h);

    assert!(preserve_is_disabled(&h), "greyed while no mesh is going to be read");
    assert!(h.state().ui.windows.project_config.as_ref().unwrap().preserve, "on by default");

    select(&mut h, &tri);

    assert!(!preserve_is_disabled(&h));

    click(&mut h, Role::CheckBox, "Preserve stroke positions on mesh");

    assert!(!h.state().ui.windows.project_config.as_ref().unwrap().preserve);

    click(&mut h, Role::Button, "OK");

    assert_eq!(h.state().session.mesh.triangle_count(), 1, "the mesh changed without reprojecting strokes");
}

/// The same quad with its UVs mirrored in u.
const MIRRORED_QUAD: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 1 0\nvt 0 0\nvt 0 1\nvt 1 1\nf 1/1 2/2 3/3 4/4\n";

/// Paints the half of the quad with the larger x on its first layer.
fn paint_the_right_half(h: &mut Harness<'_, TextureCraftApp>) -> texturecraft_engine::core::LayerId {
    use texturecraft_engine::core::Channel;

    let session = &mut h.state_mut().session;
    let id = session.doc.project.texture_sets[0].stack[0];
    let (size, cells) = {
        let cache = &session.caches[0];
        let xs: Vec<(usize, f32)> =
            (0..cache.size.0 as usize * cache.size.1 as usize).filter_map(|i| cache.gbuf.surface(&session.mesh, i).map(|p| (i, p.pos.x))).collect();
        let (low, high) = xs.iter().fold((f32::MAX, f32::MIN), |(l, h), (_, x)| (l.min(*x), h.max(*x)));
        (cache.size, xs.into_iter().filter(|(_, x)| *x > (low + high) / 2.0).map(|(i, _)| i).collect::<Vec<_>>())
    };

    let image = session.doc.pixels.layer_mut(id, Channel::BaseColor, size);
    for i in cells {
        image.set(i as u32 % size.0, i as u32 / size.0, &[65535, 0, 0, 65535]);
    }

    id
}

/// Painted texels on the right and on the left half of the quad's surface, away from the middle.
fn painted_halves(h: &Harness<'_, TextureCraftApp>, id: texturecraft_engine::core::LayerId) -> (usize, usize) {
    use texturecraft_engine::core::Channel;

    let session = &h.state().session;
    let cache = &session.caches[0];
    let image = session.doc.pixels.layer(id, Channel::BaseColor).unwrap();
    let texels: Vec<(u32, u32, f32)> = (0..cache.size.0 * cache.size.1)
        .filter_map(|i| cache.gbuf.surface(&session.mesh, i as usize).map(|p| (i % cache.size.0, i / cache.size.0, p.pos.x)))
        .collect();
    let (low, high) = texels.iter().fold((f32::MAX, f32::MIN), |(l, h), t| (l.min(t.2), h.max(t.2)));

    // Ignore the center seam where rasterization may place texels on either side
    let (middle, margin) = ((low + high) / 2.0, (high - low) * 0.05);
    let painted = |x: u32, y: u32| image.get(x, y)[3] > 32768;
    let right = texels.iter().filter(|t| t.2 > middle + margin && painted(t.0, t.1)).count();
    let left = texels.iter().filter(|t| t.2 < middle - margin && painted(t.0, t.1)).count();

    (right, left)
}

/// OK on a mirrored-UV mesh with the Preserve box as given; returns the painted (right, left) texels afterwards.
fn painted_after_changing_to_mirrored_uvs(test: &str, preserve: bool) -> ((usize, usize), (usize, usize)) {
    let dir = temp_dir(test);
    let mut h = app_on_a_quad(&dir);
    let mirrored = dir.join("mirrored.obj");
    std::fs::write(&mirrored, MIRRORED_QUAD).unwrap();
    let id = paint_the_right_half(&mut h);
    let before = painted_halves(&h, id);

    open_window(&mut h);
    select(&mut h, &mirrored);
    if !preserve {
        click(&mut h, Role::CheckBox, "Preserve stroke positions on mesh");
    }
    click(&mut h, Role::Button, "OK");

    assert!(!window_is_open(&h));

    (before, painted_halves(&h, id))
}

#[test]
fn preserve_on_moves_the_strokes_with_the_surface() {
    let (before, after) = painted_after_changing_to_mirrored_uvs("preserve-on", true);

    assert!(before.0 > 2000 && before.1 == 0, "painted on the right half: {before:?}");
    assert!(after.0 > 2000 && after.1 < 200, "still on the right half of the surface: {after:?}");
}

#[test]
fn preserve_off_leaves_the_strokes_in_the_uv_layout() {
    let (_, after) = painted_after_changing_to_mirrored_uvs("preserve-off", false);

    assert!(after.1 > 2000 && after.0 < 200, "the paint stayed where it was in UV space, now on the left half: {after:?}");
}

#[test]
fn a_mesh_and_a_format_change_in_one_ok() {
    use texturecraft_engine::core::NormalFormat;

    let dir = temp_dir("both");
    let mut h = app_on_a_quad(&dir);
    let tri = dir.join("tri.obj");
    std::fs::write(&tri, TRIANGLE).unwrap();

    open_window(&mut h);
    select(&mut h, &tri);
    click_in_window(&mut h, Role::ComboBox, FORMAT_HINT);
    click_in_window(&mut h, Role::Button, "DirectX");
    click(&mut h, Role::Button, "OK");

    assert!(!window_is_open(&h));
    assert_eq!(h.state().session.mesh.triangle_count(), 1);
    assert_eq!(project_normal_format(&h), NormalFormat::DirectX);

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);

    assert_eq!(project_normal_format(&h), NormalFormat::OpenGl, "the format is one undo step");
    assert_eq!(h.state().session.mesh.triangle_count(), 1, "and the mesh stays");
}

#[test]
fn a_format_the_project_already_has_is_not_set_again() {
    use texturecraft_engine::core::NormalFormat;

    let dir = temp_dir("same-format");
    let mut h = app_on_a_quad(&dir);

    open_window(&mut h);
    click_in_window(&mut h, Role::ComboBox, FORMAT_HINT);
    click_in_window(&mut h, Role::Button, "DirectX");

    // The Texture Set Settings panel sets it meanwhile
    texturecraft_engine::execute(&mut h.state_mut().session, "textureSet.settings", &json!({"normalFormat": "directX"})).unwrap();
    let steps = h.state().session.history_labels().len();
    click(&mut h, Role::Button, "OK");

    assert_eq!(project_normal_format(&h), NormalFormat::DirectX);
    assert_eq!(h.state().session.history_labels().len(), steps, "no extra undo step");
}
