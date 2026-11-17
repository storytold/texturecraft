//! The Alpha tab's Parameters group: hardness and the parameters every alpha shares, with the Parameters reset

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::paint::{AlphaParams, BrushAlpha, BrushSettings};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::PropsTab;

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);
const STARTUP_FRAMES: usize = 4;
const SETTLE_FRAMES: usize = 3;

fn changed_params() -> AlphaParams {
    AlphaParams { size_x: 0.5, size_y: 0.25, roundness: 0.2, contrast: 3.0, invert: true }
}

/// The app showing the Alpha tab of Properties, with `brush`
fn app_with_brush(brush: BrushSettings) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().session.tool.brush = brush;
    harness.state_mut().ui.props_tab = PropsTab::Alpha;
    harness.run_steps(STARTUP_FRAMES);
    harness
}

fn restore_defaults<'tree>(harness: &'tree Harness<'_, TextureCraftApp>) -> egui_kittest::Node<'tree> {
    harness.get_by_role_and_label(egui::Role::Button, "Parameters: Reset to defaults")
}

fn invert_checkbox<'tree>(harness: &'tree Harness<'_, TextureCraftApp>) -> Option<egui_kittest::Node<'tree>> {
    harness.query_all_by_role_and_label(egui::Role::CheckBox, "Invert").next()
}

/// Scrolls the checkbox into view and clicks it
fn click_invert(harness: &mut Harness<'_, TextureCraftApp>) {
    if let Some(node) = invert_checkbox(harness) {
        node.scroll_to_me();
    }
    harness.run_steps(SETTLE_FRAMES);
    if let Some(node) = invert_checkbox(harness) {
        node.click();
    }
    harness.run_steps(SETTLE_FRAMES);
}

fn show_group(harness: &mut Harness<'_, TextureCraftApp>) {
    restore_defaults(harness).scroll_to_me();
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn the_group_shows_hardness_and_every_shared_parameter() {
    let harness = app_with_brush(BrushSettings::default());

    for label in ["Parameters", "Hardness", "Size X", "Size Y", "Roundness", "Contrast"] {
        assert!(harness.query_by_label(label).is_some(), "{label} is in the Alpha tab");
    }
    assert!(invert_checkbox(&harness).is_some());
}

#[test]
fn restore_defaults_puts_back_hardness_and_the_parameters_but_keeps_the_alpha() {
    let brush = BrushSettings { hardness: 0.9, alpha: BrushAlpha::Star, alpha_params: changed_params(), size: 0.1, flow: 0.4, ..BrushSettings::default() };
    let mut harness = app_with_brush(brush.clone());
    show_group(&mut harness);
    assert!(!restore_defaults(&harness).accesskit_node().is_disabled());

    restore_defaults(&harness).click();
    harness.run_steps(SETTLE_FRAMES);

    let defaults = BrushSettings::default();
    let expected = BrushSettings { hardness: defaults.hardness, alpha_params: defaults.alpha_params, ..brush };
    assert_eq!(harness.state().session.tool.brush, expected);
    assert!(restore_defaults(&harness).accesskit_node().is_disabled(), "nothing left to restore");
}

#[test]
fn the_invert_checkbox_sets_the_brush_parameter() {
    let mut harness = app_with_brush(BrushSettings::default());
    show_group(&mut harness);
    assert!(restore_defaults(&harness).accesskit_node().is_disabled(), "at the defaults there is nothing to restore");

    click_invert(&mut harness);

    assert!(harness.state().session.tool.brush.alpha_params.invert);
    assert!(!restore_defaults(&harness).accesskit_node().is_disabled());

    click_invert(&mut harness);
    assert_eq!(harness.state().session.tool.brush, BrushSettings::default());
}

#[test]
fn hardness_left_the_brush_tab_and_its_reset() {
    let mut harness = app_with_brush(BrushSettings::default());
    harness.state_mut().ui.props_tab = PropsTab::Brush;
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_label("Hardness").is_none(), "Hardness is in the Alpha tab now");
    assert!(harness.query_by_label("Flow").is_some());
}
