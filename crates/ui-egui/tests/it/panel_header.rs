//! Every docked panel group's header ends in its close button, and the Properties header names what it shows

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

use crate::{BAKE_RESOLUTION, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION, click_at, panel_close_button};

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);

    harness
}

/// The rect of the tab whose name starts with `name`
fn tab_rect(harness: &Harness<'_, TextureCraftApp>, name: &str) -> egui::Rect {
    harness
        .get_all_by_role(Role::Tab)
        .find(|tab| tab.accesskit_node().label().is_some_and(|label| label.starts_with(name)))
        .map(|tab| tab.rect())
        .unwrap_or_else(|| panic!("a {name} tab"))
}

#[test]
fn every_docked_panel_header_ends_in_its_own_close_button() {
    let harness = app();
    let names = ["Assets", "Texture Sets", "Layers", "Properties - "];
    let tabs: Vec<egui::Rect> = names.iter().map(|name| tab_rect(&harness, name)).collect();
    for (name, tab) in names.iter().zip(&tabs) {
        let close = panel_close_button(&harness, *tab);

        // No other group's tab between a tab and its close button: the button is on its own bar
        let between = tabs.iter().any(|other| other.y_range().contains(close.center().y) && other.min.x > tab.max.x && other.max.x < close.min.x);
        assert!(!between, "{name} has no close button on its own header");
    }
}

#[test]
fn the_header_close_button_closes_its_panel() {
    let mut harness = app();
    assert!(harness.state().ui.dock.is_visible(Panel::Properties));

    let close = panel_close_button(&harness, tab_rect(&harness, "Properties - "));
    click_at(&mut harness, close.center());

    assert!(!harness.state().ui.dock.is_open(Panel::Properties));
    assert!(harness.state().ui.dock.is_visible(Panel::Layers), "only the panel under the bar closes");
}

/// With an effect selected the Properties header names its type, not the effect's own name (every filter reads "Filter")
#[test]
fn the_properties_header_names_the_selected_effects_type() {
    let mut harness = app();
    let session = &mut harness.state_mut().session;
    let layer = texturecraft_engine::execute(session, "layer.add.fill", &serde_json::Value::Null).unwrap()["layer"].as_u64().unwrap();
    let blur = texturecraft_engine::execute(session, "effect.add.filter", &serde_json::json!({"layer": layer, "target": "content", "filter": "blur"})).unwrap()
        ["effect"]
        .as_u64()
        .unwrap();
    texturecraft_engine::execute(session, "effect.set", &serde_json::json!({"effect": blur, "name": "Soften edges"})).unwrap();
    texturecraft_engine::execute(session, "layer.select", &serde_json::json!({"layer": layer, "target": blur})).unwrap();
    harness.run_steps(SETTLE_FRAMES);

    tab_rect(&harness, "Properties - Filter");

    let level = texturecraft_engine::execute(&mut harness.state_mut().session, "effect.add.levels", &serde_json::json!({"layer": layer, "target": "content"}))
        .unwrap()["effect"]
        .as_u64()
        .unwrap();
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.select", &serde_json::json!({"layer": layer, "target": level})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    tab_rect(&harness, "Properties - Levels");
}

/// Where the label on `name`'s row and to its right ends, for each label named `name`
fn row_ends(harness: &Harness<'_, TextureCraftApp>, name: &str) -> Vec<f32> {
    let names = harness.get_all_by_role_and_label(Role::Label, name).map(|node| node.rect());
    names
        .filter_map(|name| {
            let on_row = harness.get_all_by_role(Role::Label).map(|node| node.rect());
            on_row.filter(|rect| rect.left() > name.right() && (rect.center().y - name.center().y).abs() < 1.0).map(|rect| rect.right()).reduce(f32::max)
        })
        .collect()
}

/// Texture Set Settings keeps Base color's remove room, so its format ends where the other channels' formats do (it ended 24 px right)
#[test]
fn every_channels_format_ends_on_one_column() {
    let mut harness = app();
    let settings = tab_rect(&harness, "Texture Set Settings");
    click_at(&mut harness, settings.center());
    harness.run_steps(SETTLE_FRAMES);

    let base_color = row_ends(&harness, "Base color");
    let roughness = row_ends(&harness, "Roughness");

    assert!(base_color.len() == 1 && base_color == roughness, "Base color's format ends at {base_color:?}, Roughness's at {roughness:?}");
}
