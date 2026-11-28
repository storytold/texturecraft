//! The Alpha section's slot: its picker sets the brush alpha, × goes back to the Round shape, and Roundness shows only for the Round shape

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::paint::{BrushAlpha, BrushSettings, alphas};
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, STARTUP_FRAMES, TEXTURE_RESOLUTION, click_widget};

/// Window the Alpha section tests run in
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// Frames after a click in the slot, more than the crate's SETTLE_FRAMES to leave the alpha picker's popup time to open or close
const SLOT_CLICK_FRAMES: usize = 6;

/// Frames for the jump tab's animated scroll to settle
const SCROLL_FRAMES: usize = 60;

/// The app showing the Alpha section of Properties, with `brush`
fn app_with_brush(brush: BrushSettings) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.state_mut().session.tool.brush = brush;
    harness.run_steps(STARTUP_FRAMES);
    harness.get_by_role_and_label(Role::Tab, "Alpha").click();
    harness.run_steps(SCROLL_FRAMES);
    harness
}

fn click(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    click_widget(harness, role, label);
    harness.run_steps(SLOT_CLICK_FRAMES);
}

/// The × of the slot showing `slot`: the Clear button on its row
fn click_clear(harness: &mut Harness<'_, TextureCraftApp>, slot: &str) {
    let row = harness.get_by_role_and_label(Role::ComboBox, slot).rect();
    let clear = harness
        .query_all_by_role_and_label(Role::Button, "Clear")
        .find(|node| node.rect().y_range().intersects(row.y_range()))
        .unwrap_or_else(|| panic!("no Clear on the {slot} slot"))
        .rect();
    super::click_at(harness, clear.center());
    harness.run_steps(SLOT_CLICK_FRAMES);
}

/// Clicks the built-in shape `label`: the Shapes come first, above a library alpha of the same name
fn click_shape(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    let tile = harness
        .query_all_by_role_and_label(Role::ListBoxOption, label)
        .map(|node| node.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("no {label} tile"));
    super::click_at(harness, tile.center());
    harness.run_steps(SLOT_CLICK_FRAMES);
}

#[test]
fn the_slot_picks_a_shape_and_its_clear_goes_back_to_round() {
    let mut harness = app_with_brush(BrushSettings::default());
    let row = harness.get_by_role_and_label(Role::ComboBox, "Round").rect();
    assert!(
        !harness.query_all_by_role_and_label(Role::Button, "Clear").any(|node| node.rect().y_range().intersects(row.y_range())),
        "the Round shape has nothing to clear back to"
    );

    click(&mut harness, Role::ComboBox, "Round");
    click_shape(&mut harness, "Star");
    assert_eq!(harness.state().session.tool.brush.alpha, BrushAlpha::Star);
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Star").is_some(), "the slot shows the picked shape");

    click_clear(&mut harness, "Star");
    assert_eq!(harness.state().session.tool.brush.alpha, BrushAlpha::Round);
}

#[test]
fn the_picker_offers_the_library_alphas() {
    // Any library alpha
    let alpha = &alphas::library()[0];
    let mut harness = app_with_brush(BrushSettings::default());

    click(&mut harness, Role::ComboBox, "Round");
    harness.event(egui::Event::Text(alpha.label.clone()));
    harness.run_steps(SLOT_CLICK_FRAMES);
    click(&mut harness, Role::ListBoxOption, &alpha.label);

    let brush = &harness.state().session.tool.brush;
    assert_eq!(brush.alpha_library.as_deref(), Some(alpha.id.as_str()));
    assert!(harness.query_by_role_and_label(Role::ComboBox, &alpha.label).is_some(), "the slot shows the library alpha");
}

#[test]
fn roundness_shows_only_for_the_round_shape() {
    let roundness = |harness: &Harness<'_, TextureCraftApp>| harness.query_all_by_role_and_label(Role::Slider, "Roundness").count();

    let round = app_with_brush(BrushSettings::default());
    assert_eq!(roundness(&round), 1, "the Round shape has a roundness");

    let star = app_with_brush(BrushSettings { alpha: BrushAlpha::Star, ..BrushSettings::default() });
    assert_eq!(roundness(&star), 0, "other alphas have none");
    assert_eq!(star.query_all_by_role_and_label(Role::Slider, "Hardness").count(), 1, "the rest of the Parameters stay");
}
