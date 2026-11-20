//! Window ▸ Reset panel layout puts back only the panels; Reset UI asks before it resets the view too.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::{Mode, ViewLayout};

/// An app with the Assets panel closed, a changed view split, zoom, wireframe and mode.
fn changed_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    let ui = &mut h.state_mut().ui;
    ui.dock.close(Panel::Assets);
    ui.layout = ViewLayout::Only3D;
    ui.uv_zoom = 2.5;
    ui.mesh_wireframe = true;
    ui.mode = Mode::Rendering;
    h.run_steps(2);
    h
}

fn invoke(h: &mut Harness<'_, TextureCraftApp>, id: &str) {
    let ctx = h.ctx.clone();
    texturecraft_ui_egui::menus::invoke(h.state_mut(), &ctx, id);
    h.run_steps(2);
}

fn view_kept(h: &Harness<'_, TextureCraftApp>) -> bool {
    let ui = &h.state().ui;
    ui.layout == ViewLayout::Only3D && ui.uv_zoom == 2.5 && ui.mesh_wireframe && ui.mode == Mode::Rendering
}

#[test]
fn reset_panel_layout_restores_the_panels_and_keeps_the_view() {
    let mut h = changed_app();
    assert!(!h.state().ui.dock.is_open(Panel::Assets));
    invoke(&mut h, "ui.resetPanelLayout");
    assert!(h.state().ui.dock.is_open(Panel::Assets), "the closed panel is back");
    assert!(view_kept(&h), "view split, zoom, wireframe and mode stay");
    assert!(!h.state().ui.windows.confirm_reset_ui, "no question for the panel reset");
}

#[test]
fn reset_ui_asks_first_and_cancel_changes_nothing() {
    let mut h = changed_app();
    invoke(&mut h, "ui.resetUi");
    assert!(!h.state().ui.dock.is_open(Panel::Assets), "nothing is reset before the answer");
    assert!(view_kept(&h));
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(!h.state().ui.windows.confirm_reset_ui);
    assert!(!h.state().ui.dock.is_open(Panel::Assets));
    assert!(view_kept(&h));
}

#[test]
fn reset_ui_ok_resets_everything_as_before() {
    let mut h = changed_app();
    invoke(&mut h, "ui.resetUi");
    h.get_by_label("Reset").click();
    h.run_steps(2);
    let ui = &h.state().ui;
    assert!(!ui.windows.confirm_reset_ui);
    assert!(ui.dock.is_open(Panel::Assets));
    assert_eq!((ui.layout, ui.uv_zoom, ui.mesh_wireframe, ui.mode), (ViewLayout::Split, 1.0, false, Mode::Painting));
}

#[test]
fn reset_ui_escape_keeps_everything_and_enter_resets() {
    let mut h = changed_app();
    invoke(&mut h, "ui.resetUi");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().ui.windows.confirm_reset_ui);
    assert!(view_kept(&h) && !h.state().ui.dock.is_open(Panel::Assets));

    invoke(&mut h, "ui.resetUi");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(!h.state().ui.windows.confirm_reset_ui);
    assert!(h.state().ui.dock.is_open(Panel::Assets));
    assert_eq!(h.state().ui.layout, ViewLayout::Split);
}

#[test]
fn the_menu_invoke_command_asks_first_like_a_click() {
    let mut h = changed_app();
    let ctx = h.ctx.clone();
    let reply = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.menu.invoke", &serde_json::json!({"id": "ui.resetUi"}));
    h.run_steps(2);
    assert!(reply.get("error").is_none(), "{reply}");
    assert!(h.state().ui.windows.confirm_reset_ui, "the question is open");
    assert!(!h.state().ui.dock.is_open(Panel::Assets), "nothing is reset before the answer");
    assert!(view_kept(&h));
}

#[test]
fn the_ui_reset_control_command_resets_without_asking() {
    let mut h = changed_app();
    let ctx = h.ctx.clone();
    let reply = texturecraft_ui_egui::control::handle(h.state_mut(), &ctx, "ui.reset", &serde_json::Value::Null);
    h.run_steps(2);
    assert!(reply.get("error").is_none(), "{reply}");
    let ui = &h.state().ui;
    assert!(!ui.windows.confirm_reset_ui, "no question for a script");
    assert!(ui.dock.is_open(Panel::Assets));
    assert_eq!((ui.layout, ui.uv_zoom, ui.mesh_wireframe, ui.mode), (ViewLayout::Split, 1.0, false, Mode::Painting));
}

#[test]
fn escape_cancels_the_question_even_when_a_shortcut_binds_it() {
    let mut h = changed_app();
    h.state_mut().ui.keymap.insert("ui.hideUi".into(), "Escape".into());
    h.run_steps(2);
    invoke(&mut h, "ui.resetUi");
    assert!(h.state().ui.windows.confirm_reset_ui);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().ui.windows.confirm_reset_ui, "Escape closed the question");
    assert!(!h.state().ui.hide_ui, "and did not run the command bound to it");
    assert!(view_kept(&h) && !h.state().ui.dock.is_open(Panel::Assets));
}
