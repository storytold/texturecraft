//! The Window menu as in the reference: Views (Renderer Settings, Properties - Paint, greyed
//! Python Console and Online assets), Toolbars, Save UI Layout per mode, Load UI Layout, Hide UI's
//! check mark, and the views enabled per mode

use egui::accesskit::Toggled;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::RefCell;
use std::rc::Rc;
use texturecraft_engine::Session;
use texturecraft_ui_egui::file_dialogs::{Answer, Request};
use texturecraft_ui_egui::state::Mode;
use texturecraft_ui_egui::unavailable::NOT_AVAILABLE;
use texturecraft_ui_egui::{TextureCraftApp, menus};

use super::temp_dir::TempDir;

fn app_with(session: Session) -> Harness<'static, TextureCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

fn app() -> Harness<'static, TextureCraftApp> {
    app_with(texturecraft_engine::sample_session(128, 64).unwrap())
}

fn open_window_menu(h: &mut Harness<'_, TextureCraftApp>) {
    h.get_by_label("Window").click();
    h.run_steps(3);
}

fn open_sub(h: &mut Harness<'_, TextureCraftApp>, name: &str) {
    open_window_menu(h);
    h.get_by_label(name).click();
    h.run_steps(3);
}

fn hover_hint(h: &mut Harness<'_, TextureCraftApp>, label: &str) -> bool {
    let at = h.get_by_label_contains(label).rect().center();
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run_steps(2);
    h.hover_at(at);
    h.run_steps(30);
    h.query_by_label(NOT_AVAILABLE).is_some()
}

#[test]
fn views_lists_renderer_settings_and_the_greyed_entries_with_a_hint() {
    let mut h = app();
    open_sub(&mut h, "Views");
    assert!(h.query_by_label_contains("Renderer Settings").is_some());
    for label in ["Python Console", "Online assets"] {
        assert!(h.get_by_label_contains(label).accesskit_node().is_disabled(), "{label} is greyed");
    }
    assert!(hover_hint(&mut h, "Python Console"), "Python Console says it is not available yet");
    assert!(hover_hint(&mut h, "Online assets"), "Online assets says it is not available yet");
}

#[test]
fn properties_names_the_tool_while_a_project_is_open() {
    let mut h = app();
    open_sub(&mut h, "Views");
    assert!(h.query_all_by_label_contains("Properties - Paint").next().is_some());

    let mut closed = app_with(Session::closed().unwrap());
    open_sub(&mut closed, "Views");
    assert!(closed.query_all_by_label_contains("Properties - ").next().is_none());
    assert!(closed.query_all_by_label_contains("Properties").next().is_some());
}

#[test]
fn toolbars_lists_docks_plugins_and_tools() {
    let mut h = app();
    open_sub(&mut h, "Toolbars");
    for label in ["Docks", "Tools"] {
        assert!(!h.get_by_label_contains(label).accesskit_node().is_disabled(), "{label}");
    }
    assert!(h.get_by_label_contains("Plugins").accesskit_node().is_disabled());
    assert!(hover_hint(&mut h, "Plugins"));

    assert!(h.state().ui.toolbar_docks && h.state().ui.toolbar_tools, "both bars start shown");
    h.get_by_label_contains("Docks").click();
    h.run_steps(3);
    assert!(!h.state().ui.toolbar_docks);
    assert!(h.state().ui.toolbar_tools);
}

#[test]
fn switching_the_docks_toolbar_off_hides_the_rail_and_the_tools_toolbar_the_tool_shelf() {
    let mut h = app();
    let rail = |h: &Harness<'_, TextureCraftApp>| h.query_all_by_label_contains("Display Settings").any(|n| n.rect().min.x > 1500.0);
    let shelf = |h: &Harness<'_, TextureCraftApp>| h.query_all_by_label_contains("Eraser").any(|n| n.rect().min.x < 60.0);
    assert!(rail(&h) && shelf(&h), "both bars are shown at first");
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.toolbar.docks");
    menus::invoke(h.state_mut(), &ctx, "ui.toolbar.tools");
    h.run_steps(4);
    assert!(!rail(&h), "the right bar is gone");
    assert!(!shelf(&h), "the tool bar is gone");
}

#[test]
fn hide_ui_shows_its_check_mark() {
    let mut h = app();
    open_window_menu(&mut h);
    assert_eq!(h.get_by_label_contains("Hide UI").accesskit_node().toggled(), Some(Toggled::False));
    assert_eq!(menus::checked(h.state(), "ui.hideUi"), Some(false));
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.hideUi");
    assert_eq!(menus::checked(h.state(), "ui.hideUi"), Some(true), "the check mark follows the state");
}

#[test]
fn save_ui_layout_has_one_entry_per_mode_and_writes_that_modes_layout() {
    let dir = TempDir::new("layout-per-mode");
    for (item, file, mode, baking) in [
        ("Painting mode", "p.json", Mode::Painting, false),
        ("Rendering mode", "r.json", Mode::Rendering, false),
        ("Baking mode", "b.json", Mode::Baking, true),
    ] {
        let mut h = app();
        let path = dir.join(file);
        let target = path.clone();
        h.state_mut().file_dialogs.set_override(Some(Box::new(move |_: &Request| Answer::Paths(vec![target.clone()]))));
        open_sub(&mut h, "Save UI Layout");
        h.get_by_label_contains(item).click();
        h.run_steps(4);

        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{item}: {e}"))).unwrap();
        assert_eq!(saved["mode"], serde_json::to_value(mode).unwrap(), "{item}");
        assert_eq!(saved["dock_is_baking"], baking, "{item}: the file's dock is the mode's own");
        assert_eq!(h.state().ui.mode, Mode::Painting, "saving does not switch the mode");
    }
}

#[test]
fn the_legacy_layout_save_command_saves_the_active_mode() {
    let dir = TempDir::new("legacy-layout-save");
    let path = dir.join("layout.json");
    let target = path.clone();
    let mut h = app();
    h.state_mut().ui.mode = Mode::Baking;
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_: &Request| Answer::Paths(vec![target.clone()]))));
    h.run_steps(3);
    let ctx = h.ctx.clone();

    let reply = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.menu.invoke", &serde_json::json!({"id": "ui.layout.save"}));
    h.run_steps(4);

    assert_eq!(reply["ok"], true);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["mode"], serde_json::to_value(Mode::Baking).unwrap());
    assert_eq!(saved["dock_is_baking"], true);
}

#[test]
fn load_ui_layout_asks_with_the_reference_title() {
    let mut h = app();
    let asked: Rc<RefCell<Vec<Request>>> = Rc::default();
    let seen = asked.clone();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |request: &Request| {
        seen.borrow_mut().push(request.clone());
        Answer::Cancelled
    })));
    open_window_menu(&mut h);
    h.get_by_label("Load UI Layout").click();
    h.run_steps(4);
    let asked = asked.borrow();
    let Some(Request::File(dialog)) = asked.first() else { panic!("a file dialog was asked for") };
    assert_eq!(dialog.title.as_deref(), Some("Select a UI layout file."));
}

#[test]
fn views_are_enabled_per_mode_as_measured() {
    let mut h = app();
    let views = [
        ("ui.view.assets", "Assets"),
        ("ui.view.export", "Export"),
        ("ui.view.layers", "Layers"),
        ("ui.view.properties", "Properties"),
        ("ui.view.shader", "Shader"),
        ("ui.view.textureSetSettings", "Texture Set Settings"),
        ("ui.view.renderer", "Renderer Settings"),
        ("ui.view.display", "Display Settings"),
        ("ui.view.log", "Log"),
        ("ui.view.textureSetList", "Texture Set List"),
        ("ui.view.history", "History"),
    ];
    let enabled_in = |h: &mut Harness<'_, TextureCraftApp>, mode: Mode| -> Vec<&'static str> {
        h.state_mut().ui.mode = mode;
        views.iter().filter(|(id, _)| menus::enabled(h.state(), id)).map(|(_, name)| *name).collect()
    };
    assert_eq!(
        enabled_in(&mut h, Mode::Rendering),
        ["Shader", "Renderer Settings", "Display Settings", "Log", "Texture Set List"],
        "Rendering enables only Display Settings, Log, Renderer Settings, Shader settings and Texture Set List"
    );
    let baking = enabled_in(&mut h, Mode::Baking);
    for name in ["Assets", "Export", "Layers", "Properties", "Shader", "Texture Set Settings", "Renderer Settings"] {
        assert!(!baking.contains(&name), "{name} is greyed in Baking");
    }
    for name in ["Display Settings", "Log", "Texture Set List", "History"] {
        assert!(baking.contains(&name), "{name} stays in Baking");
    }
    let painting = enabled_in(&mut h, Mode::Painting);
    assert!(!painting.contains(&"Renderer Settings"), "Renderer Settings belongs to Rendering");
    for name in ["Assets", "Export", "Layers", "Properties", "Shader", "Texture Set Settings", "History"] {
        assert!(painting.contains(&name), "{name} in Painting");
    }
}
