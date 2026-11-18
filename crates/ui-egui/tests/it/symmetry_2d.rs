//! A click in the 2D view paints the symmetry copies too

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

/// A click on the sample's texture in the 2D view
const CLICK: Pos2 = pos2(700.0, 420.0);

/// The sample project with only the 2D view, at 1600 x 960
fn uv_view_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(4);
    harness
}

/// A primary click at `pos`
fn click(harness: &mut Harness<'_, TextureCraftApp>, pos: Pos2) {
    harness.hover_at(pos);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
}

/// The base color alpha the selected layer holds, summed over the texture
fn painted(harness: &Harness<'_, TextureCraftApp>) -> f32 {
    let session = &harness.state().session;
    let (width, height) = session.doc.project.texture_sets[session.ts()].size();
    let layer = session.selected_layer().expect("a selected layer");
    let Some(image) = session.doc.pixels.layer(layer, Channel::BaseColor) else { return 0.0 };
    (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| image.get(x, y)[3] as f32 / 65535.0).sum()
}

/// What one click paints with the symmetry settings `brush`
fn click_paints(brush: serde_json::Value) -> f32 {
    let mut harness = uv_view_app();
    harness.state_mut().run("tool.brush", brush);
    harness.run_steps(2);
    let before = painted(&harness);
    click(&mut harness, CLICK);
    painted(&harness) - before
}

#[test]
fn a_click_in_the_2d_view_paints_the_mirrored_dab_too() {
    let single = click_paints(json!({"symmetryOn": false}));
    let mirrored = click_paints(json!({"symmetryOn": true, "symmetryX": true}));

    assert!(single > 1.0, "the dab itself paints: {single}");
    assert!(mirrored > single * 1.5, "the mirrored copy adds a second dab: {mirrored} against {single}");
}

#[test]
fn radial_copies_add_dabs_in_the_2d_view() {
    let single = click_paints(json!({"symmetryOn": false}));
    let radial = click_paints(json!({"symmetryOn": true, "symmetryRadial": 4, "symmetryAxis": "y"}));

    assert!(radial > single * 2.0, "three more dabs: {radial} against {single}");
}
