//! Empty-project menu behavior and the layout restored by Reset UI.
use egui::accesskit::Toggled;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use texturecraft_engine::Session;
use texturecraft_ui_egui::{TextureCraftApp, docking::Panel, menus};

fn app() -> Harness<'static, TextureCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|cc| TextureCraftApp::new(cc, Session::closed().unwrap()));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

#[test]
fn recent_projects_stays_openable_and_clear_tracks_the_list() {
    let mut h = app();
    h.get_by_label("File").click();
    h.run_steps(3);
    let recent = h.get_by_label("Recent Projects");
    assert!(!recent.accesskit_node().is_disabled());
    recent.click();
    h.run_steps(3);
    assert!(h.get_by_label("Clear").accesskit_node().is_disabled());
    h.state_mut().ui.recent.push("/projects/painted-crate.tcraft".into());
    h.run_steps(3);
    assert!(h.query_by_label("/projects/painted-crate.tcraft").is_some());
    assert!(!h.get_by_label("Clear").accesskit_node().is_disabled());
    h.get_by_label("Clear").click();
    h.run_steps(3);
    assert!(h.state().ui.recent.is_empty());
}

#[test]
fn shortcuts_list_opens_settings_on_the_shortcuts_page() {
    let mut h = app();
    h.get_by_label("Help").click();
    h.run_steps(3);
    h.get_by_label("Shortcuts List").click();
    h.run_steps(4);
    assert!(h.state().ui.windows.settings);
    assert!(!h.state().ui.windows.shortcuts);
    assert!(h.query_by_label("Mouse shortcuts").is_some());
    assert!(h.query_by_label("General").is_some());
}

#[test]
fn empty_file_menu_disables_the_save_group_and_opens_import() {
    let mut h = app();
    h.get_by_label("File").click();
    h.run_steps(3);
    assert!(h.get_by_label("More Save Options").accesskit_node().is_disabled());
    let import = h.get_all_by_label("Import Resources").find(|node| node.accesskit_node().toggled().is_none()).unwrap();
    assert!(!import.accesskit_node().is_disabled());
    import.click();
    h.run_steps(4);
    assert!(h.state().ui.windows.import_resources.is_some());
    assert!(h.get_by_label("Import").accesskit_node().is_disabled());
}

#[test]
fn reset_restores_the_empty_workspace_and_checked_disabled_quick_mask() {
    let mut h = app();
    h.state_mut().ui.dock.close(Panel::Assets);
    h.state_mut().ui.dock.focus(Panel::History);
    h.state_mut().ui.toolbar_tools = false;
    h.state_mut().ui.toolbar_docks = false;
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.resetUi");
    h.run_steps(3);
    h.get_by_label("Reset").click();
    h.run_steps(4);
    for panel in [Panel::Assets, Panel::Layers, Panel::Properties, Panel::TextureSetList, Panel::TextureSetSettings] {
        assert!(h.state().ui.dock.is_open(panel));
    }
    for panel in [Panel::History, Panel::Display, Panel::Shader, Panel::Log, Panel::Export] {
        assert!(!h.state().ui.dock.is_open(panel));
    }
    assert!(h.state().ui.toolbar_tools && h.state().ui.toolbar_docks);
    h.get_all_by_label("Viewport").min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    h.run_steps(3);
    let quick = h.get_by_label_contains("Enable Quick Mask");
    assert!(quick.accesskit_node().is_disabled());
    assert_eq!(quick.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn tools_hides_painting_controls_but_keeps_disabled_utilities() {
    let mut h = app();
    let ctx = h.ctx.clone();
    for visible in [true, false, true] {
        if h.state().ui.toolbar_tools != visible {
            menus::invoke(h.state_mut(), &ctx, "ui.toolbar.tools");
        }
        h.run_steps(3);
        assert_eq!(h.query_by_label_contains("Eraser").is_some(), visible);
        for label in ["Snooze Autosave", "Resources Updater"] {
            assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
        }
        assert!(h.query_by_label_contains("Online assets").is_none());
    }
}

#[test]
fn loading_a_mode_layout_preserves_the_other_modes_and_preferences() {
    use texturecraft_ui_egui::{file_dialogs::Answer, state::Mode};
    let dir = super::temp_dir::TempDir::new("mode-layout-isolation");
    let file = dir.join("rendering.json");
    let mut h = app();
    h.state_mut().run("project.new", serde_json::json!({"mesh":"cube", "resolution":128})).unwrap();
    h.state_mut().ui.mode = Mode::Rendering;
    h.run_steps(3);
    h.state_mut().ui.dock.focus(Panel::Renderer);
    let target = file.clone();
    h.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![target.clone()]))));
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.layout.save.rendering");
    h.run_steps(4);
    h.state_mut().ui.mode = Mode::Painting;
    h.run_steps(3);
    h.state_mut().ui.dock.close(Panel::Assets);
    h.state_mut().ui.prefs.autosave_minutes = 17;
    h.state_mut().ui.mode = Mode::Baking;
    h.run_steps(3);
    h.state_mut().ui.dock.close(Panel::MeshMapBakers);
    menus::invoke(h.state_mut(), &ctx, "ui.layout.load");
    h.run_steps(4);
    assert_eq!(h.state().ui.mode, Mode::Rendering);
    assert!(h.state().ui.dock.is_open(Panel::Renderer));
    assert_eq!(h.state().ui.prefs.autosave_minutes, 17);
    h.state_mut().ui.mode = Mode::Painting;
    h.run_steps(3);
    assert!(!h.state().ui.dock.is_open(Panel::Assets));
    h.state_mut().ui.mode = Mode::Baking;
    h.run_steps(3);
    assert!(!h.state().ui.dock.is_open(Panel::MeshMapBakers));
}
