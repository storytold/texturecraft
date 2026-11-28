//! The Assets panel's folder button opens a second Assets panel with its own search

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

/// The sample project at 1600 x 960
fn sample_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    harness
}

/// Type `text` into the search field of the Assets panel shown left of the 3D view
fn search_shown_panel(harness: &mut Harness<'_, TextureCraftApp>, text: &str) {
    let left_of_view = harness.state().viewport_rect.min.x;
    harness.get_all_by_role(Role::TextInput).find(|field| field.rect().max.x < left_of_view).expect("the shown Assets panel has a search field").click();
    harness.run_steps(2);
    harness.event(egui::Event::Text(text.into()));
    harness.run_steps(2);
}

#[test]
fn the_folder_button_opens_a_second_assets_panel_that_searches_on_its_own() {
    let mut harness = sample_app();
    assert!(!harness.state().ui.dock.is_open(Panel::AssetsSecond), "one Assets panel at first");

    harness.get_by_label("New Assets panel").click();
    harness.run_steps(3);

    assert!(harness.state().ui.dock.is_visible(Panel::AssetsSecond), "the folder button shows a second Assets panel");

    search_shown_panel(&mut harness, "brick");
    let ui = &harness.state().ui;
    assert_eq!((ui.shelf_second.filter.as_str(), ui.shelf_filter.as_str()), ("brick", ""), "the second panel's search leaves the first panel's alone");

    harness.state_mut().ui.dock.focus(Panel::Assets);
    harness.run_steps(3);
    search_shown_panel(&mut harness, "rust");
    let ui = &harness.state().ui;
    assert_eq!((ui.shelf_second.filter.as_str(), ui.shelf_filter.as_str()), ("brick", "rust"), "the first panel's search leaves the second panel's alone");
}
