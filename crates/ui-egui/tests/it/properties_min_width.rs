//! The Properties panel never gets narrower than its rows need: the column it docks in stops shrinking there

use craft_behavior::dock::Node;
use craft_behavior::dock::SplitSize;
use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::{Dock, PROPERTIES_MIN_WIDTH};

use crate::{BAKE_RESOLUTION, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION, panel_close_button};

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The header names below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// Gives the right column (Layers over Properties) `width` points, as a layout saved at that width would
fn set_column_width(harness: &mut Harness<'_, TextureCraftApp>, width: f32) {
    let workspace = harness.state().ui.dock.workspace().clone();
    let mut layout = workspace.layout().clone();
    let Some(Node::Split { second, .. }) = layout.root.as_mut() else { panic!("the assets column and the rest") };
    let Node::Split { size, .. } = second.as_mut() else { panic!("the 3D view and the right column") };

    *size = SplitSize::FixedSecond(width);
    let workspace = texturecraft_ui_egui::docking::Workspace::from_parts(layout, workspace.hidden().to_vec(), workspace.floated().to_vec());
    harness.state_mut().ui.dock = Dock::from_value(serde_json::to_value(workspace).unwrap());
    harness.run_steps(SETTLE_FRAMES);
}

/// The Properties header's title, which starts at the column's left edge
fn properties_title(harness: &Harness<'_, TextureCraftApp>) -> egui::Rect {
    harness
        .get_all_by_role(Role::Tab)
        .find(|tab| tab.accesskit_node().label().is_some_and(|label| label.starts_with("Properties - ")))
        .map(|tab| tab.rect())
        .expect("the Properties header")
}

/// Width of the Properties header bar, from its title's left edge to its close button's right edge
fn properties_header_width(harness: &Harness<'_, TextureCraftApp>) -> f32 {
    let title = properties_title(harness);
    panel_close_button(harness, title).max.x - title.min.x
}

#[test]
fn a_layout_saved_narrower_draws_properties_at_its_minimum_width() {
    let mut harness = app();
    set_column_width(&mut harness, PROPERTIES_MIN_WIDTH);
    let at_minimum = properties_header_width(&harness);

    set_column_width(&mut harness, PROPERTIES_MIN_WIDTH * 0.5);
    let width = properties_header_width(&harness);

    assert!((width - at_minimum).abs() < 0.5, "a narrow saved column opens at the minimum: header {width}, {at_minimum} at the minimum");
}

#[test]
fn visiting_and_deleting_layers_does_not_accumulate_scroll_areas() {
    let mut harness = app();
    let count = |harness: &Harness<'_, TextureCraftApp>| harness.ctx.data(|data| data.count::<egui::scroll_area::State>());
    let initial = count(&harness);

    let mut layers = Vec::new();
    for index in 0..12 {
        let session = &mut harness.state_mut().session;
        let layer = texturecraft_engine::execute(session, "layer.add.group", &serde_json::json!({"name": format!("Folder {index}")})).unwrap()["layer"]
            .as_u64()
            .unwrap();
        texturecraft_engine::execute(session, "layer.select", &serde_json::json!({"layer": layer})).unwrap();
        harness.run_steps(1);
        layers.push(layer);
    }
    let after_visiting = count(&harness);

    let session = &mut harness.state_mut().session;
    texturecraft_engine::execute(session, "layer.remove", &serde_json::json!({"layers": layers})).unwrap();
    harness.run_steps(SETTLE_FRAMES);

    let after_removal = count(&harness);
    eprintln!("scroll states after visiting {after_visiting}, after removal {after_removal}, initially {initial}");
    assert!(
        after_visiting <= initial && after_removal <= initial,
        "scroll areas after visiting {after_visiting}, after removal {after_removal}, initially {initial}"
    );
}
