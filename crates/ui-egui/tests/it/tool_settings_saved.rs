//! The app saves the brush and other tool settings with the UI state

use egui_kittest::Harness;
use texturecraft_ui_egui::{TextureCraftApp, UiState};

mod memory_storage;

use memory_storage::MemoryStorage;

#[test]
fn saving_the_app_stores_the_current_brush_and_tool_settings() {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(2);
    let tool = &mut harness.state_mut().session.tool;
    tool.brush.size = 0.37;
    tool.brush.symmetry.z = true;
    tool.smudge_strength = 0.9;

    let mut storage = MemoryStorage::default();
    eframe::App::save(harness.state_mut(), &mut storage);

    let saved = serde_json::from_str::<UiState>(&storage.0["ui.json"]).expect("the UI state was saved");
    assert_eq!(saved.tool_settings.brush.size, 0.37);
    assert!(saved.tool_settings.brush.symmetry.z);
    assert_eq!(saved.tool_settings.smudge_strength, 0.9);
}
