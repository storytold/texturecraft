//! The 2D view hides the system pointer and draws the brush outline under it while painting.

use egui::CursorIcon;
use egui_kittest::Harness;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::{Mode, ViewLayout};

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.layout = ViewLayout::Only2D;
    h.run_steps(4);
    h
}

fn cursor(h: &Harness<'_, TextureCraftApp>) -> CursorIcon {
    h.output().platform_output.cursor_icon
}

#[test]
fn painting_in_the_2d_view_replaces_the_pointer_with_the_brush_outline() {
    let mut h = harness();
    let c = egui::pos2(800.0, 480.0);
    h.hover_at(c);
    h.run_steps(3);
    assert_eq!(cursor(&h), CursorIcon::None, "the brush outline replaces the pointer");
    h.hover_at(egui::pos2(2.0, 2.0));
    h.run_steps(3);
    assert_ne!(cursor(&h), CursorIcon::None, "outside the view the pointer is back");
}

#[test]
fn outside_painting_the_2d_view_keeps_the_normal_pointer() {
    let mut h = harness();
    h.state_mut().ui.mode = Mode::Baking;
    h.run_steps(3);
    let c = egui::pos2(800.0, 480.0);
    h.hover_at(c);
    h.run_steps(3);
    assert_ne!(cursor(&h), CursorIcon::None);
}
