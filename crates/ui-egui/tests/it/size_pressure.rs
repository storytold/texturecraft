//! The pen button on the paint tool's Size row turns pen pressure on Size on and off; Minimum Size only applies while it is on

use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// Tall enough that the Size band shows without scrolling
const WINDOW: egui::Vec2 = egui::vec2(1600.0, 960.0);

fn paint_harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder()
        .with_size(WINDOW)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

fn pen_toggled(harness: &Harness<'_, TextureCraftApp>) -> Option<Toggled> {
    harness.get_by_role_and_label(Role::CheckBox, "Size pen pressure").accesskit_node().toggled()
}

/// Whether every Minimum Size slider node is greyed
fn minimum_size_greyed(harness: &Harness<'_, TextureCraftApp>) -> bool {
    let sliders: Vec<_> = harness.get_all_by_label("Minimum Size (%)").filter(|node| node.accesskit_node().role() == Role::Slider).collect();
    assert!(!sliders.is_empty(), "Minimum Size has a slider");
    sliders.iter().all(|node| node.accesskit_node().is_disabled())
}

#[test]
fn the_pen_button_turns_size_pressure_off_and_greys_minimum_size() {
    let mut harness = paint_harness();
    assert_eq!(pen_toggled(&harness), Some(Toggled::True), "a new brush follows the pen");
    assert!(!minimum_size_greyed(&harness));

    harness.get_by_role_and_label(Role::CheckBox, "Size pen pressure").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(!harness.state().session.tool.brush.size_pressure, "the click reached the brush");
    assert_eq!(pen_toggled(&harness), Some(Toggled::False));
    assert!(minimum_size_greyed(&harness), "Minimum Size does nothing without pen pressure");
}
